use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub cors_origins: Vec<String>,
    pub frontend_build_dir: String,
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
        }
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
