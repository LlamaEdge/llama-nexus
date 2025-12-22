use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
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
    #[error("Failed to load config: {0}")]
    FailedToLoadConfig(String),
    #[error("Mcp server returned empty content")]
    McpEmptyContent,
    #[error("Mcp operation failed: {0}")]
    McpOperation(String),
    #[error("React loop exceeded maximum iterations ({0})")]
    MaxIterationsExceeded(u32),
    #[error("React loop timeout after {0} seconds")]
    ReactTimeout(u64),
}
impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, message, error_type, param, code) = match &self {
            ServerError::Operation(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                e.clone(),
                "internal_error".into(),
                None,
                Some("operation_failed".into()),
            ),
            ServerError::NotFoundServer(kind) => (
                StatusCode::NOT_FOUND,
                format!(
                    "Not found available server. Please register a(n) {kind} server via the `/admin/servers/register` endpoint."
                ),
                "not_found".into(),
                Some("server_kind".into()),
                Some("not_found_server".into()),
            ),
            ServerError::InvalidServerKind(kind) => (
                StatusCode::BAD_REQUEST,
                format!("Invalid server kind: {kind}"),
                "invalid_request_error".into(),
                Some("server_kind".into()),
                Some("invalid_server_kind".into()),
            ),
            ServerError::FailedToLoadConfig(e) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to load config: {e}"),
                "invalid_request_error".into(),
                Some("config".into()),
                Some("failed_to_load_config".into()),
            ),
            ServerError::McpEmptyContent => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Mcp server returned empty content".into(),
                "internal_error".into(),
                None,
                Some("mcp_empty".into()),
            ),
            ServerError::McpOperation(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Mcp operation failed: {e}"),
                "internal_error".into(),
                None,
                Some("mcp_operation_failed".into()),
            ),
            ServerError::MaxIterationsExceeded(max) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("React loop exceeded maximum iterations ({max})"),
                "internal_error".into(),
                Some("max_react_iterations".into()),
                Some("max_iterations_exceeded".into()),
            ),
            ServerError::ReactTimeout(secs) => (
                StatusCode::GATEWAY_TIMEOUT,
                format!("React loop timeout after {secs} seconds"),
                "timeout_error".into(),
                Some("react_timeout_secs".into()),
                Some("react_timeout".into()),
            ),
        };

        let body = OpenAIErrorResponse {
            error: OpenAIError {
                message,
                error_type,
                param,
                code,
            },
        };

        (status, Json(body)).into_response()
    }
}

#[derive(Serialize)]
struct OpenAIErrorResponse {
    error: OpenAIError,
}

#[derive(Serialize)]
struct OpenAIError {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: Option<String>,
}
