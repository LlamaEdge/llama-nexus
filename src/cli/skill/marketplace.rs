//! Skills marketplace client for skillsmp.com
//!
//! Provides API integration for:
//! - Searching skills
//! - Getting skill details
//! - Downloading skill packages

use serde::{Deserialize, Serialize};

use crate::error::{ServerError, ServerResult};

/// Base URL for skillsmp.com API
const SKILLSMP_API_BASE: &str = "https://skillsmp.com/api/v1";

/// Default User-Agent for marketplace requests
const USER_AGENT: &str = "llama-nexus/0.8.2";

/// Skills marketplace client
pub struct SkillsMarketplace {
    client: reqwest::Client,
    api_key: Option<String>,
}

/// Skill information from the marketplace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceSkill {
    /// Skill ID (used for API calls)
    pub id: String,
    /// Skill name (display name)
    pub name: String,
    /// Skill description
    pub description: String,
    /// Version (if available)
    pub version: Option<String>,
    /// Author/owner
    pub author: Option<String>,
    /// License
    pub license: Option<String>,
    /// Allowed tools
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Download URL
    pub download_url: Option<String>,
}

/// Search result from the marketplace
#[derive(Debug, Deserialize)]
struct SearchResponse {
    skills: Vec<SkillSearchResult>,
    #[allow(dead_code)]
    total: Option<usize>,
}

/// Individual search result
#[derive(Debug, Deserialize)]
struct SkillSearchResult {
    id: String,
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

impl SkillsMarketplace {
    /// Create a new marketplace client
    pub fn new(api_key: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self { client, api_key }
    }

    /// Search for skills by query
    pub async fn search(&self, query: &str, limit: usize) -> ServerResult<Vec<MarketplaceSkill>> {
        let url = format!("{}/skills/ai-search", SKILLSMP_API_BASE);

        let mut request = self
            .client
            .get(&url)
            .query(&[("q", query), ("limit", &limit.to_string())]);

        if let Some(key) = &self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request
            .send()
            .await
            .map_err(|e| ServerError::Operation(format!("Failed to search skillsmp.com: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            // Check for Cloudflare blocking
            if status == reqwest::StatusCode::FORBIDDEN {
                return Err(ServerError::Operation(
                    "skillsmp.com requires an API key or is temporarily unavailable.\n\
                     Get an API key from https://skillsmp.com and set SKILLSMP_API_KEY environment variable,\n\
                     or try again later."
                        .to_string(),
                ));
            }
            let body = response.text().await.unwrap_or_default();
            return Err(ServerError::Operation(format!(
                "skillsmp.com API error ({}): {}",
                status, body
            )));
        }

        let search_response: SearchResponse = response.json().await.map_err(|e| {
            ServerError::Operation(format!("Failed to parse skillsmp.com response: {}", e))
        })?;

        // Convert search results to MarketplaceSkill
        let skills = search_response
            .skills
            .into_iter()
            .map(|r| MarketplaceSkill {
                id: r.id.clone(),
                name: r.name.unwrap_or_else(|| extract_name_from_id(&r.id)),
                description: r.description.unwrap_or_else(|| {
                    if r.tags.is_empty() {
                        "No description available".to_string()
                    } else {
                        r.tags.join(", ")
                    }
                }),
                version: None,
                author: None,
                license: None,
                allowed_tools: vec![],
                download_url: Some(format!("{}/skills/{}/download", SKILLSMP_API_BASE, r.id)),
            })
            .collect();

        Ok(skills)
    }

    /// List popular skills (default listing)
    pub async fn list_popular(&self, limit: usize) -> ServerResult<Vec<MarketplaceSkill>> {
        // Use a generic search query to get popular skills
        self.search("agent skill", limit).await
    }

    /// Get detailed information about a skill
    pub async fn get_skill_info(&self, skill_name: &str) -> ServerResult<MarketplaceSkill> {
        // First, search for the skill to get its ID
        let skills = self.search(skill_name, 5).await?;

        // Find an exact or close match
        let skill = skills
            .into_iter()
            .find(|s| {
                s.name.to_lowercase() == skill_name.to_lowercase()
                    || s.id.contains(skill_name)
                    || extract_name_from_id(&s.id).to_lowercase() == skill_name.to_lowercase()
            })
            .ok_or_else(|| {
                ServerError::Operation(format!("Skill '{}' not found on skillsmp.com", skill_name))
            })?;

        // TODO: Fetch full details from /skills/{id} endpoint when available
        Ok(skill)
    }

    /// Download a skill package
    pub async fn download_skill(&self, skill_id: &str) -> ServerResult<bytes::Bytes> {
        let url = format!("{}/skills/{}/download", SKILLSMP_API_BASE, skill_id);

        let mut request = self.client.get(&url);

        if let Some(key) = &self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request.send().await.map_err(|e| {
            ServerError::Operation(format!("Failed to download from skillsmp.com: {}", e))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            // Check for Cloudflare blocking
            if status == reqwest::StatusCode::FORBIDDEN {
                return Err(ServerError::Operation(
                    "skillsmp.com requires an API key for downloads.\n\
                     Get an API key from https://skillsmp.com and set SKILLSMP_API_KEY environment variable."
                        .to_string(),
                ));
            }
            let body = response.text().await.unwrap_or_default();
            return Err(ServerError::Operation(format!(
                "Failed to download skill ({}): {}",
                status, body
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| ServerError::Operation(format!("Failed to read skill package: {}", e)))
    }

    /// Resolve a skill name to its marketplace ID
    ///
    /// If a version is specified, it will be included in the resolution request.
    /// Note: Version support depends on skillsmp.com API capabilities.
    pub async fn resolve_skill_id(
        &self,
        skill_name: &str,
        version: Option<&str>,
    ) -> ServerResult<String> {
        // Include version in search query if specified
        let query = if let Some(ver) = version {
            format!("{} {}", skill_name, ver)
        } else {
            skill_name.to_string()
        };

        let skill = self.get_skill_info(&query).await?;

        // If version was requested, include it in the resolved ID for future use
        // This allows the download endpoint to potentially fetch a specific version
        if let Some(ver) = version {
            // Check if the skill has version info that matches
            if let Some(skill_ver) = &skill.version
                && skill_ver != ver
            {
                println!(
                    "  Note: Requested version '{}', but found version '{}'",
                    ver, skill_ver
                );
            }
        }

        Ok(skill.id)
    }
}

/// Extract a readable name from a skillsmp.com skill ID
///
/// ID format: `{owner}-{repo}-{path-to-skill-md}`
/// Example: `krmcbride-claude-plugins-essentials-skills-documentation-lookup-skill-md`
fn extract_name_from_id(id: &str) -> String {
    // Try to extract the skill name from the path
    // Usually the second-to-last segment is the skill name
    let parts: Vec<&str> = id.split('-').collect();

    if parts.len() >= 3 {
        // Look for "skill" or "skills" in the path to find the name
        for (i, part) in parts.iter().enumerate() {
            if *part == "skill" && i > 0 {
                // The previous part is likely the skill name
                return parts[i - 1].to_string();
            }
        }

        // Fallback: try to find meaningful segments
        // Skip owner and repo (usually first 2-3 parts)
        if parts.len() > 4 {
            // Join middle parts as the name
            let name_parts: Vec<&str> = parts[2..parts.len() - 2]
                .iter()
                .filter(|p| **p != "skills" && **p != "skill" && **p != "md")
                .copied()
                .collect();
            if !name_parts.is_empty() {
                return name_parts.join("-");
            }
        }
    }

    // Fallback: use the entire ID but truncate if too long
    if id.len() > 30 {
        format!("{}...", &id[..27])
    } else {
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_name_from_id_simple() {
        let id = "user-repo-code-review-skill-md";
        let name = extract_name_from_id(id);
        assert_eq!(name, "review");
    }

    #[test]
    fn test_extract_name_from_id_complex() {
        let id = "krmcbride-claude-plugins-essentials-skills-documentation-lookup-skill-md";
        let name = extract_name_from_id(id);
        // Should extract a meaningful name
        assert!(!name.is_empty());
        assert!(!name.contains("skill-md"));
    }

    #[test]
    fn test_extract_name_from_id_short() {
        let id = "short-id";
        let name = extract_name_from_id(id);
        assert_eq!(name, "short-id");
    }
}
