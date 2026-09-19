use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub cors_origins: Vec<String>,
    pub frontend_build_dir: String,
    pub database_url: String,
    pub log_level: String,
    pub redis_url: String,
    /// Scheduler poll interval in seconds (env `SCHEDULER_TICK_SECONDS`).
    pub scheduler_tick_seconds: u64,
    /// Default probe timeout in ms (env `PROBE_TIMEOUT_MS_DEFAULT`).
    pub probe_timeout_ms_default: i64,
    /// Number of parallel probe workers (env `WORKER_CONCURRENCY`).
    pub worker_concurrency: usize,
    /// Consecutive failing probes before an alert is raised
    /// (env `PROBE_FAILURE_THRESHOLD`, min 1).
    pub probe_failure_threshold: u32,
}

impl Config {
    /// Load configuration from environment variables with a `.env` file as fallback.
    pub fn load() -> Self {
        let _ = dotenvy::dotenv();

        Self {
            host: std::env::var("APP_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: std::env::var("APP_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3000),
            cors_origins: std::env::var("APP_CORS_ORIGINS")
                .ok()
                .map(|v| v.split(',').map(String::from).collect())
                .unwrap_or_else(|| vec!["http://localhost:5173".to_string()]),
            frontend_build_dir: std::env::var("FRONTEND_BUILD_DIR")
                .unwrap_or_else(|_| "frontend/build".to_string()),
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                "postgres://postgres:postgres@localhost:5432/gis_sentinel".to_string()
            }),
            log_level: std::env::var("LOG_LEVEL")
                .unwrap_or_else(|_| "info,tower_http=debug".to_string()),
            redis_url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:637".to_string()),
            scheduler_tick_seconds: std::env::var("SCHEDULER_TICK_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
            probe_timeout_ms_default: std::env::var("PROBE_TIMEOUT_MS_DEFAULT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10_000),
            worker_concurrency: std::env::var("WORKER_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2),
            probe_failure_threshold: std::env::var("PROBE_FAILURE_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2),
        }
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// How often the scheduler polls for due targets.
    pub fn scheduler_tick(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.scheduler_tick_seconds.max(1))
    }

    /// Default probe timeout (ms) when a target configuration lacks one.
    pub fn probe_timeout_default(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.probe_timeout_ms_default.max(1_000) as u64)
    }
}
