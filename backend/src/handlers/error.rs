//! Uniform REST API error handling (task 2.1).
//!
//! Every `/api/v1` handler returns `Result<_, ApiError>`; `ApiError` maps to
//! an HTTP status plus a uniform JSON body:
//!
//! ```json
//! { "error": { "code": "validation_failed", "message": "…", "details": { "url": "…" } } }
//! ```

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
/// Error type surfaced by API handlers.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Resource does not exist (mapped from `sqlx::Error::RowNotFound`).
    #[error("{0} not found")]
    NotFound(&'static str),
    /// Malformed request (e.g. `UpdateAlertPoint` with no fields set).
    #[error("{0}")]
    BadRequest(String),
    /// Payload failed validation; carries (field, message) pairs.
    #[error("validation failed")]
    Validation(Vec<(String, String)>),
    /// Unexpected failure — full error is logged, client gets an opaque message.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::RowNotFound => ApiError::NotFound("resource"),
            sqlx::Error::Database(db_err)
                if db_err_is_unique_violation(db_err.as_ref())
                    || db_err_is_check_violation(db_err.as_ref()) =>
            {
                // Keep payload-driven 4xx semantics for constraint violations
                // (e.g. check_interval_seconds <= 0 smuggled past validation).
                tracing::warn!(
                    constraint = db_err.constraint().unwrap_or("?"),
                    "constraint violation"
                );
                ApiError::BadRequest("request violates a database constraint".to_string())
            }
            _ => {
                tracing::error!(error = %e, "database error in API handler");
                ApiError::Internal(anyhow::anyhow!(e).context("database error"))
            }
        }
    }
}

fn db_err_is_unique_violation(e: &dyn sqlx::error::DatabaseError) -> bool {
    e.kind() == sqlx::error::ErrorKind::UniqueViolation
}

fn db_err_is_check_violation(e: &dyn sqlx::error::DatabaseError) -> bool {
    e.constraint()
        .is_some_and(|c| c.contains("check") || c.contains("_check"))
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, message, details) = match &self {
            ApiError::NotFound(what) => (
                StatusCode::NOT_FOUND,
                "not_found",
                format!("{what} not found"),
                None,
            ),
            ApiError::BadRequest(msg) => {
                (StatusCode::BAD_REQUEST, "bad_request", msg.clone(), None)
            }
            ApiError::Validation(fields) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_failed",
                "request validation failed".to_string(),
                Some(serde_json::Value::Object(
                    fields
                        .iter()
                        .map(|(f, m)| (f.clone(), serde_json::Value::String(m.clone())))
                        .collect(),
                )),
            ),
            ApiError::Internal(e) => {
                tracing::error!(error = ?e, "internal error in API handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "an internal error occurred".to_string(),
                    None,
                )
            }
        };

        let body = serde_json::json!({
            "error": {
                "code": code,
                "message": message,
                "details": details,
            }
        });

        (status, Json(body)).into_response()
    }
}

/// Convenience alias for handler return types.
pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_not_found_maps_to_not_found() {
        let err: ApiError = sqlx::Error::RowNotFound.into();
        assert!(matches!(err, ApiError::NotFound(_)));
    }
}
