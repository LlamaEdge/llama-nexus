use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::Json};
use endpoints::chat::{
    ChatCompletionRequest, ChatCompletionRequestMessage, ChatCompletionUserMessageContent,
};
use tokio::sync::OnceCell;

use crate::{
    AppState as MainAppState,
    responses::{
        db::{Database, DatabaseError},
        models::{ResponseReply, ResponseRequest, Session},
    },
    server::RoutingPolicy,
};

#[derive(Debug)]
enum ResponseError {
    InvalidInput(String),
    SessionNotFound(String),
    DatabaseError(String),
    ChatBackendError(String),
}

impl ResponseError {
    fn to_http_error(&self) -> (StatusCode, String) {
        match self {
            Self::InvalidInput(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Self::SessionNotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Self::DatabaseError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            Self::ChatBackendError(msg) => (StatusCode::BAD_GATEWAY, msg.clone()),
        }
    }
}

impl From<ResponseError> for (StatusCode, String) {
    fn from(error: ResponseError) -> Self {
        error.to_http_error()
    }
}

impl From<DatabaseError> for ResponseError {
    fn from(error: DatabaseError) -> Self {
        match error {
            DatabaseError::SessionNotFound { id } => {
                ResponseError::SessionNotFound(format!("Session not found: {id}"))
            }
            DatabaseError::Sqlx(e) => ResponseError::DatabaseError(format!("Database error: {e}")),
            DatabaseError::Serialization(e) => {
                ResponseError::DatabaseError(format!("Data serialization error: {e}"))
            }
            DatabaseError::InvalidSessionData => {
                ResponseError::DatabaseError("Invalid session data".to_string())
            }
        }
    }
}

pub struct ResponsesAppState {
    db: OnceCell<Arc<Database>>,
    db_path: String,
    pub main_state: Arc<MainAppState>,
}

impl ResponsesAppState {
    pub fn new(db_path: String, main_state: Arc<MainAppState>) -> Self {
        Self {
            db: OnceCell::new(),
            db_path,
            main_state,
        }
    }

    async fn get_or_create_db(&self) -> Result<Arc<Database>, DatabaseError> {
        let db = self
            .db
            .get_or_try_init(|| async { Database::new(&self.db_path).await.map(Arc::new) })
            .await?;

        Ok(Arc::clone(db))
    }
}

pub async fn responses_handler(
    State(state): State<Arc<ResponsesAppState>>,
    Json(req): Json<ResponseRequest>,
) -> Result<Json<ResponseReply>, (StatusCode, String)> {
    match responses_handler_impl(state, req).await {
        Ok(response) => Ok(response),
        Err(error) => Err(error.into()),
    }
}

async fn responses_handler_impl(
    state: Arc<ResponsesAppState>,
    req: ResponseRequest,
) -> Result<Json<ResponseReply>, ResponseError> {
    if req.model.trim().is_empty() {
        return Err(ResponseError::InvalidInput(
            "Model name cannot be empty".to_string(),
        ));
    }
    if req.input.trim().is_empty() {
        return Err(ResponseError::InvalidInput(
            "Input message cannot be empty".to_string(),
        ));
    }

    let mut warnings = validate_request(&req)?;

    let model = req.model.clone();
    let response_id = format!("resp_{}", uuid::Uuid::new_v4().simple());

    let db = state.get_or_create_db().await?;

    let mut session = if let Some(prev_id) = &req.previous_response_id {
        match db.find_session_by_response_id(prev_id).await? {
            Some(existing_session) => existing_session,
            None => {
                return Err(ResponseError::SessionNotFound(format!(
                    "Previous response ID not found: {prev_id}"
                )));
            }
        }
    } else {
        Session::new(response_id.clone(), model.clone(), req.instructions.clone())
    };

    let user_tokens = estimate_tokens(&req.input);
    session.add_message(
        "user".to_string(),
        req.input.clone(),
        user_tokens,
        None,
        None,
    );

    let conversation = session.get_conversation_history();

    let mut messages = Vec::new();
    for (role, content) in conversation {
        match role.as_str() {
            "system" => {
                let system_msg = ChatCompletionRequestMessage::new_system_message(content, None);
                messages.push(system_msg);
            }
            "user" => {
                let user_msg = ChatCompletionRequestMessage::new_user_message(
                    ChatCompletionUserMessageContent::Text(content),
                    None,
                );
                messages.push(user_msg);
            }
            "assistant" => {
                let assistant_msg =
                    ChatCompletionRequestMessage::new_assistant_message(Some(content), None, None);
                messages.push(assistant_msg);
            }
            _ => {}
        }
    }

    let mut chat_request = ChatCompletionRequest {
        model: Some(model.clone()),
        messages,
        user: req
            .user
            .clone()
            .or_else(|| Some("responses-api".to_string())),
        stream: Some(false),
        ..Default::default()
    };

    if let Some(temp) = req.temperature {
        chat_request.temperature = Some(temp.into());
    }
    if let Some(top_p) = req.top_p {
        chat_request.top_p = Some(top_p.into());
    }
    if let Some(max_tokens) = req.max_output_tokens {
        chat_request.max_completion_tokens = Some(max_tokens as i32);
    }

    if let Some(user_tools) = &req.tools
        && !user_tools.is_empty()
    {
        let mcp_tool_names: Vec<String> = Vec::new();
        let tool_warnings = check_tool_conflicts(&req, &mcp_tool_names)?;
        warnings.extend(tool_warnings);

        warnings
            .push("User-supplied tools are not yet fully integrated with chat backend".to_string());
    }

    if let Some(_tool_choice) = &req.tool_choice {
        warnings.push("Tool choice specification not yet fully supported".to_string());
    }

    let chat_result = call_chat_backend(&state.main_state, chat_request)
        .await
        .map_err(|e| ResponseError::ChatBackendError(format!("Chat backend failed: {e}")))?;

    let output_tokens = estimate_tokens(&chat_result);
    session.add_message(
        "assistant".to_string(),
        chat_result.clone(),
        output_tokens,
        None,
        Some(response_id.clone()),
    );

    let final_result = chat_result;

    let mut extended_data = serde_json::json!({});
    if let Some(metadata) = &req.metadata {
        extended_data["metadata"] =
            serde_json::to_value(metadata).unwrap_or(serde_json::Value::Null);
    }
    if let Some(attachments) = &req.attachments {
        extended_data["attachments"] =
            serde_json::to_value(attachments).unwrap_or(serde_json::Value::Null);
    }
    if let Some(tool_resources) = &req.tool_resources {
        extended_data["tool_resources"] =
            serde_json::to_value(tool_resources).unwrap_or(serde_json::Value::Null);
    }

    if !extended_data.as_object().unwrap().is_empty() {
        session.extended_data = Some(extended_data);
    }

    db.save_session(&session).await?;

    let mut response = ResponseReply::new(
        response_id,
        model,
        final_result,
        user_tokens,
        output_tokens,
        req.previous_response_id,
    );

    response.metadata = req.metadata.clone();
    response.instructions = req.instructions.clone();
    response.modalities = req.modalities.clone();
    response.response_format = req.response_format.clone();

    if !warnings.is_empty() {
        response.warnings = Some(warnings);
    }

    Ok(Json(response))
}

fn estimate_tokens(text: &str) -> i32 {
    (text.len() as f32 / 4.0).ceil() as i32
}

fn validate_request(req: &ResponseRequest) -> Result<Vec<String>, ResponseError> {
    let mut warnings = Vec::new();

    if let Some(temp) = req.temperature
        && !(0.0..=2.0).contains(&temp)
    {
        return Err(ResponseError::InvalidInput(
            "Temperature must be between 0.0 and 2.0".to_string(),
        ));
    }

    if let Some(top_p) = req.top_p
        && !(0.0..=1.0).contains(&top_p)
    {
        return Err(ResponseError::InvalidInput(
            "top_p must be between 0.0 and 1.0".to_string(),
        ));
    }

    if let Some(max_tokens) = req.max_output_tokens
        && max_tokens == 0
    {
        return Err(ResponseError::InvalidInput(
            "max_output_tokens must be positive".to_string(),
        ));
    }

    if req.stream == Some(true) {
        return Err(ResponseError::InvalidInput(
            "Streaming not yet implemented".to_string(),
        ));
    }

    if req.modalities.is_some() {
        warnings.push("Multiple modalities not yet fully supported".to_string());
    }

    if req.reasoning.is_some() {
        warnings.push("Reasoning enhancements not yet supported by backend".to_string());
    }

    if req.response_format.is_some() {
        warnings.push("Custom response formats not yet fully supported".to_string());
    }

    if req.tool_resources.is_some() {
        warnings.push("Tool resources not yet fully supported".to_string());
    }

    if req.attachments.is_some() {
        warnings.push("Attachments not yet fully supported".to_string());
    }

    Ok(warnings)
}

fn check_tool_conflicts(
    req: &ResponseRequest,
    mcp_tool_names: &[String],
) -> Result<Vec<String>, ResponseError> {
    let mut warnings = Vec::new();

    if let Some(user_tools) = &req.tools {
        for tool in user_tools {
            if let Some(function) = &tool.function
                && mcp_tool_names.contains(&function.name)
            {
                return Err(ResponseError::InvalidInput(format!(
                    "Tool name '{}' conflicts with existing MCP tool",
                    function.name
                )));
            }
        }

        if !user_tools.is_empty() {
            warnings.push(format!(
                "User-supplied tools ({}) will be appended after MCP tools",
                user_tools.len()
            ));
        }
    }

    Ok(warnings)
}

async fn call_chat_backend(
    main_state: &Arc<MainAppState>,
    request: ChatCompletionRequest,
) -> Result<String, String> {
    let servers = main_state.server_group.read().await;
    let chat_servers = match servers.get(&crate::server::ServerKind::chat) {
        Some(servers) => servers,
        None => return Err("No chat server available".to_string()),
    };

    let target_server = match chat_servers.next().await {
        Ok(server) => server,
        Err(e) => return Err(format!("Failed to get chat server: {e}")),
    };

    let url = format!(
        "{}/chat/completions",
        target_server.url.trim_end_matches('/')
    );

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !response.status().is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Chat API Error: {error_text}"));
    }

    let chat_response: endpoints::chat::ChatCompletionObject = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    let text = chat_response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_ref())
        .map(|content| content.to_string())
        .unwrap_or_else(|| "No response content".to_string());

    Ok(text)
}

pub async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "responses-api"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);

        assert_eq!(estimate_tokens("a"), 1);

        assert_eq!(estimate_tokens("test"), 1);
        assert_eq!(estimate_tokens("hello"), 2);

        assert_eq!(estimate_tokens("This is a test message"), 6);

        assert_eq!(estimate_tokens("Hello, world!"), 4);
    }

    #[test]
    fn test_health_handler() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let response = runtime.block_on(health_handler());

        let json_value = response.0;
        assert_eq!(json_value["status"], "ok");
        assert_eq!(json_value["service"], "responses-api");
    }

    #[test]
    fn test_response_error_to_http_error() {
        let invalid_input = ResponseError::InvalidInput("Invalid request".to_string());
        let (status, msg) = invalid_input.to_http_error();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(msg, "Invalid request");

        let session_not_found = ResponseError::SessionNotFound("Session missing".to_string());
        let (status, msg) = session_not_found.to_http_error();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(msg, "Session missing");

        let database_error = ResponseError::DatabaseError("DB connection failed".to_string());
        let (status, msg) = database_error.to_http_error();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(msg, "DB connection failed");

        let backend_error = ResponseError::ChatBackendError("Backend unavailable".to_string());
        let (status, msg) = backend_error.to_http_error();
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(msg, "Backend unavailable");
    }

    #[test]
    fn test_response_error_from_conversion() {
        let error = ResponseError::InvalidInput("Bad data".to_string());
        let (status, msg): (StatusCode, String) = error.into();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(msg, "Bad data");
    }

    #[test]
    fn test_validate_request_temperature_range() {
        use crate::responses::models::ResponseRequest;

        let mut req = ResponseRequest {
            model: "test_model".to_string(),
            input: "test input".to_string(),
            instructions: None,
            previous_response_id: None,
            modalities: None,
            response_format: None,
            reasoning: None,
            temperature: Some(1.5),
            top_p: None,
            max_output_tokens: None,
            stream: None,
            include: None,
            tools: None,
            tool_choice: None,
            tool_resources: None,
            attachments: None,
            metadata: None,
            user: None,
        };

        let result = validate_request(&req);
        assert!(result.is_ok());

        req.temperature = Some(2.5);
        let result = validate_request(&req);
        assert!(result.is_err());

        req.temperature = Some(-0.1);
        let result = validate_request(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_request_top_p_range() {
        use crate::responses::models::ResponseRequest;

        let mut req = ResponseRequest {
            model: "test_model".to_string(),
            input: "test input".to_string(),
            instructions: None,
            previous_response_id: None,
            modalities: None,
            response_format: None,
            reasoning: None,
            temperature: None,
            top_p: Some(0.95),
            max_output_tokens: None,
            stream: None,
            include: None,
            tools: None,
            tool_choice: None,
            tool_resources: None,
            attachments: None,
            metadata: None,
            user: None,
        };

        let result = validate_request(&req);
        assert!(result.is_ok());

        req.top_p = Some(1.1);
        let result = validate_request(&req);
        assert!(result.is_err());

        req.top_p = Some(-0.1);
        let result = validate_request(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_request_max_output_tokens() {
        use crate::responses::models::ResponseRequest;

        let mut req = ResponseRequest {
            model: "test_model".to_string(),
            input: "test input".to_string(),
            instructions: None,
            previous_response_id: None,
            modalities: None,
            response_format: None,
            reasoning: None,
            temperature: None,
            top_p: None,
            max_output_tokens: Some(100),
            stream: None,
            include: None,
            tools: None,
            tool_choice: None,
            tool_resources: None,
            attachments: None,
            metadata: None,
            user: None,
        };

        let result = validate_request(&req);
        assert!(result.is_ok());

        req.max_output_tokens = Some(0);
        let result = validate_request(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_request_streaming_not_implemented() {
        use crate::responses::models::ResponseRequest;

        let req = ResponseRequest {
            model: "test_model".to_string(),
            input: "test input".to_string(),
            instructions: None,
            previous_response_id: None,
            modalities: None,
            response_format: None,
            reasoning: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            stream: Some(true),
            include: None,
            tools: None,
            tool_choice: None,
            tool_resources: None,
            attachments: None,
            metadata: None,
            user: None,
        };

        let result = validate_request(&req);
        assert!(result.is_err());
        match result {
            Err(ResponseError::InvalidInput(msg)) => {
                assert!(msg.contains("Streaming not yet implemented"));
            }
            _ => panic!("Expected InvalidInput error"),
        }
    }

    #[test]
    fn test_validate_request_warnings() {
        use std::collections::HashMap;

        use crate::responses::models::{ReasoningSettings, ResponseRequest};

        let req = ResponseRequest {
            model: "test_model".to_string(),
            input: "test input".to_string(),
            instructions: None,
            previous_response_id: None,
            modalities: Some(vec!["text".to_string(), "audio".to_string()]),
            response_format: None,
            reasoning: Some(ReasoningSettings {
                effort: "medium".to_string(),
                extra: HashMap::new(),
            }),
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            stream: None,
            include: None,
            tools: None,
            tool_choice: None,
            tool_resources: None,
            attachments: None,
            metadata: None,
            user: None,
        };

        let result = validate_request(&req);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.contains("modalities")));
        assert!(warnings.iter().any(|w| w.contains("Reasoning")));
    }
}
