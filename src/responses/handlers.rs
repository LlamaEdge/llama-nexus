use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::Json};
use tokio::sync::OnceCell;

use crate::{
    AppState as MainAppState,
    responses::{
        db::{Database, DatabaseError},
        models::{
            Input, InputItem, InputMessageContent, ResponseItemInputMessageContent,
            ResponseOutputItem, ResponseOutputItemOutputMessageContent, ResponseReply,
            ResponseRequest, Session, ToolChoice,
        },
    },
    server::RoutingPolicy,
};

#[derive(Debug)]
enum ResponseError {
    InvalidInput(String),
    SessionNotFound(String),
    DatabaseError(String),
    BackendError(String),
}

impl ResponseError {
    fn to_http_error(&self) -> (StatusCode, String) {
        match self {
            Self::InvalidInput(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Self::SessionNotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Self::DatabaseError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            Self::BackendError(msg) => (StatusCode::BAD_GATEWAY, msg.clone()),
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
    let mut warnings = validate_request(&req)?;

    let db = state.get_or_create_db().await?;

    let existing_session = if let Some(prev_id) = req.upstream().previous_response_id.as_ref() {
        match db.find_session_by_response_id(prev_id).await? {
            Some(session) => Some(session),
            None => {
                return Err(ResponseError::SessionNotFound(format!(
                    "Previous response ID not found: {prev_id}"
                )));
            }
        }
    } else {
        None
    };

    let user_text = extract_user_text(&req);

    let mut response = call_responses_backend(&state.main_state, &req).await?;

    let response_id = response.id.clone();
    let model_used = response.model.clone();

    let mut session = if let Some(mut session) = existing_session {
        session.model_used = model_used.clone();
        session
    } else {
        Session::new(
            response_id.clone(),
            model_used.clone(),
            req.upstream().instructions.clone(),
        )
    };

    if let Some(text) = user_text.filter(|value| !value.trim().is_empty()) {
        let user_tokens = estimate_tokens(&text);
        session.add_message("user".to_string(), text, user_tokens, None, None);
    }

    if let Some(assistant_text) = extract_assistant_text(&response) {
        session.add_message(
            "assistant".to_string(),
            assistant_text,
            response.usage.output_tokens,
            None,
            Some(response_id.clone()),
        );
    }

    update_session_extended_data(&mut session, &req);

    db.save_session(&session).await?;

    apply_warnings(&mut response, &mut warnings);

    Ok(Json(response))
}

fn update_session_extended_data(session: &mut Session, req: &ResponseRequest) {
    let inner = req.upstream();

    let mut extended = session
        .extended_data
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();

    if let Some(metadata) = &inner.metadata {
        extended.insert(
            "metadata".to_string(),
            serde_json::to_value(metadata).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(include) = &inner.include {
        extended.insert(
            "include".to_string(),
            serde_json::to_value(include).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(conversation) = &inner.conversation {
        extended.insert(
            "conversation".to_string(),
            serde_json::to_value(conversation).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(tools) = &inner.tools {
        extended.insert(
            "tools".to_string(),
            serde_json::to_value(tools).unwrap_or(serde_json::Value::Null),
        );
    }
    if !matches!(inner.tool_choice, ToolChoice::None) {
        extended.insert(
            "tool_choice".to_string(),
            serde_json::to_value(&inner.tool_choice).unwrap_or(serde_json::Value::Null),
        );
    }
    if inner.max_tool_calls.is_some() {
        extended.insert(
            "max_tool_calls".to_string(),
            serde_json::to_value(inner.max_tool_calls).unwrap_or(serde_json::Value::Null),
        );
    }
    if inner.parallel_tool_calls.is_some() {
        extended.insert(
            "parallel_tool_calls".to_string(),
            serde_json::to_value(inner.parallel_tool_calls).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(modalities) = &req.modalities {
        extended.insert(
            "modalities".to_string(),
            serde_json::to_value(modalities).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(response_format) = &req.response_format {
        extended.insert(
            "response_format".to_string(),
            serde_json::to_value(response_format).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(reasoning) = &req.reasoning {
        extended.insert(
            "reasoning".to_string(),
            serde_json::to_value(reasoning).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(tool_resources) = &req.tool_resources {
        extended.insert(
            "tool_resources".to_string(),
            serde_json::to_value(tool_resources).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(attachments) = &req.attachments {
        extended.insert(
            "attachments".to_string(),
            serde_json::to_value(attachments).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(user) = &req.user {
        extended.insert("user".to_string(), serde_json::Value::String(user.clone()));
    }

    session.extended_data = if extended.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(extended))
    };
}

fn extract_user_text(req: &ResponseRequest) -> Option<String> {
    let input = req.upstream().input.as_ref()?;

    match input {
        Input::Text(text) => {
            if text.trim().is_empty() {
                None
            } else {
                Some(text.clone())
            }
        }
        Input::InputItemList(items) => items.iter().rev().find_map(|item| {
            if let InputItem::InputMessage { content, role, .. } = item
                && role == "user"
            {
                match content {
                    InputMessageContent::Text(text) => {
                        if !text.trim().is_empty() {
                            return Some(text.clone());
                        }
                    }
                    InputMessageContent::InputItemContentList(parts) => {
                        let mut buffer = String::new();
                        for part in parts {
                            if let ResponseItemInputMessageContent::Text { text, .. } = part {
                                buffer.push_str(text);
                            }
                        }

                        if !buffer.trim().is_empty() {
                            return Some(buffer);
                        }
                    }
                }
            }
            None
        }),
    }
}

fn extract_assistant_text(response: &ResponseReply) -> Option<String> {
    for item in &response.output {
        if let ResponseOutputItem::OutputMessage { content, .. } = item {
            let mut buffer = String::new();
            for element in content {
                match element {
                    ResponseOutputItemOutputMessageContent::OutputText { text, .. } => {
                        buffer.push_str(text)
                    }
                    ResponseOutputItemOutputMessageContent::Refusal { refusal, .. } => {
                        buffer.push_str(refusal)
                    }
                }
            }

            if !buffer.is_empty() {
                return Some(buffer);
            }
        }
    }

    None
}

fn estimate_tokens(text: &str) -> u64 {
    (text.len() as f32 / 4.0).ceil() as u64
}

fn apply_warnings(response: &mut ResponseReply, warnings: &mut Vec<String>) {
    if warnings.is_empty() {
        return;
    }

    let merged = std::mem::take(warnings).join(" | ");
    let key = "llama_nexus_warnings".to_string();

    if let Some(existing) = response.metadata.get_mut(&key) {
        if !existing.is_empty() {
            existing.push_str(" | ");
        }
        existing.push_str(&merged);
    } else {
        response.metadata.insert(key, merged);
    }
}

fn validate_request(req: &ResponseRequest) -> Result<Vec<String>, ResponseError> {
    let mut warnings = Vec::new();
    let inner = req.upstream();

    let model = inner
        .model
        .as_ref()
        .ok_or_else(|| ResponseError::InvalidInput("Model name cannot be empty".to_string()))?;
    if model.trim().is_empty() {
        return Err(ResponseError::InvalidInput(
            "Model name cannot be empty".to_string(),
        ));
    }

    match extract_user_text(req) {
        Some(input) if !input.trim().is_empty() => {}
        _ => {
            return Err(ResponseError::InvalidInput(
                "Input message cannot be empty".to_string(),
            ));
        }
    }

    if inner
        .temperature
        .is_some_and(|temp| !(0.0..=2.0).contains(&temp))
    {
        return Err(ResponseError::InvalidInput(
            "Temperature must be between 0.0 and 2.0".to_string(),
        ));
    }

    if inner
        .top_p
        .is_some_and(|top_p| !(0.0..=1.0).contains(&top_p))
    {
        return Err(ResponseError::InvalidInput(
            "top_p must be between 0.0 and 1.0".to_string(),
        ));
    }

    if inner
        .max_output_tokens
        .is_some_and(|max_tokens| max_tokens <= 0)
    {
        return Err(ResponseError::InvalidInput(
            "max_output_tokens must be positive".to_string(),
        ));
    }

    if inner.stream == Some(true) {
        return Err(ResponseError::InvalidInput(
            "Streaming not yet implemented".to_string(),
        ));
    }

    if inner
        .include
        .as_ref()
        .is_some_and(|items| !items.is_empty())
    {
        warnings.push(
            "`include` ignored: llama-api-server currently returns core text fields only"
                .to_string(),
        );
    }

    if inner.tools.as_ref().is_some_and(|tools| !tools.is_empty()) {
        warnings
            .push("`tools` ignored: tool calling is not enabled for text responses".to_string());
    }

    if !matches!(inner.tool_choice, ToolChoice::None) {
        warnings.push(
            "`tool_choice` ignored: tool calling is not enabled for text responses".to_string(),
        );
    }

    if inner.max_tool_calls.is_some() {
        warnings.push(
            "`max_tool_calls` ignored: tool calling is not enabled for text responses".to_string(),
        );
    }

    if inner.parallel_tool_calls.is_some() {
        warnings.push(
            "`parallel_tool_calls` ignored: tool calling is not enabled for text responses"
                .to_string(),
        );
    }

    if inner.background == Some(true) {
        warnings.push("`background` ignored: asynchronous responses are not supported".to_string());
    }

    if inner.conversation.is_some() {
        warnings.push(
            "`conversation` object ignored: llama-nexus manages sessions locally".to_string(),
        );
    }

    if req
        .modalities
        .as_ref()
        .is_some_and(|items| !items.is_empty())
    {
        warnings.push(
            "`modalities` ignored: llama-api-server currently supports text responses only"
                .to_string(),
        );
    }

    if req.response_format.is_some() {
        warnings
            .push("`response_format` ignored: structured outputs not yet supported".to_string());
    }

    if req.reasoning.is_some() {
        warnings.push("`reasoning` ignored: reasoning traces not yet supported".to_string());
    }

    if req.tool_resources.is_some() {
        warnings.push(
            "`tool_resources` ignored: tool calling is not enabled for text responses".to_string(),
        );
    }

    if req
        .attachments
        .as_ref()
        .is_some_and(|items| !items.is_empty())
    {
        warnings.push(
            "`attachments` ignored: llama-api-server currently supports text inputs only"
                .to_string(),
        );
    }

    if req.user.is_some() {
        warnings
            .push("`user` ignored: responses backend does not accept user identifiers".to_string());
    }

    Ok(warnings)
}

async fn call_responses_backend(
    main_state: &Arc<MainAppState>,
    request: &ResponseRequest,
) -> Result<ResponseReply, ResponseError> {
    let servers = main_state.server_group.read().await;
    let chat_servers = servers
        .get(&crate::server::ServerKind::chat)
        .ok_or_else(|| ResponseError::BackendError("No chat server available".to_string()))?;

    let target_server = chat_servers
        .next()
        .await
        .map_err(|e| ResponseError::BackendError(format!("Failed to get chat server: {e}")))?;

    let url = crate::handlers::build_api_url(&target_server.url, "responses");

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(request.upstream())
        .send()
        .await
        .map_err(|e| ResponseError::BackendError(format!("Request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(ResponseError::BackendError(format!(
            "Responses API error ({status}): {body}"
        )));
    }

    response
        .json::<ResponseReply>()
        .await
        .map_err(|e| ResponseError::BackendError(format!("Failed to parse response: {e}")))
}

pub async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "responses-api"
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::responses::models::{ReasoningSettings, ResponseFormat};

    fn base_request() -> ResponseRequest {
        ResponseRequest {
            inner: endpoints::responses::response_object::RequestOfModelResponse {
                background: None,
                conversation: None,
                include: None,
                input: Some(Input::Text("test input".to_string())),
                instructions: None,
                max_output_tokens: None,
                max_tool_calls: None,
                metadata: None,
                model: Some("test_model".to_string()),
                parallel_tool_calls: None,
                previous_response_id: None,
                safety_identifier: None,
                store: None,
                stream: None,
                temperature: None,
                tool_choice: ToolChoice::None,
                tools: None,
                top_p: None,
                truncation: None,
            },
            modalities: None,
            response_format: None,
            reasoning: None,
            tool_resources: None,
            attachments: None,
            user: None,
        }
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0_u64);
        assert_eq!(estimate_tokens("a"), 1_u64);
        assert_eq!(estimate_tokens("test"), 1_u64);
        assert_eq!(estimate_tokens("hello"), 2_u64);
        assert_eq!(estimate_tokens("This is a test message"), 6_u64);
        assert_eq!(estimate_tokens("Hello, world!"), 4_u64);
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

        let backend_error = ResponseError::BackendError("Backend unavailable".to_string());
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
        let mut req = base_request();
        req.inner.temperature = Some(1.5);
        let result = validate_request(&req);
        assert!(result.is_ok());

        req.inner.temperature = Some(2.5);
        let result = validate_request(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_request_top_p_range() {
        let mut req = base_request();
        req.inner.top_p = Some(0.9);
        let result = validate_request(&req);
        assert!(result.is_ok());

        req.inner.top_p = Some(1.1);
        assert!(validate_request(&req).is_err());

        req.inner.top_p = Some(-0.1);
        assert!(validate_request(&req).is_err());
    }

    #[test]
    fn test_validate_request_max_output_tokens() {
        let mut req = base_request();
        req.inner.max_output_tokens = Some(100);
        assert!(validate_request(&req).is_ok());

        req.inner.max_output_tokens = Some(0);
        assert!(validate_request(&req).is_err());
    }

    #[test]
    fn test_validate_request_streaming_not_implemented() {
        let mut req = base_request();
        req.inner.stream = Some(true);

        let result = validate_request(&req);
        match result {
            Err(ResponseError::InvalidInput(msg)) => {
                assert!(msg.contains("Streaming not yet implemented"));
            }
            _ => panic!("Expected InvalidInput error"),
        }
    }

    #[test]
    fn test_validate_request_warnings() {
        let mut req = base_request();
        req.inner.include = Some(vec!["message.output_text.logprobs".to_string()]);
        req.inner.tool_choice = ToolChoice::Auto;
        req.inner.parallel_tool_calls = Some(false);
        req.modalities = Some(vec!["text".to_string()]);
        req.response_format = Some(ResponseFormat::JsonObject);
        req.reasoning = Some(ReasoningSettings {
            effort: crate::responses::models::ReasoningEffort::Medium,
            extra: HashMap::new(),
        });
        req.user = Some("demo-user".to_string());

        let warnings = validate_request(&req).unwrap();
        assert!(warnings.iter().any(|w| w.contains("include")));
        assert!(warnings.iter().any(|w| w.contains("tool calling")));
        assert!(warnings.iter().any(|w| w.contains("modalities")));
        assert!(warnings.iter().any(|w| w.contains("response_format")));
        assert!(warnings.iter().any(|w| w.contains("reasoning")));
        assert!(warnings.iter().any(|w| w.contains("user")));
    }
}
