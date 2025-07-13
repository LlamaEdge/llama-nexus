use axum::{Json, http::StatusCode, response::IntoResponse};
use thiserror::Error;

pub type ServerResult<T> = std::result::Result<T, ServerError>;

#[derive(Error, Debug, Clone)]
pub enum ServerError {
    #[error("{0}")]
    Operation(String),
    #[error(
        "Not found available server. Please register a(n) {0} server via the `/admin/servers/register` endpoint."
    )]
    NotFoundServer(String),
    #[error("Invalid server kind: {0}")]
    InvalidServerKind(String),
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Failed to load config: {0}")]
    FailedToLoadConfig(String),
    #[error("Mcp server returned empty content")]
    McpEmptyContent,
    #[error("Mcp server not found")]
    McpNotFoundClient,
    #[error("Mcp operation failed: {0}")]
    McpOperation(String),
}
impl IntoResponse for ServerError {
    fn into_response(self) -> axum::response::Response {
        let (status, err_response) = match &self {
            ServerError::Operation(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            ServerError::NotFoundServer(e) => (StatusCode::NOT_FOUND, e.to_string()),
            ServerError::InvalidServerKind(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            ServerError::BadRequest(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            ServerError::FailedToLoadConfig(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            ServerError::McpEmptyContent => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Mcp server returned empty content".to_string(),
            ),
            ServerError::McpNotFoundClient => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Mcp server not found".to_string(),
            ),
            ServerError::McpOperation(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };

        (status, Json(err_response)).into_response()
    }
}
