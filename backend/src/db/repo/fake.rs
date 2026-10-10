//! In-memory fakes of the repository traits (see `docs/tests.md` §1.3).
//!
//! The fakes emulate the PostgreSQL semantics the API and smoke tests rely on
//! (unique open alert per point, cascade delete, snippet truncation, ILIKE
//! filtering, pagination, uptime windows) so unit/integration tests can run
//! without a live database. The live-service smoke tests remain the source of
//! truth that the sqlx implementations match these semantics.
//!
//! Gated behind `test-utils` (or `test`) so production builds carry no test code.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::db::models::{
    ActiveAlert, AlertPoint, AlertType, HttpMethod, NewActiveAlert, NewAlertPoint, NewProbeResult,
    ProbeResult, ServiceType, UpdateAlertPoint,
};
use crate::db::repo::alert_points::ListFilter;
use crate::db::repo::traits::{
    ActiveAlertStore, AlertPointRepository, ProbeResultStore, RepoError,
};

/// Maximum length of a stored `raw_response_snippet` (mirrors the pg impl).
pub const SNIPPET_MAX_CHARS: usize = 2000;

// ---------------------------------------------------------------------------
// shared failure-injection helper
// ---------------------------------------------------------------------------

/// Per-fake hook: the next operation returns the stored error once.
#[derive(Debug, Default)]
pub struct FailNext(Mutex<Option<RepoError>>);

impl FailNext {
    fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// Make the *next* operation return `error`, then resume normal behavior.
    pub fn arm(&self, error: RepoError) {
        *self.0.lock().expect("fail_next poisoned") = Some(error);
    }

    /// Take the armed error, if any.
    fn take(&self) -> Option<RepoError> {
        self.0.lock().expect("fail_next poisoned").take()
    }
}

// ---------------------------------------------------------------------------
// ILIKE emulation
// ---------------------------------------------------------------------------

/// Case-insensitive SQL `ILIKE`-style match supporting `%` (any sequence) and
/// `_` (any single char), including the `\%` / `\_` / `\\` escapes produced by
/// the API's `like_pattern` helper.
pub fn ilike_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().flat_map(|c| c.to_lowercase()).collect();

    // recursive matcher over (pattern pos, text pos)
    fn m(p: &[char], t: &[char]) -> bool {
        if p.is_empty() {
            return t.is_empty();
        }
        match p[0] {
            '%' => {
                // try matching % against 0..=n chars
                (0..=t.len()).any(|skip| m(&p[1..], &t[skip..]))
            }
            '\\' if p.len() >= 2 => {
                // escaped literal (\%, \_, \\)
                if t.is_empty() || t[0] != p[1] {
                    false
                } else {
                    m(&p[2..], &t[1..])
                }
            }
            '_' => !t.is_empty() && m(&p[1..], &t[1..]),
            c => {
                let cl: Vec<char> = c.to_lowercase().collect();
                !t.is_empty() && t[0] == cl[0] && m(&p[1..], &t[1..])
            }
        }
    }

    m(&p, &t)
}

// ---------------------------------------------------------------------------
// state
// ---------------------------------------------------------------------------

/// Shared mutable state of the fakes; clone `InMemory*Store` structs freely
/// (they share the same backing state via `Arc` internally).
#[derive(Debug, Default)]
struct RepoState {
    alert_points: HashMap<Uuid, AlertPoint>,
    probe_results: Vec<ProbeResult>,
    active_alerts: Vec<ActiveAlert>,
    next_result_id: i64,
    next_alert_id: i64,
}

impl RepoState {
    fn order_key(p: &AlertPoint) -> (DateTime<Utc>, Uuid) {
        (p.created_at, p.id)
    }

    /// Mirrors `UPDATE alert_points SET … = COALESCE($, …)`.
    fn apply_update(point: &mut AlertPoint, input: &UpdateAlertPoint) {
        if let Some(v) = &input.name {
            point.name = v.clone();
        }
        if let Some(v) = &input.url {
            point.url = v.clone();
        }
        if let Some(v) = input.service_type {
            point.service_type = v;
        }
        if let Some(v) = input.check_interval_seconds {
            point.check_interval_seconds = v;
        }
        if let Some(v) = input.expected_response_time_ms {
            point.expected_response_time_ms = v;
        }
        if let Some(v) = input.http_method {
            point.http_method = v;
        }
        if let Some(v) = &input.custom_headers {
            point.custom_headers = v.clone();
        }
        if let Some(v) = &input.auth_config {
            point.auth_config = v.clone();
        }
        if let Some(v) = input.enabled {
            point.enabled = v;
        }
        point.updated_at = Utc::now();
    }

    fn matches_filter(p: &AlertPoint, filter: &ListFilter) -> bool {
        if let Some(enabled) = filter.enabled {
            if p.enabled != enabled {
                return false;
            }
        }
        if let Some(st) = filter.service_type {
            if p.service_type != st {
                return false;
            }
        }
        if let Some(pattern) = &filter.search {
            if !ilike_match(pattern, &p.name) && !ilike_match(pattern, &p.url) {
                return false;
            }
        }
        true
    }

    /// Cascade: drop probe results + alerts of deleted points.
    fn cascade_delete(&mut self, id: Uuid) {
        self.probe_results.retain(|r| r.alert_point_id != id);
        self.active_alerts.retain(|a| a.alert_point_id != id);
    }
}

/// `Arc`-shared state wrapper used by all three fake stores.
///
/// All three fakes share one `SharedState` when created together via
/// [`FakeStores::new`] so cross-store semantics (e.g. cascade delete) work.
#[derive(Debug, Default)]
struct SharedState {
    inner: Mutex<RepoState>,
    fail_next: FailNext,
}

impl SharedState {
    fn lock(&self) -> std::sync::MutexGuard<'_, RepoState> {
        self.inner.lock().expect("repo state poisoned")
    }

    /// One common entry point for failure injection across all operations.
    fn check_failure(&self) -> Result<(), RepoError> {
        match self.fail_next.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Bundle of the three repo fakes sharing one backing state, mirroring a
/// single PostgreSQL database (cascade deletes span the stores).
#[derive(Debug, Clone, Default)]
pub struct FakeStores {
    pub alert_points: FakeAlertPointRepository,
    pub probe_results: FakeProbeResultStore,
    pub active_alerts: FakeActiveAlertStore,
}

impl FakeStores {
    /// Create three linked fakes over one shared in-memory state.
    pub fn new() -> Self {
        let shared = Arc::new(SharedState::default());
        Self {
            alert_points: FakeAlertPointRepository(Arc::clone(&shared)),
            probe_results: FakeProbeResultStore(Arc::clone(&shared)),
            active_alerts: FakeActiveAlertStore(shared),
        }
    }
}

// ---------------------------------------------------------------------------
// fake AlertPointRepository
// ---------------------------------------------------------------------------

/// In-memory [`AlertPointRepository`]. Cloneable; clones share state.
#[derive(Debug, Clone, Default)]
pub struct FakeAlertPointRepository(Arc<SharedState>);

use std::sync::Arc;

impl FakeAlertPointRepository {
    pub fn new() -> Self {
        Self(Arc::new(SharedState {
            inner: Mutex::new(RepoState::default()),
            fail_next: FailNext::new(),
        }))
    }

    /// Arm the failure-injection hook: the next operation fails with `error`.
    pub fn fail_next(&self, error: RepoError) {
        self.0.fail_next.arm(error);
    }

    /// Direct access to the stored points (test assertions).
    pub fn len(&self) -> usize {
        self.0.lock().alert_points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl AlertPointRepository for FakeAlertPointRepository {
    async fn create(&self, new: NewAlertPoint) -> Result<AlertPoint, RepoError> {
        self.0.check_failure()?;
        let now = Utc::now();
        let custom_headers = if new.custom_headers.is_null() {
            serde_json::json!({})
        } else {
            new.custom_headers
        };
        let point = AlertPoint {
            id: Uuid::now_v7(),
            name: new.name,
            url: new.url,
            service_type: new.service_type,
            check_interval_seconds: new.check_interval_seconds,
            expected_response_time_ms: new.expected_response_time_ms,
            http_method: new.http_method,
            custom_headers,
            auth_config: new.auth_config,
            enabled: new.enabled,
            created_at: now,
            updated_at: now,
        };
        let mut state = self.0.lock();
        state.alert_points.insert(point.id, point.clone());
        Ok(point)
    }

    async fn get(&self, id: Uuid) -> Result<AlertPoint, RepoError> {
        self.0.check_failure()?;
        self.0
            .lock()
            .alert_points
            .get(&id)
            .cloned()
            .ok_or(RepoError::NotFound)
    }

    async fn list(
        &self,
        enabled_only: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AlertPoint>, RepoError> {
        self.0.check_failure()?;
        let state = self.0.lock();
        let mut points: Vec<AlertPoint> = state
            .alert_points
            .values()
            .filter(|p| !enabled_only || p.enabled)
            .cloned()
            .collect();
        points.sort_by_key(RepoState::order_key);
        Ok(points
            .into_iter()
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .collect())
    }

    async fn list_filtered(
        &self,
        filter: &ListFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AlertPoint>, RepoError> {
        self.0.check_failure()?;
        let state = self.0.lock();
        let mut points: Vec<AlertPoint> = state
            .alert_points
            .values()
            .filter(|p| RepoState::matches_filter(p, filter))
            .cloned()
            .collect();
        points.sort_by_key(RepoState::order_key);
        Ok(points
            .into_iter()
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .collect())
    }

    async fn count_filtered(&self, filter: &ListFilter) -> Result<i64, RepoError> {
        self.0.check_failure()?;
        let state = self.0.lock();
        Ok(state
            .alert_points
            .values()
            .filter(|p| RepoState::matches_filter(p, filter))
            .count() as i64)
    }

    async fn update(&self, id: Uuid, input: UpdateAlertPoint) -> Result<AlertPoint, RepoError> {
        self.0.check_failure()?;
        let mut state = self.0.lock();
        let point = state.alert_points.get_mut(&id).ok_or(RepoError::NotFound)?;
        RepoState::apply_update(point, &input);
        Ok(point.clone())
    }

    async fn delete(&self, id: Uuid) -> Result<bool, RepoError> {
        self.0.check_failure()?;
        let mut state = self.0.lock();
        let deleted = state.alert_points.remove(&id).is_some();
        if deleted {
            // ON DELETE CASCADE
            state.cascade_delete(id);
        }
        Ok(deleted)
    }
}

// ---------------------------------------------------------------------------
// fake ProbeResultStore
// ---------------------------------------------------------------------------

/// In-memory [`ProbeResultStore`]. Cloneable; clones share state.
#[derive(Debug, Clone, Default)]
pub struct FakeProbeResultStore(Arc<SharedState>);

impl FakeProbeResultStore {
    pub fn new() -> Self {
        Self(Arc::new(SharedState {
            inner: Mutex::new(RepoState::default()),
            fail_next: FailNext::new(),
        }))
    }

    /// Arm the failure-injection hook: the next operation fails with `error`.
    pub fn fail_next(&self, error: RepoError) {
        self.0.fail_next.arm(error);
    }

    /// Number of stored probe results (test assertions).
    pub fn len(&self) -> usize {
        self.0.lock().probe_results.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl ProbeResultStore for FakeProbeResultStore {
    async fn insert(&self, result: NewProbeResult) -> Result<ProbeResult, RepoError> {
        self.0.check_failure()?;
        let mut state = self.0.lock();
        // FK-style check: the alert point must exist
        if !state.alert_points.contains_key(&result.alert_point_id) {
            return Err(RepoError::Other(format!(
                "alert_point {} does not exist",
                result.alert_point_id
            )));
        }
        let id = state.next_result_id;
        state.next_result_id += 1;
        let probe = ProbeResult {
            id,
            alert_point_id: result.alert_point_id,
            timestamp: Utc::now(),
            response_time_ms: result.response_time_ms,
            status_code: result.status_code,
            is_up: result.is_up,
            error_message: result.error_message,
            raw_response_snippet: result
                .raw_response_snippet
                .map(|s| s.chars().take(SNIPPET_MAX_CHARS).collect::<String>()),
        };
        state.probe_results.push(probe.clone());
        Ok(probe)
    }

    async fn list_recent(
        &self,
        alert_point_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ProbeResult>, RepoError> {
        self.0.check_failure()?;
        let state = self.0.lock();
        let mut results: Vec<ProbeResult> = state
            .probe_results
            .iter()
            .filter(|r| r.alert_point_id == alert_point_id)
            .cloned()
            .collect();
        // newest first (timestamp DESC, id DESC — id is monotonic per insert)
        results.sort_by_key(|r| (std::cmp::Reverse(r.timestamp), std::cmp::Reverse(r.id)));
        Ok(results.into_iter().take(limit.max(0) as usize).collect())
    }

    async fn uptime_percentage(
        &self,
        alert_point_id: Uuid,
        hours: i32,
    ) -> Result<Option<f64>, RepoError> {
        self.0.check_failure()?;
        let state = self.0.lock();
        let cutoff = Utc::now() - chrono::Duration::hours(hours as i64);
        let relevant: Vec<&ProbeResult> = state
            .probe_results
            .iter()
            .filter(|r| r.alert_point_id == alert_point_id && r.timestamp > cutoff)
            .collect();
        if relevant.is_empty() {
            return Ok(None);
        }
        let up = relevant.iter().filter(|r| r.is_up).count();
        Ok(Some(100.0 * up as f64 / relevant.len() as f64))
    }
}

// ---------------------------------------------------------------------------
// fake ActiveAlertStore
// ---------------------------------------------------------------------------

/// In-memory [`ActiveAlertStore`]. Cloneable; clones share state.
#[derive(Debug, Clone, Default)]
pub struct FakeActiveAlertStore(Arc<SharedState>);

impl FakeActiveAlertStore {
    pub fn new() -> Self {
        Self(Arc::new(SharedState {
            inner: Mutex::new(RepoState::default()),
            fail_next: FailNext::new(),
        }))
    }

    /// Arm the failure-injection hook: the next operation fails with `error`.
    pub fn fail_next(&self, error: RepoError) {
        self.0.fail_next.arm(error);
    }

    /// Number of stored alerts (open + resolved, test assertions).
    pub fn len(&self) -> usize {
        self.0.lock().active_alerts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl ActiveAlertStore for FakeActiveAlertStore {
    async fn insert(&self, alert: NewActiveAlert) -> Result<ActiveAlert, RepoError> {
        self.0.check_failure()?;
        let mut state = self.0.lock();
        // unique partial index: at most one open alert per alert point
        if state
            .active_alerts
            .iter()
            .any(|a| a.alert_point_id == alert.alert_point_id && a.resolved_at.is_none())
        {
            return Err(RepoError::Conflict);
        }
        let id = state.next_alert_id;
        state.next_alert_id += 1;
        let active = ActiveAlert {
            id,
            alert_point_id: alert.alert_point_id,
            alert_type: alert.alert_type,
            reason: alert.reason,
            triggered_at: Utc::now(),
            resolved_at: None,
        };
        state.active_alerts.push(active.clone());
        Ok(active)
    }

    async fn resolve(&self, alert_point_id: Uuid) -> Result<u64, RepoError> {
        self.0.check_failure()?;
        let mut state = self.0.lock();
        let mut resolved = 0;
        for alert in state.active_alerts.iter_mut() {
            if alert.alert_point_id == alert_point_id && alert.resolved_at.is_none() {
                alert.resolved_at = Some(Utc::now());
                resolved += 1;
            }
        }
        Ok(resolved)
    }

    async fn list_open(&self) -> Result<Vec<ActiveAlert>, RepoError> {
        self.0.check_failure()?;
        let state = self.0.lock();
        let mut alerts: Vec<ActiveAlert> = state
            .active_alerts
            .iter()
            .filter(|a| a.resolved_at.is_none())
            .cloned()
            .collect();
        // newest first (triggered_at DESC, id DESC)
        alerts.sort_by_key(|a| (std::cmp::Reverse(a.triggered_at), std::cmp::Reverse(a.id)));
        Ok(alerts)
    }

    async fn latest_open(&self, alert_point_id: Uuid) -> Result<Option<ActiveAlert>, RepoError> {
        self.0.check_failure()?;
        let state = self.0.lock();
        Ok(state
            .active_alerts
            .iter()
            .filter(|a| a.alert_point_id == alert_point_id && a.resolved_at.is_none())
            .max_by_key(|a| (a.triggered_at, a.id))
            .cloned())
    }
}

// ---------------------------------------------------------------------------
// helpers for tests / fixtures
// ---------------------------------------------------------------------------

/// A valid `NewAlertPoint` fixture; override fields as needed.
pub fn sample_new_point(name: &str, url: &str) -> NewAlertPoint {
    NewAlertPoint {
        name: name.to_string(),
        url: url.to_string(),
        service_type: ServiceType::Wms,
        check_interval_seconds: 30,
        expected_response_time_ms: 500,
        http_method: HttpMethod::Get,
        custom_headers: serde_json::json!({}),
        auth_config: None,
        enabled: true,
    }
}

/// A valid `NewProbeResult` fixture; override fields as needed.
pub fn sample_new_probe_result(alert_point_id: Uuid, is_up: bool) -> NewProbeResult {
    NewProbeResult {
        alert_point_id,
        response_time_ms: Some(120),
        status_code: Some(200),
        is_up,
        error_message: None,
        raw_response_snippet: None,
    }
}

/// A valid `NewActiveAlert` fixture; override fields as needed.
pub fn sample_new_alert(alert_point_id: Uuid, alert_type: AlertType) -> NewActiveAlert {
    NewActiveAlert {
        alert_point_id,
        alert_type,
        reason: "probe failed".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    async fn two_points() -> (FakeAlertPointRepository, AlertPoint, AlertPoint) {
        let repo = FakeAlertPointRepository::new();
        let a = repo
            .create(sample_new_point("alpha", "https://a.example.com/wms"))
            .await
            .unwrap();
        let b = repo
            .create(sample_new_point("beta", "https://b.example.com/wfs"))
            .await
            .unwrap();
        (repo, a, b)
    }

    #[tokio::test]
    async fn create_get_update_delete_lifecycle() {
        let (repo, a, _b) = two_points().await;

        let got = repo.get(a.id).await.unwrap();
        assert_eq!(got.name, "alpha");
        assert!(got.enabled);
        assert_eq!(got.custom_headers, serde_json::json!({}));

        let updated = repo
            .update(
                a.id,
                UpdateAlertPoint {
                    name: Some("renamed".to_string()),
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "renamed");
        assert!(!updated.enabled);
        // untouched fields stay
        assert_eq!(updated.url, a.url);
        assert!(updated.updated_at >= a.updated_at);

        assert!(repo.delete(a.id).await.unwrap());
        assert!(matches!(repo.get(a.id).await, Err(RepoError::NotFound)));
        assert!(!repo.delete(a.id).await.unwrap());
        assert_eq!(repo.len(), 1);
    }

    #[tokio::test]
    async fn get_unknown_is_not_found() {
        let repo = FakeAlertPointRepository::new();
        assert!(matches!(
            repo.get(Uuid::now_v7()).await,
            Err(RepoError::NotFound)
        ));
    }

    #[tokio::test]
    async fn list_orders_by_creation_and_supports_enabled_only() {
        let (repo, a, b) = two_points().await;
        // disable b: enabled_only list must hide it
        repo.update(
            b.id,
            UpdateAlertPoint {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let all = repo.list(false, 10, 0).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, a.id);
        assert_eq!(all[1].id, b.id);

        let enabled = repo.list(true, 10, 0).await.unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].id, a.id);

        // limit/offset
        let page = repo.list(false, 1, 1).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, b.id);
    }

    #[tokio::test]
    async fn filters_and_pagination() {
        let (repo, a, b) = two_points().await;

        // search by name substring
        let filter = ListFilter {
            search: Some("%alph%".to_string()),
            ..Default::default()
        };
        let found = repo.list_filtered(&filter, 10, 0).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, a.id);

        // search matches url too, case-insensitively
        let filter = ListFilter {
            search: Some("%B.EXAMPLE%".to_string()),
            ..Default::default()
        };
        let found = repo.list_filtered(&filter, 10, 0).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, b.id);

        // no match
        let filter = ListFilter {
            search: Some("%nope%".to_string()),
            ..Default::default()
        };
        assert_eq!(repo.count_filtered(&filter).await.unwrap(), 0);

        // combined: enabled filter + search
        let filter = ListFilter {
            enabled: Some(true),
            search: Some("%alph%".to_string()),
            ..Default::default()
        };
        let found = repo.list_filtered(&filter, 10, 0).await.unwrap();
        assert_eq!(found.len(), 1);

        // offset pagination
        let filter = ListFilter::default();
        let page2 = repo.list_filtered(&filter, 1, 1).await.unwrap();
        assert_eq!(page2.len(), 1);
        assert_eq!(page2[0].id, b.id);
    }

    #[tokio::test]
    async fn probe_results_truncate_list_uptime() {
        let stores = FakeStores::new();
        let repo = &stores.alert_points;
        let probes = &stores.probe_results;

        let a = repo
            .create(sample_new_point("probe", "https://p.example.com"))
            .await
            .unwrap();

        let probe = probes
            .insert(sample_new_probe_result(a.id, true))
            .await
            .unwrap();
        assert!(probe.is_up);

        // snippet truncation to 2000 chars
        let probe = probes
            .insert(NewProbeResult {
                raw_response_snippet: Some("x".repeat(5000)),
                ..sample_new_probe_result(a.id, false)
            })
            .await
            .unwrap();
        assert_eq!(
            probe.raw_response_snippet.as_deref().unwrap().len(),
            SNIPPET_MAX_CHARS
        );

        // newest first
        let recent = probes.list_recent(a.id, 1).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert!(!recent[0].is_up);
        assert!(recent[0].id > probe.id - 1);

        // uptime: 1 up + 1 down = 50%
        let uptime = probes.uptime_percentage(a.id, 24).await.unwrap();
        assert_eq!(uptime, Some(50.0));

        // unknown point: empty list, no uptime
        let other = Uuid::now_v7();
        assert!(probes.list_recent(other, 5).await.unwrap().is_empty());
        assert_eq!(probes.uptime_percentage(other, 24).await.unwrap(), None);
    }

    #[tokio::test]
    async fn uptime_ignores_results_outside_window() {
        let stores = FakeStores::new();
        let repo = &stores.alert_points;
        let probes = &stores.probe_results;

        let a = repo
            .create(sample_new_point("old", "https://old.example.com"))
            .await
            .unwrap();

        // backdate one stored result beyond the 24h window
        let probe = probes
            .insert(sample_new_probe_result(a.id, false))
            .await
            .unwrap();
        {
            let mut st = probes.0.lock();
            let stored = st
                .probe_results
                .iter_mut()
                .find(|r| r.id == probe.id)
                .unwrap();
            stored.timestamp = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).single().unwrap();
        }

        assert_eq!(probes.uptime_percentage(a.id, 24).await.unwrap(), None);
    }

    #[tokio::test]
    async fn probe_insert_unknown_point_fails() {
        let probes = FakeProbeResultStore::new();
        let err = probes
            .insert(sample_new_probe_result(Uuid::now_v7(), true))
            .await
            .unwrap_err();
        assert!(matches!(err, RepoError::Other(_)));
    }

    #[tokio::test]
    async fn one_open_alert_per_point() {
        let (_repo, a, b) = two_points().await;
        let alerts = FakeActiveAlertStore::new();

        let alert = alerts
            .insert(sample_new_alert(a.id, AlertType::New))
            .await
            .unwrap();
        assert_eq!(alert.alert_type, AlertType::New);

        // second open alert for the same point conflicts
        assert!(matches!(
            alerts
                .insert(sample_new_alert(a.id, AlertType::Update))
                .await,
            Err(RepoError::Conflict)
        ));

        // other point is unaffected
        let b_alert = alerts
            .insert(sample_new_alert(b.id, AlertType::Update))
            .await
            .unwrap();
        assert_eq!(alerts.list_open().await.unwrap().len(), 2);

        // latest_open
        let latest = alerts.latest_open(a.id).await.unwrap().unwrap();
        assert_eq!(latest.id, alert.id);
        assert!(alerts.latest_open(Uuid::now_v7()).await.unwrap().is_none());

        // resolve then re-insert works
        assert_eq!(alerts.resolve(a.id).await.unwrap(), 1);
        assert_eq!(alerts.resolve(a.id).await.unwrap(), 0); // already resolved
        assert_eq!(alerts.list_open().await.unwrap().len(), 1);
        assert_eq!(alerts.list_open().await.unwrap()[0].id, b_alert.id);
        assert!(alerts
            .insert(sample_new_alert(a.id, AlertType::Remove))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn delete_cascades_probe_results_and_alerts() {
        let stores = FakeStores::new();
        let repo = &stores.alert_points;
        let probes = &stores.probe_results;
        let alerts = &stores.active_alerts;

        let a = repo
            .create(sample_new_point("cascade", "https://c.example.com"))
            .await
            .unwrap();

        probes
            .insert(sample_new_probe_result(a.id, true))
            .await
            .unwrap();
        alerts
            .insert(sample_new_alert(a.id, AlertType::New))
            .await
            .unwrap();

        assert!(repo.delete(a.id).await.unwrap());
        assert!(probes.list_recent(a.id, 10).await.unwrap().is_empty());
        assert!(alerts.list_open().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn failure_injection_is_one_shot() {
        let (repo, a, _b) = two_points().await;

        repo.fail_next(RepoError::Other("injected".to_string()));
        assert!(matches!(repo.get(a.id).await, Err(RepoError::Other(_))));
        // next call works again
        assert!(repo.get(a.id).await.is_ok());
    }

    #[test]
    fn ilike_matching() {
        assert!(ilike_match("%alph%", "The ALPHA service"));
        assert!(ilike_match("%a_c%", "abc"));
        assert!(!ilike_match("%a_c%", "abbc"));
        assert!(ilike_match("%a\\%b%", "xA%By"));
        assert!(!ilike_match("%a\\%b%", "xaby"));
        assert!(ilike_match("exact", "EXACT"));
        assert!(ilike_match("multi%part", "multiWORDpart"));
        // %multi%part% matches any string with "multi" before "part"
        assert!(ilike_match("multi%part", "multiWORDthenOTHERpart"));
        assert!(!ilike_match("multi%part", "partBEFOREmulti"));
    }
}
