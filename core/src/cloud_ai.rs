//! The Cloud AI plugin (`data-model.md` §5.7 `CloudAiSecret`; `architecture.md` §8–§9;
//! `api.md` §5.6–§5.7; W27).
//!
//! Two things live here that `crate::session` composes:
//!
//! - **Storage.** `CloudAiSecret` is envelope-encrypted exactly like `crate::config::Config`
//!   (a fresh per-write DEK wraps the plaintext under AAD kind 5 `PluginSecret`, the master
//!   key wraps that DEK under kind 7 `WrappedDek`) — same two-layer shape, different kind.
//!   Unlike `Config`, absence is meaningful ("not configured", api.md §5.7), so there is no
//!   `Default` and `clear` is a real operation (cryptographic erase, architecture §4.3), not
//!   just "write the default back".
//! - **The network client.** `architecture.md` §9.2: "All Cloud AI HTTP runs in the Rust
//!   core... The webview has no HTTP capability." [`CloudAiClient`] is that HTTP boundary —
//!   loopback-style discipline borrowed from `crate::detector::ollama::OllamaClient` (no
//!   ambient proxy, redirects never followed — architecture §9.2 "Redirects that change host
//!   are refused": since a redirect response is `!status().is_success()`, refusing it is the
//!   same "treat as refused, never follow" shape §7.4 already established for the Ollama
//!   client's `set_redirect` case), but pointed at the **user-configured** host instead of a
//!   fixed loopback address, and speaking an OpenAI-Chat-Completions-compatible wire shape
//!   (architecture §9.1: "OpenAI-compatible base URL + model id") so a real allowlisted
//!   OpenAI-compatible endpoint — including Ollama's own cloud API — can sit behind it, not
//!   only the in-process mock this chunk's tests use.
//!
//! `crate::session::SessionManager::preview_share`/`commit_share` own the **identity**
//! guarantee (api.md §5.6: the POSTed document body is the exact `ai_payload_preview`
//! string) by putting that string in its own `"document"` JSON field rather than folding it
//! into the chat `messages[]` the model actually reads — the two carry the same bytes, but
//! keeping them separately addressable is what makes "identical to the preview" a field
//! comparison instead of a substring argument.

use std::time::Duration;

use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::crypto::{Aad, ArtifactKind, Dek, DEK_LEN};
use crate::keys::VaultMasterKey;

/// `format_version` bound into the AAD of both layers (architecture §3.1), mirroring
/// `crate::config::CONFIG_FORMAT_VERSION`'s role for kind 4.
pub const CLOUD_AI_FORMAT_VERSION: u32 = 1;

/// data-model §5.7 `CloudAiSecret`. `key_last4` is derived, not user input — see
/// [`key_last4`] — but is carried on the struct because `cloud_ai_get_config` (api.md §5.7)
/// returns it without ever returning `api_key` itself.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CloudAiSecret {
    pub endpoint_url: String,
    pub model: String,
    pub api_key: String,
    pub key_last4: String,
}

/// Last 4 ASCII bytes of `api_key`, or the whole key if shorter than 4 — data-model §5.7:
/// "not a secret; for get-config without returning the key." A 4-character prefix is not a
/// meaningful credential fragment either way; this exists so the UI can show "…a1b2" as a
/// configured/changed indicator, not as a security boundary.
#[must_use]
pub fn key_last4(api_key: &str) -> String {
    let len = api_key.len();
    let start = len.saturating_sub(4);
    // `api_key` is arbitrary bytes from the frontend in principle; only slice on a char
    // boundary so this never panics on a key containing multi-byte UTF-8 near the cut.
    let mut start = start;
    while start < len && !api_key.is_char_boundary(start) {
        start += 1;
    }
    api_key[start..].to_string()
}

/// Failure modes of the Cloud AI secret backend. Same discipline as [`crate::config::ConfigError`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloudAiError {
    Backend(&'static str),
}

impl core::fmt::Display for CloudAiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CloudAiError::Backend(class) => write!(f, "cloud ai backend failure: {class}"),
        }
    }
}

impl std::error::Error for CloudAiError {}

/// Where the `CloudAiSecret` artifact lives. `crate::vault::SqlCipherVault` implements this
/// over the `artifact` table's unique `kind=5` row (`uq_artifact_cloud_ai`, W3's schema) plus
/// the `plugin_secret` table it references.
pub trait CloudAiStore: Send + Sync {
    /// `None` means "not configured" (api.md §5.7 `configured: false`) — unlike
    /// [`crate::config::ConfigStore::load`], absence here is a real, meaningful state, not a
    /// row that merely hasn't been written yet.
    ///
    /// # Errors
    /// [`CloudAiError::Backend`] on any I/O/backend/decrypt failure.
    fn load(&self, master: &VaultMasterKey) -> Result<Option<CloudAiSecret>, CloudAiError>;

    /// Replace the secret. Always the whole object, like `ConfigStore::store`.
    ///
    /// # Errors
    /// [`CloudAiError::Backend`] on any I/O/backend/encrypt failure.
    fn store(&self, master: &VaultMasterKey, secret: &CloudAiSecret) -> Result<(), CloudAiError>;

    /// Cryptographic erase (architecture §4.3): drop the row (and with it, the wrapped DEK)
    /// so the ciphertext becomes permanently unrecoverable. Idempotent — clearing an already
    /// unconfigured secret is not an error (api.md §5.7 `cloud_ai_clear_config` has no
    /// "not configured" failure mode).
    ///
    /// # Errors
    /// [`CloudAiError::Backend`] on any I/O/backend failure.
    fn clear(&self) -> Result<(), CloudAiError>;
}

/// The pre-W27 no-op backend, matching `crate::config::NullConfigStore`'s role: `load`
/// reports "not configured", `store`/`clear` error. Exists so every constructor that
/// predates W27 keeps working unmodified.
#[derive(Debug, Default)]
pub struct NullCloudAiStore;

impl CloudAiStore for NullCloudAiStore {
    fn load(&self, _master: &VaultMasterKey) -> Result<Option<CloudAiSecret>, CloudAiError> {
        Ok(None)
    }
    fn store(&self, _master: &VaultMasterKey, _secret: &CloudAiSecret) -> Result<(), CloudAiError> {
        Err(CloudAiError::Backend("no cloud ai store configured"))
    }
    fn clear(&self) -> Result<(), CloudAiError> {
        Err(CloudAiError::Backend("no cloud ai store configured"))
    }
}

/// AAD for the secret's plaintext layer (architecture §3.1, kind 5, not document-scoped).
#[must_use]
pub fn cloud_ai_plaintext_aad() -> Aad {
    Aad::global(ArtifactKind::PluginSecret, CLOUD_AI_FORMAT_VERSION)
}

/// AAD for the secret's DEK-wrap layer — mirrors `crate::config::config_dek_wrap_aad`.
#[must_use]
pub fn cloud_ai_dek_wrap_aad() -> Aad {
    Aad::global(ArtifactKind::WrappedDek, CLOUD_AI_FORMAT_VERSION)
}

/// Encrypt `secret` under a fresh DEK, then wrap that DEK under `master`. Same shape as
/// `crate::config::seal_config`.
///
/// # Errors
/// Whatever the underlying AEAD wrap calls return.
pub fn seal_cloud_ai_secret(
    master: &VaultMasterKey,
    secret: &CloudAiSecret,
) -> Result<(crate::crypto::WrappedBlob, crate::crypto::WrappedBlob), CloudAiError> {
    let dek = Dek::generate();
    let plaintext =
        serde_json::to_vec(secret).map_err(|_| CloudAiError::Backend("serialize failed"))?;
    let artifact_blob = crate::crypto::wrap(dek.as_bytes(), &plaintext, &cloud_ai_plaintext_aad())
        .map_err(|_| CloudAiError::Backend("artifact wrap failed"))?;
    let wrapped_dek = crate::crypto::wrap(master.as_bytes(), dek.as_bytes(), &cloud_ai_dek_wrap_aad())
        .map_err(|_| CloudAiError::Backend("dek wrap failed"))?;
    Ok((wrapped_dek, artifact_blob))
}

/// The inverse of [`seal_cloud_ai_secret`]. Same discipline as `crate::config::open_config`:
/// any authentication or parse failure is an error, never a silently wrong secret.
///
/// # Errors
/// [`CloudAiError::Backend`] if either AEAD layer fails to authenticate, or the plaintext is
/// not valid `CloudAiSecret` JSON.
pub fn open_cloud_ai_secret(
    master: &VaultMasterKey,
    wrapped_dek: &crate::crypto::WrappedBlob,
    artifact_blob: &crate::crypto::WrappedBlob,
) -> Result<CloudAiSecret, CloudAiError> {
    let dek_bytes: Zeroizing<Vec<u8>> = Zeroizing::new(
        crate::crypto::unwrap(master.as_bytes(), wrapped_dek, &cloud_ai_dek_wrap_aad())
            .map_err(|_| CloudAiError::Backend("dek unwrap failed"))?,
    );
    let dek_array: [u8; DEK_LEN] = dek_bytes
        .as_slice()
        .try_into()
        .map_err(|_| CloudAiError::Backend("dek has the wrong length"))?;
    let dek = Dek::from_bytes(dek_array);

    let plaintext = crate::crypto::unwrap(dek.as_bytes(), artifact_blob, &cloud_ai_plaintext_aad())
        .map_err(|_| CloudAiError::Backend("artifact unwrap failed"))?;
    serde_json::from_slice(&plaintext).map_err(|_| CloudAiError::Backend("malformed cloud ai secret JSON"))
}

// ---------------------------------------------------------------------------
// `endpoint_url` validation (api.md §5.7)
// ---------------------------------------------------------------------------

/// api.md §5.7: "`endpoint_url` must be `https://` with a host; `file://`, `http://`, and
/// userinfo in the URL are `invalid_input`." Returns the host (with port, if any) on
/// success — the same string api.md calls `endpoint_host` on `cloud_ai_get_config` /
/// `cloud_ai_set_config` output and the audit `share` payload.
///
/// Deliberately not a general-purpose URL parser: this only recognizes the shape the
/// command accepts, so a URL this function rejects is rejected for a reason a caller can
/// name, not "some parser upstream didn't like it."
///
/// # Errors
/// A fixed, non-secret reason string (never the URL itself, which could carry query-string
/// PII in principle — C-API-1 discipline extended to config, not just documents).
pub fn validate_endpoint_url(url: &str) -> Result<String, &'static str> {
    let rest = url
        .strip_prefix("https://")
        .ok_or("endpoint_url must be https://")?;
    if rest.is_empty() {
        return Err("endpoint_url must have a host");
    }
    // Cut at the first `/`, `?`, or `#` to isolate the authority component.
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return Err("endpoint_url must have a host");
    }
    if authority.contains('@') {
        return Err("endpoint_url must not contain userinfo");
    }
    Ok(authority.to_string())
}

// ---------------------------------------------------------------------------
// The HTTP client (architecture §9.2)
// ---------------------------------------------------------------------------

const TEST_TIMEOUT: Duration = Duration::from_secs(10);
const SEND_TIMEOUT: Duration = Duration::from_secs(60);

/// First-party system preamble sent with every AI share (api.md §5.6: "a fixed system
/// preamble; the preamble is first-party, contains no vault secrets, and is not shown in
/// the preview").
pub const SYSTEM_PREAMBLE: &str =
    "You are assisting with a document that has already been redacted by its owner using \
     Privacy Gate. Follow the user's instruction about the document text provided. Do not \
     attempt to guess, infer, or reconstruct any information that may have been removed.";

/// Why a Cloud AI HTTP call failed (api.md §3: `cloud_ai_network` vs `cloud_ai_refused`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudAiCallError {
    /// TLS/connect/timeout failure, or a success-status response that did not parse as the
    /// expected shape — a protocol-level failure talking to the host, not a rejection of
    /// the request's content.
    Network,
    /// The endpoint responded with 4xx/5xx.
    Refused,
}

/// Result of a successful [`CloudAiClient::send`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudAiResponse {
    pub output_text: String,
}

/// The Rust-side-only HTTP boundary for Cloud AI (architecture §9.2: "the webview has no
/// HTTP capability"). No ambient proxy, redirects never followed — same discipline as
/// `crate::detector::ollama::OllamaClient`, aimed at a configured host instead of a fixed
/// loopback address.
pub struct CloudAiClient {
    http: reqwest::blocking::Client,
}

impl CloudAiClient {
    /// # Errors
    /// `"http client build failed"` if the underlying `reqwest` client cannot be built
    /// (never happens with this fixed configuration in practice; kept fallible rather than
    /// panicking so a future config change can't turn into an unwrap panic).
    pub fn new() -> Result<Self, &'static str> {
        let http = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "http client build failed")?;
        Ok(Self { http })
    }

    /// api.md §5.7 `cloud_ai_test`: "Sends **no** vault document content." No `"document"`
    /// field, no instruction — just enough of a request to prove the endpoint + key work.
    ///
    /// # Errors
    /// [`CloudAiCallError`] on connect/TLS failure or a non-2xx response.
    pub fn test(&self, secret: &CloudAiSecret) -> Result<(), CloudAiCallError> {
        let body = json!({
            "model": secret.model,
            "messages": [{ "role": "user", "content": "ping" }],
        });
        let resp = self
            .http
            .post(&secret.endpoint_url)
            .timeout(TEST_TIMEOUT)
            .bearer_auth(&secret.api_key)
            .json(&body)
            .send()
            .map_err(|_| CloudAiCallError::Network)?;
        if !resp.status().is_success() {
            return Err(CloudAiCallError::Refused);
        }
        Ok(())
    }

    /// Send `document` (== `ai_payload_preview`, api.md §5.6) plus `instruction` and the
    /// fixed [`SYSTEM_PREAMBLE`], and return the model's read-only text output.
    ///
    /// The wire body is OpenAI Chat-Completions-shaped (`model` + `messages[]`) so a real
    /// OpenAI-compatible endpoint can serve it, with `document` carried as its own top-level
    /// field — extra fields are ignored by every OpenAI-compatible server this was checked
    /// against — so the exact POSTed document text stays a single, directly comparable JSON
    /// value rather than a substring of the composed prompt (module docs above).
    ///
    /// # Errors
    /// [`CloudAiCallError::Network`] on connect/TLS/timeout failure or an unparseable
    /// success response; [`CloudAiCallError::Refused`] on a 4xx/5xx status (api.md §3).
    pub fn send(
        &self,
        secret: &CloudAiSecret,
        instruction: &str,
        document: &str,
    ) -> Result<CloudAiResponse, CloudAiCallError> {
        let user_content = format!("{instruction}\n\n{document}");
        let body = json!({
            "model": secret.model,
            "document": document,
            "messages": [
                { "role": "system", "content": SYSTEM_PREAMBLE },
                { "role": "user", "content": user_content },
            ],
        });
        let resp = self
            .http
            .post(&secret.endpoint_url)
            .timeout(SEND_TIMEOUT)
            .bearer_auth(&secret.api_key)
            .json(&body)
            .send()
            .map_err(|_| CloudAiCallError::Network)?;
        if !resp.status().is_success() {
            return Err(CloudAiCallError::Refused);
        }
        let value: Value = resp.json().map_err(|_| CloudAiCallError::Network)?;
        let output_text = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .ok_or(CloudAiCallError::Network)?
            .to_string();
        Ok(CloudAiResponse { output_text })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_last4_takes_the_tail() {
        assert_eq!(key_last4("sk-abcdefgh"), "efgh");
        assert_eq!(key_last4("ab"), "ab");
        assert_eq!(key_last4(""), "");
    }

    #[test]
    fn validate_endpoint_url_accepts_https_with_host() {
        assert_eq!(
            validate_endpoint_url("https://api.example.com/v1/chat").unwrap(),
            "api.example.com"
        );
        assert_eq!(
            validate_endpoint_url("https://api.example.com:8443/v1").unwrap(),
            "api.example.com:8443"
        );
    }

    #[test]
    fn validate_endpoint_url_rejects_non_https_file_and_userinfo() {
        assert!(validate_endpoint_url("http://api.example.com").is_err());
        assert!(validate_endpoint_url("file:///etc/passwd").is_err());
        assert!(validate_endpoint_url("https://user:pass@api.example.com").is_err());
        assert!(validate_endpoint_url("https://").is_err());
        assert!(validate_endpoint_url("not-a-url").is_err());
    }
}
