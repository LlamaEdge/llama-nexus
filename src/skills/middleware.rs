//! Middleware for Skills API authentication and rate limiting
//!
//! Provides:
//! - API key authentication (optional)
//! - Sliding window rate limiting

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, Response, StatusCode},
    middleware::Next,
};
use reqwest::header::CONTENT_TYPE;
use tokio::sync::RwLock;

use crate::{config::SkillApiConfig, dual_info, dual_warn};

/// Global rate limiter state
pub static RATE_LIMITER: once_cell::sync::OnceCell<Arc<RateLimiter>> =
    once_cell::sync::OnceCell::new();

/// Rate limiter using sliding window algorithm
pub struct RateLimiter {
    /// Request counts per client (keyed by IP or API key)
    requests: RwLock<HashMap<String, Vec<Instant>>>,
    /// Maximum requests allowed in the window
    max_requests: u32,
    /// Time window duration
    window: Duration,
}

impl RateLimiter {
    /// Create a new rate limiter
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            requests: RwLock::new(HashMap::new()),
            max_requests,
            window: Duration::from_secs(window_secs),
        }
    }

    /// Check if a request is allowed and record it if so
    ///
    /// Returns `true` if the request is allowed, `false` if rate limited.
    pub async fn check_and_record(&self, client_id: &str) -> bool {
        let now = Instant::now();
        let cutoff = now - self.window;

        let mut requests = self.requests.write().await;
        let timestamps = requests.entry(client_id.to_string()).or_default();

        // Remove expired timestamps
        timestamps.retain(|t| *t > cutoff);

        // Check if under limit
        if timestamps.len() < self.max_requests as usize {
            timestamps.push(now);
            true
        } else {
            false
        }
    }

    /// Get the remaining requests for a client
    #[allow(dead_code)]
    pub async fn remaining(&self, client_id: &str) -> u32 {
        let now = Instant::now();
        let cutoff = now - self.window;

        let requests = self.requests.read().await;
        if let Some(timestamps) = requests.get(client_id) {
            let valid_count = timestamps.iter().filter(|t| **t > cutoff).count();
            self.max_requests.saturating_sub(valid_count as u32)
        } else {
            self.max_requests
        }
    }

    /// Clean up expired entries (call periodically)
    pub async fn cleanup(&self) {
        let now = Instant::now();
        let cutoff = now - self.window;

        let mut requests = self.requests.write().await;
        requests.retain(|_, timestamps| {
            timestamps.retain(|t| *t > cutoff);
            !timestamps.is_empty()
        });
    }
}

/// Initialize the global rate limiter
pub fn init_rate_limiter(config: &SkillApiConfig) {
    if config.rate_limiting_enabled() {
        let limiter = Arc::new(RateLimiter::new(
            config.rate_limit_requests,
            config.rate_limit_window_secs,
        ));
        let _ = RATE_LIMITER.set(limiter.clone());

        // Spawn cleanup task
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                if let Some(limiter) = RATE_LIMITER.get() {
                    limiter.cleanup().await;
                }
            }
        });

        dual_info!(
            "Skills API rate limiting enabled: {} requests per {} seconds",
            config.rate_limit_requests,
            config.rate_limit_window_secs
        );
    }
}

/// Skills API state for middleware
#[derive(Clone)]
pub struct SkillsApiState {
    /// API key for authentication (None = no auth required)
    pub api_key: Option<String>,
    /// Whether rate limiting is enabled
    pub rate_limiting_enabled: bool,
}

impl SkillsApiState {
    pub fn from_config(config: Option<&SkillApiConfig>) -> Self {
        match config {
            Some(cfg) => Self {
                api_key: cfg.get_api_key(),
                rate_limiting_enabled: cfg.rate_limiting_enabled(),
            },
            None => Self {
                api_key: None,
                rate_limiting_enabled: false,
            },
        }
    }
}

/// Create an error response
fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    let body = serde_json::json!({ "error": message });
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Middleware for Skills API authentication and rate limiting
pub async fn skills_api_middleware(
    State(state): State<SkillsApiState>,
    headers: HeaderMap,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown");

    // 1. Authentication check
    if let Some(expected_key) = &state.api_key {
        let auth_header = headers.get("authorization").and_then(|h| h.to_str().ok());

        let provided_key = match auth_header {
            Some(h) if h.starts_with("Bearer ") => Some(h.trim_start_matches("Bearer ").trim()),
            Some(h) => Some(h.trim()),
            None => None,
        };

        match provided_key {
            Some(key) if key == expected_key => {
                // Authentication successful
            }
            Some(_) => {
                dual_warn!(
                    "Skills API authentication failed: invalid API key - request_id: {}",
                    request_id
                );
                return error_response(StatusCode::UNAUTHORIZED, "Invalid API key");
            }
            None => {
                dual_warn!(
                    "Skills API authentication failed: missing API key - request_id: {}",
                    request_id
                );
                return error_response(StatusCode::UNAUTHORIZED, "API key required");
            }
        }
    }

    // 2. Rate limiting check
    if state.rate_limiting_enabled
        && let Some(limiter) = RATE_LIMITER.get()
    {
        // Use client IP or API key as identifier
        let client_id = headers
            .get("x-forwarded-for")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
            .or_else(|| {
                headers
                    .get("x-real-ip")
                    .and_then(|h| h.to_str().ok())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "unknown".to_string());

        if !limiter.check_and_record(&client_id).await {
            dual_warn!(
                "Skills API rate limit exceeded for client {} - request_id: {}",
                client_id,
                request_id
            );
            return error_response(StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded");
        }
    }

    // Continue to handler
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_allows_requests_under_limit() {
        let limiter = RateLimiter::new(5, 60);

        for i in 0..5 {
            assert!(
                limiter.check_and_record("client1").await,
                "Request {} should be allowed",
                i
            );
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_blocks_requests_over_limit() {
        let limiter = RateLimiter::new(3, 60);

        // First 3 requests should be allowed
        for _ in 0..3 {
            assert!(limiter.check_and_record("client1").await);
        }

        // 4th request should be blocked
        assert!(!limiter.check_and_record("client1").await);
    }

    #[tokio::test]
    async fn test_rate_limiter_tracks_clients_separately() {
        let limiter = RateLimiter::new(2, 60);

        // Client 1 uses 2 requests
        assert!(limiter.check_and_record("client1").await);
        assert!(limiter.check_and_record("client1").await);
        assert!(!limiter.check_and_record("client1").await);

        // Client 2 should still have requests available
        assert!(limiter.check_and_record("client2").await);
        assert!(limiter.check_and_record("client2").await);
        assert!(!limiter.check_and_record("client2").await);
    }

    #[tokio::test]
    async fn test_rate_limiter_remaining() {
        let limiter = RateLimiter::new(5, 60);

        assert_eq!(limiter.remaining("client1").await, 5);

        limiter.check_and_record("client1").await;
        assert_eq!(limiter.remaining("client1").await, 4);

        limiter.check_and_record("client1").await;
        limiter.check_and_record("client1").await;
        assert_eq!(limiter.remaining("client1").await, 2);
    }

    #[tokio::test]
    async fn test_skills_api_state_from_config() {
        // No config
        let state = SkillsApiState::from_config(None);
        assert!(state.api_key.is_none());
        assert!(!state.rate_limiting_enabled);

        // With config
        let config = SkillApiConfig::default();
        let state = SkillsApiState::from_config(Some(&config));
        assert!(state.api_key.is_none()); // Default has no key
        assert!(state.rate_limiting_enabled); // Default enables rate limiting
    }

    #[tokio::test]
    async fn test_cleanup_removes_expired_entries() {
        let limiter = RateLimiter::new(100, 1); // 1 second window

        // Add some requests
        limiter.check_and_record("client1").await;
        limiter.check_and_record("client2").await;

        // Wait for window to expire
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Cleanup
        limiter.cleanup().await;

        // All entries should be removed
        let requests = limiter.requests.read().await;
        assert!(requests.is_empty());
    }
}
