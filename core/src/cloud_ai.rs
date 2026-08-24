//! Cloud AI plugin: the envelope-encrypted `CloudAiSecret` (`kind=5`, data-model §5.7) and
//! the Rust-side HTTP client that speaks to the user-configured host (`architecture.md`
//! §8–§9; W27).
//!
//! Same two-AEAD-layer shape `crate::config` established for `kind=4` — a fresh per-write
//! DEK wraps the plaintext JSON under AAD kind 5 (`ArtifactKind::PluginSecret`),
//! `vault_master_key` wraps that DEK under AAD kind 7 — except unlike `Config`, absence is
//! meaningful ("not configured", data-model §5.7), so this module's store trait has an
//! explicit `clear` alongside `load`/`store` rather than `Config`'s always-write `store`.
//!
//! # Where the secret lives and which process speaks HTTP (architecture §9)
//! The API key never crosses the webview boundary after `cloud_ai_set_config` returns
//! (architecture §9.1). All Cloud AI HTTP happens in this module, in the Rust core
//! (`reqwest` + `rustls`, already a W15b dependency for the Ollama loopback client) — the
//! webview has no HTTP capability (C-ARCH-2).
//!
//! # TLS-mock gap (documented per dev-plan W27 instructions)
//! `endpoint_url` must be `https://` at `cloud_ai_set_config` time
//! ([`validate_https_endpoint`]) — that check is unit-tested directly. There is no TLS
//! testing double anywhere in this repo (`testing.md` §6.3 calls for a "mock allowlisted
//! HTTPS origin" but the existing test infrastructure — `core/tests/ollama_w15b.rs`'s
//! plain-HTTP `TcpListener` mock — has no TLS crate to build one from, and this chunk does
//! not add one). So the actual HTTP-send path (redirect refusal, body identity, failure
//! auditing) is tested against a plain-HTTP mock reached via
//! [`crate::session::SessionManager::test_only_set_cloud_ai_secret`], which bypasses the
//! `https://` validation the same way production `cloud_ai_set_config` never would. What is
//! proven: the https-only gate at config time, and the send/redirect/audit logic against a
//! real socket. What is *not* proven: that a real TLS handshake against a real HTTPS host
//! behaves the same way — that would need a TLS-capable mock, a gap this dev-log carries
//! forward explicitly (mirrors `OLLAMA_GEMMA4_E2B_DIGEST` being `None`-pinned in W15b).
//!
//! # Redirect policy (dev-plan W27 instructions)
//! architecture §9.2: "Redirects that change host are refused." Implemented as a custom
//! [`reqwest::redirect::Policy`] that inspects the redirect target's host against the
//! initial request's host and refuses only a *host-changing* redirect — a same-host
//! redirect (e.g. `https://api.example.com/v1` -> `https://api.example.com/v1/`) is still
//! followed. This is the literal spec wording, not the stricter "refuse all redirects"
//! reading dev-plan's task notes offered as an acceptable fallback.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto::{Aad, ArtifactKind, Dek, WrappedBlob, DEK_LEN};
use crate::keys::VaultMasterKey;

/// `format_version` bound into the AAD of both layers.
pub const CLOUD_AI_FORMAT_VERSION: u32 = 1;

/// v1's only plugin id (data-model §7 `plugin_secret.plugin_id`).
pub const CLOUD_AI_PLUGIN_ID: &str = "cloud_ai";

/// data-model §5.7 `CloudAiSecret`. `Debug` is hand-written to redact `api_key` — the same
/// discipline `crate::keys::VaultMasterKey` and friends use for key material — even though
/// this struct also crosses the DTO boundary at `cloud_ai_set_config`'s `In` side.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudAiSecret {
    pub endpoint_url: String,
    pub model: String,
    pub api_key: String,
    /// Not a secret (data-model §5.7/§6.5): stored at set-config time so `cloud_ai_get_config`
    /// need not decrypt-then-slice the key on every read.
    pub key_last4: String,
}

impl core::fmt::Debug for CloudAiSecret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CloudAiSecret")
            .field("endpoint_url", &self.endpoint_url)
            .field("model", &self.model)
            .field("api_key", &"<redacted>")
            .field("key_last4", &self.key_last4)
            .finish()
    }
}

/// Failure modes of the plugin-secret backend. Coarse and non-secret, same discipline as
/// every other error class in the core.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloudAiSecretError {
    Backend(&'static str),
}

impl core::fmt::Display for CloudAiSecretError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CloudAiSecretError::Backend(class) => write!(f, "plugin secret backend failure: {class}"),
        }
    }
}

impl std::error::Error for CloudAiSecretError {}

/// Where the `CloudAiSecret` artifact lives. `crate::vault::SqlCipherVault` implements this
/// over the `plugin_secret` table's single `plugin_id = 'cloud_ai'` row (data-model §7) plus
/// its `kind=5` artifact.
pub trait PluginSecretStore: Send + Sync {
    /// `None` if Cloud AI has never been configured, or was cleared — absence is
    /// "not configured" (data-model §5.7), not an error.
    ///
    /// # Errors
    /// [`CloudAiSecretError::Backend`] on any I/O/backend/decrypt failure.
    fn load(&self, master: &VaultMasterKey) -> Result<Option<CloudAiSecret>, CloudAiSecretError>;

    /// Upsert the single `cloud_ai` row + its `kind=5` artifact (data-model §8).
    ///
    /// # Errors
    /// [`CloudAiSecretError::Backend`] on any I/O/backend/encrypt failure.
    fn store(&self, master: &VaultMasterKey, secret: &CloudAiSecret) -> Result<(), CloudAiSecretError>;

    /// Cryptographic erase (architecture §4.3): destroy the `kind=5` artifact's DEK, then
    /// drop both rows. Idempotent — `Ok(())` even if nothing was configured (same posture
    /// as `delete_retained_original`).
    ///
    /// # Errors
    /// [`CloudAiSecretError::Backend`] on I/O/backend failure.
    fn clear(&self) -> Result<(), CloudAiSecretError>;
}

/// The pre-W27-era no-op backend: `load` reports "not configured", `store` errors (nothing
/// to write to), `clear` is a no-op. Exists so every constructor that predates W27 keeps
/// working unmodified.
#[derive(Debug, Default)]
pub struct NullPluginSecretStore;

impl PluginSecretStore for NullPluginSecretStore {
    fn load(&self, _master: &VaultMasterKey) -> Result<Option<CloudAiSecret>, CloudAiSecretError> {
        Ok(None)
    }
    fn store(&self, _master: &VaultMasterKey, _secret: &CloudAiSecret) -> Result<(), CloudAiSecretError> {
        Err(CloudAiSecretError::Backend("no plugin secret store configured"))
    }
    fn clear(&self) -> Result<(), CloudAiSecretError> {
        Ok(())
    }
}

/// AAD for the plugin-secret plaintext layer (architecture §3.1, kind 5, not
/// document-scoped).
#[must_use]
pub fn cloud_ai_plaintext_aad() -> Aad {
    Aad::global(ArtifactKind::PluginSecret, CLOUD_AI_FORMAT_VERSION)
}

/// AAD for the plugin-secret artifact's DEK-wrap layer (kind 7, not document-scoped).
#[must_use]
pub fn cloud_ai_dek_wrap_aad() -> Aad {
    Aad::global(ArtifactKind::WrappedDek, CLOUD_AI_FORMAT_VERSION)
}

/// Encrypt `secret` under a fresh DEK, then wrap that DEK under `master`. Mirrors
/// `crate::config::seal_config`.
///
/// # Errors
/// Whatever the underlying AEAD wrap calls return (CSPRNG failure).
pub fn seal_cloud_ai_secret(
    master: &VaultMasterKey,
    secret: &CloudAiSecret,
) -> Result<(WrappedBlob, WrappedBlob), CloudAiSecretError> {
    let dek = Dek::generate();
    let plaintext =
        serde_json::to_vec(secret).map_err(|_| CloudAiSecretError::Backend("serialize failed"))?;
    let artifact_blob = crate::crypto::wrap(dek.as_bytes(), &plaintext, &cloud_ai_plaintext_aad())
        .map_err(|_| CloudAiSecretError::Backend("artifact wrap failed"))?;
    let wrapped_dek = crate::crypto::wrap(master.as_bytes(), dek.as_bytes(), &cloud_ai_dek_wrap_aad())
        .map_err(|_| CloudAiSecretError::Backend("dek wrap failed"))?;
    Ok((wrapped_dek, artifact_blob))
}

/// The inverse of [`seal_cloud_ai_secret`].
///
/// # Errors
/// [`CloudAiSecretError::Backend`] if either AEAD layer fails to authenticate, or the
/// plaintext is not valid `CloudAiSecret` JSON.
pub fn open_cloud_ai_secret(
    master: &VaultMasterKey,
    wrapped_dek: &WrappedBlob,
    artifact_blob: &WrappedBlob,
) -> Result<CloudAiSecret, CloudAiSecretError> {
    let dek_bytes: Zeroizing<Vec<u8>> = Zeroizing::new(
        crate::crypto::unwrap(master.as_bytes(), wrapped_dek, &cloud_ai_dek_wrap_aad())
            .map_err(|_| CloudAiSecretError::Backend("dek unwrap failed"))?,
    );
    let dek_array: [u8; DEK_LEN] = dek_bytes
        .as_slice()
        .try_into()
        .map_err(|_| CloudAiSecretError::Backend("dek has the wrong length"))?;
    let dek = Dek::from_bytes(dek_array);

    let plaintext = crate::crypto::unwrap(dek.as_bytes(), artifact_blob, &cloud_ai_plaintext_aad())
        .map_err(|_| CloudAiSecretError::Backend("artifact unwrap failed"))?;
    serde_json::from_slice(&plaintext).map_err(|_| CloudAiSecretError::Backend("malformed cloud ai secret JSON"))
}

/// Last 4 characters of an API key (data-model §5.7 `key_last4` — "not a secret"). Whole
/// key if shorter than 4 characters (still not sensitive on its own — `cloud_ai_get_config`
/// never returns the key itself).
#[must_use]
pub fn last4(api_key: &str) -> String {
    let chars: Vec<char> = api_key.chars().collect();
    let start = chars.len().saturating_sub(4);
    chars[start..].iter().collect()
}

/// api.md §5.7: `endpoint_url` must be `https://` with a host; `file://`, `http://`, and
/// userinfo in the URL are `invalid_input`. Hand-rolled rather than pulling in the `url`
/// crate — this repo is deliberate about its dependency list (`core/Cargo.toml`'s
/// per-dependency comments) and this is a small, fully-tested check.
///
/// Returns the host (no port, no scheme) on success.
#[must_use]
pub fn validate_https_endpoint(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    if rest.is_empty() {
        return None;
    }
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() || authority.contains('@') {
        // Userinfo (`user:pass@host`) is rejected outright, not stripped.
        return None;
    }
    let host = authority.split(':').next().unwrap_or(authority);
    if host.is_empty() {
        return None;
    }
    Some(host.to_string())
}

// ---------------------------------------------------------------------------
// HTTP client (architecture §9.2). All Cloud AI network I/O lives here.
// ---------------------------------------------------------------------------

/// api.md §5.6/§9.3: `error_class` values this module can produce. Never includes the
/// failing host or any response body (C-API-1 discipline — no caller input in a message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudAiSendError {
    /// Connection failure, timeout, non-success status, or an unparsable response.
    Network,
    /// A redirect that would change host (architecture §9.2).
    Refused,
}

impl CloudAiSendError {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CloudAiSendError::Network => "network",
            CloudAiSendError::Refused => "refused",
        }
    }
}

/// First-party system preamble (architecture §9.2 / api.md §5.6: "a fixed system preamble
/// ... contains no vault secrets, and is not shown in the preview"). Wraps the approved
/// document body; the instruction is layered on top by [`CloudAiClient::send`].
const SYSTEM_PREAMBLE: &str = "You are assisting a Privacy Gate user with a document they have \
already reviewed and redacted themselves. Only the approved, redacted text below is provided. \
Follow the user's instruction about this text.";

const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Rust-core HTTP client for the user-configured Cloud AI host (architecture §9.2). Not
/// loopback-only (unlike [`crate::detector::ollama::OllamaClient`]) — it speaks to whatever
/// host `endpoint_url` names, refusing only a host-changing redirect.
pub struct CloudAiClient {
    endpoint_url: String,
    api_key: String,
    model: String,
}

impl CloudAiClient {
    #[must_use]
    pub fn new(endpoint_url: String, api_key: String, model: String) -> Self {
        Self {
            endpoint_url,
            api_key,
            model,
        }
    }

    fn build_http(&self) -> Result<reqwest::blocking::Client, CloudAiSendError> {
        let initial_host = url_host(&self.endpoint_url).ok_or(CloudAiSendError::Network)?;
        reqwest::blocking::Client::builder()
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                let same_host = attempt
                    .url()
                    .host_str()
                    .is_some_and(|h| h.eq_ignore_ascii_case(&initial_host));
                if same_host {
                    attempt.follow()
                } else {
                    attempt.error(RedirectHostChanged)
                }
            }))
            .build()
            .map_err(|_| CloudAiSendError::Network)
    }

    /// Send `approved_text` wrapped by `instruction` and the fixed preamble. Returns the
    /// model's read-only text (api.md §5.6 `output_text`). The exact bytes POSTed as the
    /// approved-document body are `approved_text` verbatim, byte-identical to what
    /// `preview_share` showed as `ai_payload_preview` (api.md §5.6 identity guarantee) —
    /// this function only adds the preamble/instruction wrapper around it, never mutates it.
    pub fn send(&self, instruction: &str, approved_text: &str) -> Result<String, CloudAiSendError> {
        let http = self.build_http()?;
        let user_content = format!("{instruction}\n\n---\n\n{approved_text}");
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": SYSTEM_PREAMBLE },
                { "role": "user", "content": user_content },
            ],
        });
        let resp = http
            .post(&self.endpoint_url)
            .bearer_auth(&self.api_key)
            .timeout(SEND_TIMEOUT)
            .json(&body)
            .send()
            .map_err(classify_send_error)?;
        if !resp.status().is_success() {
            return Err(CloudAiSendError::Network);
        }
        let value: serde_json::Value = resp.json().map_err(|_| CloudAiSendError::Network)?;
        extract_output_text(&value).ok_or(CloudAiSendError::Network)
    }

    /// `cloud_ai_test` (api.md §5.7): a lightweight probe, no vault document content.
    pub fn probe(&self) -> Result<(), CloudAiSendError> {
        let http = self.build_http()?;
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "user", "content": "ping" },
            ],
        });
        let resp = http
            .post(&self.endpoint_url)
            .bearer_auth(&self.api_key)
            .timeout(HANDSHAKE_TIMEOUT)
            .json(&body)
            .send()
            .map_err(classify_send_error)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(CloudAiSendError::Network)
        }
    }
}

/// Marker error the custom redirect policy raises for a host-changing redirect
/// (`reqwest::redirect::Policy::custom`'s `Attempt::error`).
#[derive(Debug)]
struct RedirectHostChanged;

impl core::fmt::Display for RedirectHostChanged {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("redirect changed host")
    }
}

impl std::error::Error for RedirectHostChanged {}

fn classify_send_error(err: reqwest::Error) -> CloudAiSendError {
    // `reqwest::Error::is_redirect` is `true` exactly when the error came from the redirect
    // policy — the only redirect error this policy ever raises is our host-changed one.
    if err.is_redirect() {
        CloudAiSendError::Refused
    } else {
        CloudAiSendError::Network
    }
}

/// Extract the host (no port, no scheme) from a URL — used both to seed the redirect
/// policy's same-host check and (by `crate::session`) as the audit payload's
/// `endpoint_host` (api.md §5.8). `None` for an unparsable URL.
#[must_use]
pub fn url_host(endpoint_url: &str) -> Option<String> {
    reqwest::Url::parse(endpoint_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
}

/// OpenAI-compatible chat-completion response shape (architecture §9.1: "OpenAI-compatible
/// base URL"): `choices[0].message.content`.
fn extract_output_text(value: &serde_json::Value) -> Option<String> {
    value
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_https_endpoint_accepts_a_bare_https_host() {
        assert_eq!(
            validate_https_endpoint("https://api.example.com/v1"),
            Some("api.example.com".to_string())
        );
    }

    #[test]
    fn validate_https_endpoint_accepts_a_port() {
        assert_eq!(
            validate_https_endpoint("https://api.example.com:8443/v1"),
            Some("api.example.com".to_string())
        );
    }

    #[test]
    fn validate_https_endpoint_rejects_http() {
        assert_eq!(validate_https_endpoint("http://api.example.com/v1"), None);
    }

    #[test]
    fn validate_https_endpoint_rejects_file_scheme() {
        assert_eq!(validate_https_endpoint("file:///etc/passwd"), None);
    }

    #[test]
    fn validate_https_endpoint_rejects_userinfo() {
        assert_eq!(
            validate_https_endpoint("https://user:pass@api.example.com/v1"),
            None
        );
    }

    #[test]
    fn validate_https_endpoint_rejects_no_host() {
        assert_eq!(validate_https_endpoint("https://"), None);
    }

    #[test]
    fn last4_of_a_normal_key() {
        assert_eq!(last4("sk-abcdEFGH1234"), "1234");
    }

    #[test]
    fn last4_of_a_short_key() {
        assert_eq!(last4("ab"), "ab");
    }

    #[test]
    fn debug_never_includes_the_api_key() {
        let secret = CloudAiSecret {
            endpoint_url: "https://api.example.com/v1".to_string(),
            model: "gpt-x".to_string(),
            api_key: "sk-super-secret-value".to_string(),
            key_last4: "alue".to_string(),
        };
        let rendered = format!("{secret:?}");
        assert!(!rendered.contains("sk-super-secret-value"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn seal_and_open_round_trip() {
        let master = VaultMasterKey::generate().expect("generate master key");
        let secret = CloudAiSecret {
            endpoint_url: "https://api.example.com/v1".to_string(),
            model: "gpt-x".to_string(),
            api_key: "sk-abc123".to_string(),
            key_last4: "c123".to_string(),
        };
        let (wrapped_dek, blob) = seal_cloud_ai_secret(&master, &secret).expect("seal");
        let opened = open_cloud_ai_secret(&master, &wrapped_dek, &blob).expect("open");
        assert_eq!(opened, secret);
    }
}
