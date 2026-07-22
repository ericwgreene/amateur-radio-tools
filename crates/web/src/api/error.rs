//! JSON error type for the REST API.
//!
//! Unlike [`crate::error::AppError`] (which renders HTML for the browser), API errors are
//! serialized as JSON so machine clients get a consistent, parseable shape:
//! `{ "error": "unauthorized", "message": "..." }`.

use actix_web::{HttpResponse, http::StatusCode, http::header};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("authentication required")]
    Unauthorized,
    #[error("{0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Internal(String),
}

impl ApiError {
    pub fn internal(msg: impl Into<String>) -> Self {
        ApiError::Internal(msg.into())
    }
}

impl From<sea_orm::DbErr> for ApiError {
    fn from(e: sea_orm::DbErr) -> Self {
        tracing::error!(error = ?e, "database error in API handler");
        ApiError::Internal("database error".to_string())
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
}

impl actix_web::ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        let code = self.status_code();
        let error = match self {
            ApiError::Unauthorized => "unauthorized",
            ApiError::BadRequest(_) => "bad_request",
            ApiError::NotFound => "not_found",
            ApiError::Internal(_) => "internal_error",
        };
        // Public message: hide internal detail behind a generic string.
        let message = match self {
            ApiError::Internal(_) => "an internal error occurred".to_string(),
            other => other.to_string(),
        };

        let mut builder = HttpResponse::build(code);
        if matches!(self, ApiError::Unauthorized) {
            builder.insert_header((header::WWW_AUTHENTICATE, "Bearer"));
        }
        builder.json(ErrorBody { error, message })
    }
}
