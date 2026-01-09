//! HTTP handlers for Skills management API
//!
//! Provides RESTful endpoints for managing skills:
//! - List all skills
//! - Get skill details
//! - Enable/disable skills
//! - Reload skills

use axum::{
    Json,
    body::Body,
    extract::Path,
    http::{HeaderMap, Response, StatusCode},
};
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};

use super::{SkillRegistry, SkillSummary, error::SkillError};
use crate::{dual_error, dual_info, dual_warn, error::ServerResult};

/// Response for listing skills
#[derive(Debug, Serialize)]
pub struct SkillListResponse {
    /// List of skill summaries
    pub skills: Vec<SkillSummary>,
    /// Total count
    pub total: usize,
}

/// Response for skill details
#[derive(Debug, Serialize)]
pub struct SkillDetailResponse {
    /// Skill name
    pub name: String,
    /// Skill description
    pub description: String,
    /// Whether the skill is enabled
    pub enabled: bool,
    /// License information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Allowed tools
    pub allowed_tools: Vec<String>,
    /// Allowed scripts
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_scripts: Option<Vec<String>>,
    /// Available scripts in the skill
    pub scripts: Vec<String>,
    /// Skill content (markdown)
    pub content: String,
}

/// Request for enabling/disabling a skill
#[derive(Debug, Deserialize)]
pub struct EnableSkillRequest {
    pub enabled: bool,
}

/// Response for enable/disable and reload operations
#[derive(Debug, Serialize)]
pub struct SkillOperationResponse {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
}

/// Response for reload all operation
#[derive(Debug, Serialize)]
pub struct ReloadAllResponse {
    pub success: bool,
    pub message: String,
    pub skills_loaded: usize,
}

/// Helper to create an error response
fn error_response(status: StatusCode, message: &str) -> ServerResult<Response<Body>> {
    let body = serde_json::json!({ "error": message });
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .map_err(|e| {
            let err_msg = format!("Failed to create error response: {e}");
            crate::error::ServerError::Operation(err_msg)
        })
}

/// Helper to get registry or return error
fn get_registry() -> Result<&'static SkillRegistry, SkillError> {
    SkillRegistry::global()
}

/// GET /api/skills - List all skills
///
/// Returns a list of all loaded skills with their summaries.
pub async fn list_skills_handler(headers: HeaderMap) -> ServerResult<Response<Body>> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!("Listing all skills - request_id: {}", request_id);

    let registry = match get_registry() {
        Ok(r) => r,
        Err(_) => {
            dual_warn!(
                "Skills registry not initialized - request_id: {}",
                request_id
            );
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Skills system is not enabled",
            );
        }
    };

    let summaries = registry.get_summaries().await;
    let total = summaries.len();

    dual_info!("Found {} skills - request_id: {}", total, request_id);

    let response = SkillListResponse {
        skills: summaries,
        total,
    };

    let json_body = serde_json::to_string(&response).map_err(|e| {
        let err_msg = format!("Failed to serialize skills: {e}");
        dual_error!("{err_msg} - request_id: {request_id}");
        crate::error::ServerError::Operation(err_msg)
    })?;

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(json_body))
        .map_err(|e| {
            let err_msg = format!("Failed to create response: {e}");
            dual_error!("{err_msg} - request_id: {request_id}");
            crate::error::ServerError::Operation(err_msg)
        })
}

/// GET /api/skills/{name} - Get skill details
///
/// Returns detailed information about a specific skill.
pub async fn get_skill_handler(
    Path(name): Path<String>,
    headers: HeaderMap,
) -> ServerResult<Response<Body>> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!(
        "Getting skill details for '{}' - request_id: {}",
        name,
        request_id
    );

    let registry = match get_registry() {
        Ok(r) => r,
        Err(_) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Skills system is not enabled",
            );
        }
    };

    let skill = match registry.get(&name).await {
        Some(s) => s,
        None => {
            dual_warn!("Skill '{}' not found - request_id: {}", name, request_id);
            return error_response(
                StatusCode::NOT_FOUND,
                &format!("Skill '{}' not found", name),
            );
        }
    };

    let response = SkillDetailResponse {
        name: skill.metadata.name.clone(),
        description: skill.metadata.description.clone(),
        enabled: skill.enabled,
        license: skill.metadata.license.clone(),
        allowed_tools: skill.metadata.get_allowed_tools(),
        allowed_scripts: skill.metadata.allowed_scripts.clone(),
        scripts: skill.scripts.iter().map(|s| s.name.clone()).collect(),
        content: skill.content.clone(),
    };

    let json_body = serde_json::to_string(&response).map_err(|e| {
        let err_msg = format!("Failed to serialize skill: {e}");
        dual_error!("{err_msg} - request_id: {request_id}");
        crate::error::ServerError::Operation(err_msg)
    })?;

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(json_body))
        .map_err(|e| {
            let err_msg = format!("Failed to create response: {e}");
            dual_error!("{err_msg} - request_id: {request_id}");
            crate::error::ServerError::Operation(err_msg)
        })
}

/// PUT /api/skills/{name}/enabled - Enable or disable a skill
///
/// Toggles the enabled state of a skill.
pub async fn set_skill_enabled_handler(
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(request): Json<EnableSkillRequest>,
) -> ServerResult<Response<Body>> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let action = if request.enabled { "enable" } else { "disable" };
    dual_info!(
        "Setting skill '{}' to {} - request_id: {}",
        name,
        action,
        request_id
    );

    let registry = match get_registry() {
        Ok(r) => r,
        Err(_) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Skills system is not enabled",
            );
        }
    };

    match registry.set_enabled(&name, request.enabled).await {
        Ok(()) => {
            dual_info!(
                "Skill '{}' {}d successfully - request_id: {}",
                name,
                action,
                request_id
            );

            let response = SkillOperationResponse {
                success: true,
                message: format!("Skill '{}' {}d successfully", name, action),
                skill_name: Some(name),
            };

            let json_body = serde_json::to_string(&response).map_err(|e| {
                crate::error::ServerError::Operation(format!("Failed to serialize response: {e}"))
            })?;

            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json_body))
                .map_err(|e| {
                    crate::error::ServerError::Operation(format!("Failed to create response: {e}"))
                })
        }
        Err(SkillError::NotFound(_)) => {
            dual_warn!("Skill '{}' not found - request_id: {}", name, request_id);
            error_response(
                StatusCode::NOT_FOUND,
                &format!("Skill '{}' not found", name),
            )
        }
        Err(e) => {
            dual_error!(
                "Failed to {} skill '{}': {} - request_id: {}",
                action,
                name,
                e,
                request_id
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to {} skill: {}", action, e),
            )
        }
    }
}

/// POST /api/skills/{name}/reload - Reload a specific skill
///
/// Reloads the skill from disk, picking up any changes to SKILL.md.
pub async fn reload_skill_handler(
    Path(name): Path<String>,
    headers: HeaderMap,
) -> ServerResult<Response<Body>> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!("Reloading skill '{}' - request_id: {}", name, request_id);

    let registry = match get_registry() {
        Ok(r) => r,
        Err(_) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Skills system is not enabled",
            );
        }
    };

    match registry.reload(&name).await {
        Ok(()) => {
            dual_info!(
                "Skill '{}' reloaded successfully - request_id: {}",
                name,
                request_id
            );

            let response = SkillOperationResponse {
                success: true,
                message: format!("Skill '{}' reloaded successfully", name),
                skill_name: Some(name),
            };

            let json_body = serde_json::to_string(&response).map_err(|e| {
                crate::error::ServerError::Operation(format!("Failed to serialize response: {e}"))
            })?;

            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json_body))
                .map_err(|e| {
                    crate::error::ServerError::Operation(format!("Failed to create response: {e}"))
                })
        }
        Err(SkillError::NotFound(_)) => {
            dual_warn!("Skill '{}' not found - request_id: {}", name, request_id);
            error_response(
                StatusCode::NOT_FOUND,
                &format!("Skill '{}' not found", name),
            )
        }
        Err(e) => {
            dual_error!(
                "Failed to reload skill '{}': {} - request_id: {}",
                name,
                e,
                request_id
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to reload skill: {}", e),
            )
        }
    }
}

/// POST /api/skills/reload - Reload all skills
///
/// Clears the skill cache and reloads all skills from the skills directory.
pub async fn reload_all_skills_handler(headers: HeaderMap) -> ServerResult<Response<Body>> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!("Reloading all skills - request_id: {}", request_id);

    let registry = match get_registry() {
        Ok(r) => r,
        Err(_) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Skills system is not enabled",
            );
        }
    };

    match registry.reload_all().await {
        Ok(count) => {
            dual_info!(
                "Reloaded {} skills successfully - request_id: {}",
                count,
                request_id
            );

            let response = ReloadAllResponse {
                success: true,
                message: format!("Reloaded {} skills successfully", count),
                skills_loaded: count,
            };

            let json_body = serde_json::to_string(&response).map_err(|e| {
                crate::error::ServerError::Operation(format!("Failed to serialize response: {e}"))
            })?;

            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json_body))
                .map_err(|e| {
                    crate::error::ServerError::Operation(format!("Failed to create response: {e}"))
                })
        }
        Err(e) => {
            dual_error!(
                "Failed to reload all skills: {} - request_id: {}",
                e,
                request_id
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to reload skills: {}", e),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_list_response_serialization() {
        let response = SkillListResponse {
            skills: vec![SkillSummary {
                name: "test-skill".to_string(),
                description: "A test skill".to_string(),
                allowed_tools: vec!["Bash".to_string()],
            }],
            total: 1,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("test-skill"));
        assert!(json.contains("\"total\":1"));
    }

    #[test]
    fn test_skill_detail_response_serialization() {
        let response = SkillDetailResponse {
            name: "my-skill".to_string(),
            description: "Description".to_string(),
            enabled: true,
            license: Some("MIT".to_string()),
            allowed_tools: vec!["Read".to_string()],
            allowed_scripts: Some(vec!["*.js".to_string()]),
            scripts: vec!["process.js".to_string()],
            content: "# My Skill".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("my-skill"));
        assert!(json.contains("MIT"));
        assert!(json.contains("*.js"));
    }

    #[test]
    fn test_skill_detail_response_without_optional_fields() {
        let response = SkillDetailResponse {
            name: "minimal".to_string(),
            description: "Minimal skill".to_string(),
            enabled: true,
            license: None,
            allowed_tools: vec![],
            allowed_scripts: None,
            scripts: vec![],
            content: "".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        // Optional fields should be omitted
        assert!(!json.contains("license"));
        assert!(!json.contains("allowed_scripts"));
    }

    #[test]
    fn test_enable_skill_request_deserialization() {
        let json = r#"{"enabled": true}"#;
        let request: EnableSkillRequest = serde_json::from_str(json).unwrap();
        assert!(request.enabled);

        let json = r#"{"enabled": false}"#;
        let request: EnableSkillRequest = serde_json::from_str(json).unwrap();
        assert!(!request.enabled);
    }

    #[test]
    fn test_skill_operation_response_serialization() {
        let response = SkillOperationResponse {
            success: true,
            message: "Operation completed".to_string(),
            skill_name: Some("test".to_string()),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("test"));
    }

    #[test]
    fn test_skill_operation_response_without_skill_name() {
        let response = SkillOperationResponse {
            success: false,
            message: "Error occurred".to_string(),
            skill_name: None,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("skill_name"));
    }

    #[test]
    fn test_reload_all_response_serialization() {
        let response = ReloadAllResponse {
            success: true,
            message: "Reloaded 5 skills".to_string(),
            skills_loaded: 5,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"skills_loaded\":5"));
    }
}
