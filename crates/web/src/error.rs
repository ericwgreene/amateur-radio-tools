//! Application error type with HTML rendering.
//!
//! Every handler returns `Result<_, AppError>`. When an error bubbles up, actix-web calls
//! [`AppError::error_response`], which renders a friendly HTML error page.

use actix_web::{HttpResponse, http::StatusCode, http::header::ContentType};
use askama::Template;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("page not found")]
    NotFound,
    #[error("you don't have permission to view this page")]
    Forbidden,
    #[error("{0}")]
    BadRequest(String),
    #[error("authentication is not configured on this server")]
    AuthNotConfigured,
    #[error("authentication failed: {0}")]
    Auth(String),
    /// Anything unexpected. The underlying detail is logged, not shown to the user.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sea_orm::DbErr> for AppError {
    fn from(e: sea_orm::DbErr) -> Self {
        AppError::Internal(anyhow::Error::new(e))
    }
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage<'a> {
    status: u16,
    title: &'a str,
    message: String,
}

impl actix_web::ResponseError for AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::AuthNotConfigured => StatusCode::SERVICE_UNAVAILABLE,
            AppError::Auth(_) => StatusCode::UNAUTHORIZED,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        // Log the full detail of unexpected errors; never leak it to the client.
        if let AppError::Internal(e) = self {
            tracing::error!(error = ?e, "internal server error");
        }
        let status = self.status_code();
        let title = match self {
            AppError::NotFound => "Not found",
            AppError::Auth(_) => "Sign in required",
            AppError::Forbidden => "Forbidden",
            AppError::BadRequest(_) => "Bad request",
            AppError::AuthNotConfigured => "Authentication unavailable",
            AppError::Internal(_) => "Something went wrong",
        };
        let message = match self {
            AppError::Internal(_) => "An unexpected error occurred. Please try again.".to_string(),
            other => other.to_string(),
        };

        let body = ErrorPage {
            status: status.as_u16(),
            title,
            message,
        }
        .render()
        .unwrap_or_else(|_| "Error".to_string());

        HttpResponse::build(status)
            .content_type(ContentType::html())
            .body(body)
    }
}
