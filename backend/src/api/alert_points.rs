//! Handlers for the Alert Points CRUD endpoints (task 2.1).
//!
//! Routes (nested under `/api/v1`):
//! - `GET    /alert-points`         list w/ filtering + pagination
//! - `POST   /alert-points`         create (`201`)
//! - `GET    /alert-points/{id}`    details + recent probe history
//! - `PUT    /alert-points/{id}`    partial update
//! - `DELETE /alert-points/{id}`    delete (`204`)
//! - `POST   /alert-points/{id}/test` enqueue an on-demand probe job (`202`)

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use validator::{Validate, ValidationError};

use crate::api::{
    ApiError, ApiQuery, ApiState, ValidateRequest, ValidatedJson, DEFAULT_PROBE_TIMEOUT_MS,
};
use crate::db::models::{
    ActiveAlert, AlertPoint, HttpMethod, NewAlertPoint, ProbeResult, ServiceType,
    UpdateAlertPoint as DbUpdateAlertPoint,
};
use crate::db::repo::alert_points::ListFilter;
use crate::queue::ProbeJob;

// ---------------------------------------------------------------------------
// DTOs & validation
// ---------------------------------------------------------------------------

/// Field -> error messages for the `details` object of the error envelope.
type FieldErrors = HashMap<String, Vec<String>>;

/// Build a `ValidationError` with a human-readable `message`
/// (`ValidationError::new` only sets `code`, which the envelope ignores).
fn validation_error(message: &'static str) -> ValidationError {
    let mut e = ValidationError::new("invalid");
    e.message = Some(message.into());
    e
}

/// Validate a `name`: trimmed non-empty, at most 200 characters.
fn validate_name(name: &str) -> Result<(), ValidationError> {
    if name.trim().is_empty() {
        return Err(validation_error("must not be empty"));
    }
    if name.chars().count() > 200 {
        return Err(validation_error("must be at most 200 characters long"));
    }
    Ok(())
}

/// Validate that `url` parses as an absolute `http`/`https` URL.
/// Used both by the `validator` derive (custom function) and by manual checks.
fn validate_url(url: &str) -> Result<(), ValidationError> {
    match url::Url::parse(url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => Ok(()),
        Ok(_) => Err(validation_error("must be an http or https URL")),
        Err(_) => Err(validation_error("is not a valid absolute URL")),
    }
}

/// `custom_headers` must be a JSON object whose values are all strings.
fn validate_custom_headers(headers: &Value, errors: &mut FieldErrors) {
    if let Some(map) = headers.as_object() {
        if let Some((key, _)) = map.iter().find(|(_, v)| !v.is_string()) {
            errors.insert(
                "custom_headers".to_string(),
                vec![format!("value of header '{key}' must be a string")],
            );
        }
    } else {
        errors.insert(
            "custom_headers".to_string(),
            vec!["must be a JSON object of string values".to_string()],
        );
    }
}

/// `auth_config`, when present, must be a JSON object.
fn validate_auth_config(auth: &Value, errors: &mut FieldErrors) {
    if !auth.is_object() {
        errors.insert(
            "auth_config".to_string(),
            vec!["must be a JSON object".to_string()],
        );
    }
}

fn push_error(errors: &mut FieldErrors, field: &str, message: &str) {
    errors
        .entry(field.to_string())
        .or_default()
        .push(message.to_string());
}

fn check_url(errors: &mut FieldErrors, url: &str) {
    if let Err(e) = validate_url(url) {
        push_error(errors, "url", &e.to_string());
    }
}

fn check_interval(errors: &mut FieldErrors, field: &str, value: i32, min: i32, max: i32) {
    if !(min..=max).contains(&value) {
        push_error(errors, field, &format!("must be between {min} and {max}"));
    }
}

/// Request body for `POST /api/v1/alert-points`.
#[derive(Debug, Deserialize, Validate)]
pub struct CreateAlertPointRequest {
    #[validate(custom(function = "validate_name"))]
    pub name: String,
    #[validate(length(min = 1, max = 2048), custom(function = "validate_url"))]
    pub url: String,
    pub service_type: ServiceType,
    #[validate(range(min = 1, max = 86_400))]
    pub check_interval_seconds: i32,
    #[validate(range(min = 1, max = 600_000))]
    pub expected_response_time_ms: i32,
    #[serde(default)]
    pub http_method: HttpMethod,
    #[serde(default)]
    pub custom_headers: Value,
    #[serde(default)]
    pub auth_config: Option<Value>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl ValidateRequest for CreateAlertPointRequest {
    fn validate_request(&self) -> Result<(), FieldErrors> {
        let mut errors = match self.validate() {
            Ok(()) => FieldErrors::new(),
            Err(e) => crate::api::errors_to_map(e),
        };
        validate_custom_headers(&self.custom_headers, &mut errors);
        if let Some(auth) = &self.auth_config {
            validate_auth_config(auth, &mut errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl From<CreateAlertPointRequest> for NewAlertPoint {
    fn from(req: CreateAlertPointRequest) -> Self {
        NewAlertPoint {
            name: req.name,
            url: req.url,
            service_type: req.service_type,
            check_interval_seconds: req.check_interval_seconds,
            expected_response_time_ms: req.expected_response_time_ms,
            http_method: req.http_method,
            custom_headers: req.custom_headers,
            auth_config: req.auth_config,
            enabled: req.enabled,
        }
    }
}

/// Request body for `PUT /api/v1/alert-points` (partial update: only
/// `Some` fields change; `auth_config: null` clears it).
#[derive(Debug, Default, Deserialize)]
pub struct UpdateAlertPointRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub service_type: Option<ServiceType>,
    pub check_interval_seconds: Option<i32>,
    pub expected_response_time_ms: Option<i32>,
    pub http_method: Option<HttpMethod>,
    pub custom_headers: Option<Value>,
    /// Three-state: absent = unchanged, `null` = cleared, value = set.
    pub auth_config: Option<Option<Value>>,
    pub enabled: Option<bool>,
}

impl ValidateRequest for UpdateAlertPointRequest {
    /// Validate only the provided (`Some`) fields.
    fn validate_request(&self) -> Result<(), FieldErrors> {
        let mut errors = FieldErrors::new();
        if let Some(name) = &self.name {
            let trimmed = name.trim();
            if trimmed.is_empty() || name.chars().count() > 200 {
                push_error(&mut errors, "name", "must be 1 to 200 characters long");
            }
        }
        if let Some(url) = &self.url {
            check_url(&mut errors, url);
        }
        if let Some(v) = self.check_interval_seconds {
            check_interval(&mut errors, "check_interval_seconds", v, 1, 86_400);
        }
        if let Some(v) = self.expected_response_time_ms {
            check_interval(&mut errors, "expected_response_time_ms", v, 1, 600_000);
        }
        if let Some(headers) = &self.custom_headers {
            validate_custom_headers(headers, &mut errors);
        }
        if let Some(Some(auth)) = &self.auth_config {
            validate_auth_config(auth, &mut errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl From<UpdateAlertPointRequest> for DbUpdateAlertPoint {
    fn from(req: UpdateAlertPointRequest) -> Self {
        DbUpdateAlertPoint {
            name: req.name,
            url: req.url,
            service_type: req.service_type,
            check_interval_seconds: req.check_interval_seconds,
            expected_response_time_ms: req.expected_response_time_ms,
            http_method: req.http_method,
            custom_headers: req.custom_headers,
            auth_config: req.auth_config,
            enabled: req.enabled,
        }
    }
}

/// Query parameters for `GET /api/v1/alert-points`.
#[derive(Debug, Default, Deserialize)]
pub struct ListAlertPointsQuery {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub enabled: Option<bool>,
    pub service_type: Option<ServiceType>,
    pub search: Option<String>,
}

/// Clamp page params to sane bounds (page >= 1, page_size in 1..=100).
fn normalize_list_params(page: Option<u32>, page_size: Option<u32>) -> (u32, u32) {
    (
        page.unwrap_or(1).max(1),
        page_size.unwrap_or(20).clamp(1, 100),
    )
}

/// Escape LIKE wildcards and wrap into a contains-pattern.
fn like_pattern(search: &str) -> String {
    format!(
        "%{}%",
        search
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Pagination metadata of the list envelope.
#[derive(Debug, Serialize)]
pub struct PaginationMeta {
    pub page: u32,
    pub page_size: u32,
    pub total: i64,
    pub total_pages: u64,
}

#[derive(Debug, Serialize)]
pub struct Paginated<T> {
    pub data: Vec<T>,
    pub pagination: PaginationMeta,
}

/// `GET /api/v1/alert-points/{id}` response: details + recent probe history.
#[derive(Debug, Serialize)]
pub struct AlertPointDetail {
    pub alert_point: AlertPoint,
    pub recent_results: Vec<ProbeResult>,
    /// Uptime percentage over the last 24 hours; `null` without results.
    pub uptime_24h_pct: Option<f64>,
    /// Currently open alert, if any.
    pub open_alert: Option<ActiveAlert>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/alert-points` — list with filtering & pagination.
pub async fn list(
    ApiQuery(q): ApiQuery<ListAlertPointsQuery>,
    State(state): State<ApiState>,
) -> Result<Json<Paginated<AlertPoint>>, ApiError> {
    let (page, page_size) = normalize_list_params(q.page, q.page_size);
    let search = q
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(like_pattern);
    let filter = ListFilter {
        enabled: q.enabled,
        service_type: q.service_type,
        search,
    };

    let total = state.alert_points.count_filtered(&filter).await?;
    let data = state
        .alert_points
        .list_filtered(
            &filter,
            page_size as i64,
            ((page - 1) as i64) * (page_size as i64),
        )
        .await?;

    Ok(Json(Paginated {
        data,
        pagination: PaginationMeta {
            page,
            page_size,
            total,
            total_pages: (total.max(0) as u64).div_ceil(page_size as u64),
        },
    }))
}

/// `POST /api/v1/alert-points` — create a monitoring point (`201`).
pub async fn create(
    State(state): State<ApiState>,
    ValidatedJson(req): ValidatedJson<CreateAlertPointRequest>,
) -> Result<Response, ApiError> {
    let point = state.alert_points.create(req.into()).await?;
    Ok((StatusCode::CREATED, Json(point)).into_response())
}

/// `GET /api/v1/alert-points/{id}` — details + recent probe history.
pub async fn get(
    Path(id): Path<Uuid>,
    State(state): State<ApiState>,
) -> Result<Json<AlertPointDetail>, ApiError> {
    let alert_point = state.alert_points.get(id).await?;
    let recent_results = state.probe_results.list_recent(id, 20).await?;
    let uptime_24h_pct = state.probe_results.uptime_percentage(id, 24).await?;
    let open_alert = state.active_alerts.latest_open(id).await?;

    Ok(Json(AlertPointDetail {
        alert_point,
        recent_results,
        uptime_24h_pct,
        open_alert,
    }))
}

/// `PUT /api/v1/alert-points/{id}` — partial update of settings.
pub async fn update(
    Path(id): Path<Uuid>,
    State(state): State<ApiState>,
    ValidatedJson(req): ValidatedJson<UpdateAlertPointRequest>,
) -> Result<Json<AlertPoint>, ApiError> {
    let point = state.alert_points.update(id, req.into()).await?;
    Ok(Json(point))
}

/// `DELETE /api/v1/alert-points/{id}` — delete (`204`); history cascades.
pub async fn delete(
    Path(id): Path<Uuid>,
    State(state): State<ApiState>,
) -> Result<StatusCode, ApiError> {
    if state.alert_points.delete(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// `POST /api/v1/alert-points/{id}/test` — enqueue an on-demand probe job.
///
/// The job is pushed onto `sentinel:probe_jobs` and consumed by the worker
/// (task 2.3). Queue errors surface as `503 queue unavailable`.
pub async fn test(
    Path(id): Path<Uuid>,
    State(state): State<ApiState>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let point = state.alert_points.get(id).await?;
    let job = ProbeJob {
        target_id: point.id,
        target_name: point.name,
        url: point.url,
        service_type: point.service_type,
        expected_time_ms: point.expected_response_time_ms,
        timeout_ms: DEFAULT_PROBE_TIMEOUT_MS,
        http_method: point.http_method,
        custom_headers: serde_json::from_value(point.custom_headers).unwrap_or_default(),
        auth: point
            .auth_config
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
    };
    state.queue.enqueue_probe_job(&job).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "queued": true, "job": job })),
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_create_request() -> CreateAlertPointRequest {
        CreateAlertPointRequest {
            name: "Geoserver".to_string(),
            url: "https://example.com/geoserver/wms".to_string(),
            service_type: ServiceType::Wms,
            check_interval_seconds: 30,
            expected_response_time_ms: 500,
            http_method: HttpMethod::Get,
            custom_headers: json!({}),
            auth_config: None,
            enabled: true,
        }
    }

    #[test]
    fn valid_create_request_passes() {
        assert!(valid_create_request().validate_request().is_ok());
    }

    #[test]
    fn empty_name_is_rejected() {
        let mut req = valid_create_request();
        req.name = "  ".to_string();
        let errors = req.validate_request().unwrap_err();
        assert!(errors.contains_key("name"));
    }

    #[test]
    fn overly_long_name_is_rejected() {
        let mut req = valid_create_request();
        req.name = "x".repeat(201);
        let errors = req.validate_request().unwrap_err();
        assert!(errors.contains_key("name"));
    }

    #[test]
    fn non_http_url_is_rejected() {
        let mut req = valid_create_request();
        req.url = "ftp://example.com/wms".to_string();
        let errors = req.validate_request().unwrap_err();
        assert_eq!(
            errors.get("url").map(Vec::as_slice),
            Some(&["must be an http or https URL".to_string()][..])
        );
    }

    #[test]
    fn unparseable_url_is_rejected() {
        let mut req = valid_create_request();
        req.url = "not a url".to_string();
        assert!(req.validate_request().unwrap_err().contains_key("url"));
    }

    #[test]
    fn zero_interval_is_rejected() {
        let mut req = valid_create_request();
        req.check_interval_seconds = 0;
        assert!(req
            .validate_request()
            .unwrap_err()
            .contains_key("check_interval_seconds"));
    }

    #[test]
    fn zero_expected_latency_is_rejected() {
        let mut req = valid_create_request();
        req.expected_response_time_ms = -5;
        assert!(req
            .validate_request()
            .unwrap_err()
            .contains_key("expected_response_time_ms"));
    }

    #[test]
    fn non_object_custom_headers_are_rejected() {
        let mut req = valid_create_request();
        req.custom_headers = json!("nope");
        let errors = req.validate_request().unwrap_err();
        assert!(errors.contains_key("custom_headers"));

        req.custom_headers = json!({"X-Token": 42});
        let errors = req.validate_request().unwrap_err();
        assert!(errors.contains_key("custom_headers"));
    }

    #[test]
    fn non_object_auth_config_is_rejected() {
        let mut req = valid_create_request();
        req.auth_config = Some(json!([1, 2, 3]));
        assert!(req
            .validate_request()
            .unwrap_err()
            .contains_key("auth_config"));
    }

    #[test]
    fn update_request_validates_only_some_fields() {
        // all-None request is valid
        let req = UpdateAlertPointRequest::default();
        assert!(req.validate_request().is_ok());

        // only the Some fields are checked: valid URL, bad interval
        let req = UpdateAlertPointRequest {
            url: Some("https://example.com/wms".to_string()),
            check_interval_seconds: Some(0),
            ..Default::default()
        };
        let errors = req.validate_request().unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors.contains_key("check_interval_seconds"));

        // clearing auth_config (None inside) must not be shape-checked
        let req = UpdateAlertPointRequest {
            auth_config: Some(None),
            ..Default::default()
        };
        assert!(req.validate_request().is_ok());
    }

    #[test]
    fn list_params_are_clamped() {
        assert_eq!(normalize_list_params(None, None), (1, 20));
        assert_eq!(normalize_list_params(Some(0), Some(0)), (1, 1));
        assert_eq!(normalize_list_params(Some(999), Some(500)), (999, 100));
    }

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_pattern("abc"), "%abc%");
        assert_eq!(like_pattern("a%c_d\\e"), "%a\\%c\\_d\\\\e%");
    }

    #[test]
    fn update_request_maps_into_db_payload() {
        let req = UpdateAlertPointRequest {
            name: Some("renamed".to_string()),
            auth_config: Some(None),
            ..Default::default()
        };
        let db: DbUpdateAlertPoint = req.into();
        assert_eq!(db.name.as_deref(), Some("renamed"));
        assert_eq!(db.auth_config, Some(None));
    }
}
