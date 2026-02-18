//! HTTP client for the AI Barometer API.
//!
//! Provides a thin wrapper around `reqwest::blocking::Client` for interacting
//! with key management and auth endpoints. All methods return `anyhow::Result`
//! and translate HTTP errors into user-friendly messages per FR-8.

// This module is a foundation for future auth/keys command specs. The public API
// will be consumed once those command handlers are added. Suppress dead_code until then.
#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Endpoint path constants
// ---------------------------------------------------------------------------

const KEYS_PUBLIC_PATH: &str = "/api/keys/public";
const AUTH_PATH: &str = "/api/auth";
const AUTH_EXCHANGE_PATH: &str = "/api/auth/exchange";

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

/// Request body for `POST /api/auth/exchange`.
#[derive(Serialize)]
struct ExchangeCodeRequest<'a> {
    code: &'a str,
}

// ---------------------------------------------------------------------------
// Response DTOs
// ---------------------------------------------------------------------------

/// Response from `GET /api/keys/public`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ApiPublicKey {
    pub fingerprint: String,
    pub armored_public_key: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub rotated_at: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

/// Response from `POST /api/auth/exchange`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ExchangeCodeResponse {
    pub token: String,
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// Standard API response envelope used by current backend endpoints.
#[derive(Debug, Deserialize)]
struct ApiResponseEnvelope<T> {
    data: T,
}

// ---------------------------------------------------------------------------
// ApiClient
// ---------------------------------------------------------------------------

/// HTTP client for the AI Barometer API.
///
/// Wraps `reqwest::blocking::Client` with a normalized base URL and optional
/// Bearer token. All auth-required endpoints verify the token is present before
/// sending; `exchange_code` is the sole unauthenticated endpoint.
pub struct ApiClient {
    client: reqwest::blocking::Client,
    base_url: String,
    token: Option<String>,
}

impl ApiClient {
    /// Create a new API client.
    ///
    /// `base_url` is trimmed and stripped of a trailing slash to prevent
    /// double-slash issues when joining endpoint paths. `token` is optional —
    /// only `exchange_code` can be called without one.
    pub fn new(base_url: &str, token: Option<String>) -> Self {
        let normalized = base_url.trim().trim_end_matches('/').to_string();
        Self {
            client: reqwest::blocking::Client::new(),
            base_url: normalized,
            token,
        }
    }

    // -----------------------------------------------------------------------
    // Public endpoint methods
    // -----------------------------------------------------------------------

    /// Fetch the current API public key.
    pub fn get_api_public_key(&self) -> Result<ApiPublicKey> {
        let url = self.url(KEYS_PUBLIC_PATH);
        let resp = self
            .client
            .get(&url)
            .send()
            .with_context(|| format!("failed to connect to API at {url}"))?;

        let body = map_http_error(resp)?;
        let parsed: ApiPublicKey =
            parse_response_payload(&body, "failed to parse api public key response")?;
        Ok(parsed)
    }

    /// Revoke the current authentication token.
    pub fn revoke_token(&self) -> Result<()> {
        let url = self.url(AUTH_PATH);
        let resp = self
            .auth_request(reqwest::Method::DELETE, &url)?
            .send()
            .with_context(|| format!("failed to connect to API at {url}"))?;

        let status = resp.status();
        if status == reqwest::StatusCode::NO_CONTENT || status.is_success() {
            return Ok(());
        }

        // Non-success — fall through to error mapping
        map_http_error(resp)?;
        Ok(())
    }

    /// Exchange an OAuth authorization code for an API token.
    ///
    /// This is the only endpoint that does **not** require a Bearer token.
    pub fn exchange_code(&self, code: &str) -> Result<ExchangeCodeResponse> {
        let url = self.url(AUTH_EXCHANGE_PATH);
        let payload = ExchangeCodeRequest { code };
        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .with_context(|| format!("failed to connect to API at {url}"))?;

        let body = map_http_error(resp)?;
        let parsed: ExchangeCodeResponse =
            parse_response_payload(&body, "failed to parse auth exchange response")?;
        Ok(parsed)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Build a full URL by joining the base URL with an endpoint path.
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Build an authenticated request. Returns an error if no token is set.
    fn auth_request(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> Result<reqwest::blocking::RequestBuilder> {
        let token = self
            .token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run `cadence auth login` first."))?;

        Ok(self
            .client
            .request(method, url)
            .header("Authorization", format!("Bearer {token}")))
    }
}

// ---------------------------------------------------------------------------
// HTTP error mapping (FR-8)
// ---------------------------------------------------------------------------

/// Read a response body and return it as a string, or map non-success status
/// codes to user-friendly error messages.
fn map_http_error(resp: reqwest::blocking::Response) -> Result<String> {
    let status = resp.status();
    if status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Ok(body);
    }

    let body = resp.text().unwrap_or_default();

    match status.as_u16() {
        401 => anyhow::bail!("Not authenticated. Run `cadence auth login` to sign in."),
        400 => {
            let detail = extract_error_message(&body);
            anyhow::bail!("Bad request: {detail}");
        }
        404 => {
            let detail = extract_error_message(&body);
            anyhow::bail!("Not found: {detail}");
        }
        500..=599 => {
            let detail = extract_error_message(&body);
            anyhow::bail!("Server error: {detail}");
        }
        _ => {
            anyhow::bail!("Unexpected response (HTTP {status}): {body}");
        }
    }
}

/// Parse either an enveloped API response (`{"data": ...}`) or a legacy
/// direct payload (`{...}`) for backward compatibility.
fn parse_response_payload<T>(body: &str, context: &'static str) -> Result<T>
where
    T: DeserializeOwned,
{
    if let Ok(enveloped) = serde_json::from_str::<ApiResponseEnvelope<T>>(body) {
        return Ok(enveloped.data);
    }

    serde_json::from_str::<T>(body).context(context)
}

/// Try to extract a `message` or `error` field from a JSON error body.
/// Falls back to the raw body (truncated) if parsing fails.
fn extract_error_message(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(msg) = value.get("message").or(value.get("error"))
        && let Some(s) = msg.as_str()
    {
        return s.to_string();
    }

    if body.is_empty() {
        return "no details provided".to_string();
    }

    // Truncate large error bodies to prevent noisy output
    let trimmed = body.trim();
    if trimmed.len() > 200 {
        format!("{}...", &trimmed[..200])
    } else {
        trimmed.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_joins_paths() {
        let client = ApiClient::new("https://api.example.com/", None);
        assert_eq!(
            client.url("/api/keys/public"),
            "https://api.example.com/api/keys/public"
        );
    }
}
