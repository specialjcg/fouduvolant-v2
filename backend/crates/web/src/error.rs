//! HTTP error type and mappings from the application/aggregate errors.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use app::{AggregateError, AppError};

/// An HTTP error with a status and a user-facing message.
pub(crate) struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub(crate) fn not_found(what: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: format!("{what} not found"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}

impl<E: std::error::Error> From<AggregateError<E>> for ApiError {
    fn from(e: AggregateError<E>) -> Self {
        let status = match &e {
            AggregateError::UserError(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AggregateError::AggregateConflict => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: e.to_string(),
        }
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        let status = match e {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Command(_) => StatusCode::UNPROCESSABLE_ENTITY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: e.to_string(),
        }
    }
}
