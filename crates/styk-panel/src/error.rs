use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("{1}")]
    Forbidden(&'static str, String),
    #[error("{0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    #[error("too many attempts, try again later")]
    TooManyRequests,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

/// Agent refusals ("read-only", "file changed on disk", …) are meant for the user;
/// transport failures stay internal.
pub fn agent_error(e: anyhow::Error) -> ApiError {
    match e.to_string().strip_prefix("styk-agent: ") {
        Some(msg) => ApiError::BadRequest(msg.to_string()),
        None => ApiError::Internal(e),
    }
}

impl ApiError {
    pub fn sudo_required() -> Self {
        Self::Forbidden("sudo_required", "confirm with a TOTP code".into())
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl From<bollard::errors::Error> for ApiError {
    fn from(e: bollard::errors::Error) -> Self {
        match e {
            bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            } => Self::NotFound,
            e => Self::Internal(e.into()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden(code, _) => (StatusCode::FORBIDDEN, *code),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::Internal(e) => {
                tracing::error!("internal error: {e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        };
        let message = match &self {
            Self::Internal(_) => "internal error".to_string(),
            e => e.to_string(),
        };
        (status, Json(json!({ "error": code, "message": message }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
