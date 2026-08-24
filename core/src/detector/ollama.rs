//! `pg-hybrid-ollama-v1` — W13 patterns plus NER via a local Ollama server (W15b).
//!
//! architecture.md §10.1 / decision 0009. This host is **selectable** via
//! `SessionManager::with_detector`; it is not `import_document`'s default (W15c).
//! Handshake (tags + show + allowlist + digest) happens before any document text is
//! sent. The HTTP client is IP-literal loopback only, no DNS, no ambient proxy, no
//! redirects.

use std::net::SocketAddr;
use std::time::Duration;

use serde_json::{json, Value};

use crate::catalog::DetectedField;
use crate::importer::{Document, TextSpan};

use super::{Detector, PatternsUkV1};

/// architecture.md §10.1 identity for the optional Ollama backend.
pub const HYBRID_OLLAMA_V1_ID: &str = "pg-hybrid-ollama-v1";

/// Seed allowlist tag (architecture §10.1.2). Extending the list is an architecture
/// amendment.
pub const OLLAMA_ALLOWLISTED_TAG: &str = "gemma4:e2b";

/// Digest pinned for [`OLLAMA_ALLOWLISTED_TAG`] (architecture §10.1.2 / §4.2 discipline).
/// `None` until a nightly golden records the real `/api/show` (or `/api/tags`) digest —
/// an unrecorded pin must not silently accept a live model.
pub const OLLAMA_GEMMA4_E2B_DIGEST: Option<&str> = None;

/// architecture §10.1.5: chunk size / overlap. Conservative byte bounds, **not** a claim
/// about `gemma4:e2b`'s context window — that figure is [`GEMMA4_E2B_CONTEXT_TOKENS`]
/// and stays `None` until the nightly golden fills it.
pub const CHUNK_SIZE_BYTES: usize = 4096;
pub const CHUNK_OVERLAP_BYTES: usize = 256;
pub const GEMMA4_E2B_CONTEXT_TOKENS: Option<u32> = None;

/// architecture §10.1.4: a chunk whose rejected/total entity rate **exceeds** this
/// value fails the whole document's Ollama pass.
pub const OFFSET_REJECT_THRESHOLD: f64 = 0.5;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(200);
const GENERATE_TIMEOUT: Duration = Duration::from_secs(20);

/// data-model.md / decision 0009 `fallback_reason` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    OllamaUnreachable,
    SchemaVerificationFailed,
    ModelNotAllowlisted,
    DigestMismatch,
    OffsetVerificationFailed,
}

impl FallbackReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OllamaUnreachable => "ollama_unreachable",
            Self::SchemaVerificationFailed => "schema_verification_failed",
            Self::ModelNotAllowlisted => "model_not_allowlisted",
            Self::DigestMismatch => "digest_mismatch",
            Self::OffsetVerificationFailed => "offset_verification_failed",
        }
    }
}

/// One hardcoded allowlist row (tag + digest pin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllowlistEntry {
    pub tag: &'static str,
    pub digest: &'static str,
}

/// Result of one Ollama-backed detect. [`Detector::detect`] returns `fields` only;
/// W15c reads the rest for the audit `detect` event.
#[derive(Debug, Clone)]
pub struct OllamaDetectOutcome {
    pub fields: Vec<DetectedField>,
    pub fallback_reason: Option<FallbackReason>,
    pub model_tag: Option<String>,
}

/// Loopback-only Ollama HTTP client (architecture §10.1.1).
pub struct OllamaClient {
    base: String,
    http: reqwest::blocking::Client,
    allowlist: Vec<AllowlistEntry>,
}

impl OllamaClient {
    /// `addr` must be a loopback IP (`127.0.0.1` / `::1`). Hostnames are not accepted
    /// because this type takes a [`SocketAddr`], not a name — no DNS path exists.
    pub fn connect(
        addr: SocketAddr,
        allowlist: Vec<AllowlistEntry>,
    ) -> Result<Self, FallbackReason> {
        if !addr.ip().is_loopback() {
            return Err(FallbackReason::OllamaUnreachable);
        }
        let http = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| FallbackReason::OllamaUnreachable)?;
        Ok(Self {
            base: format!("http://{addr}"),
            http,
            allowlist,
        })
    }

    fn get_tags(&self) -> Result<Value, FallbackReason> {
        let url = format!("{}/api/tags", self.base);
        let resp = self
            .http
            .get(&url)
            .timeout(HANDSHAKE_TIMEOUT)
            .send()
            .map_err(|_| FallbackReason::OllamaUnreachable)?;
        if !resp.status().is_success() {
            return Err(FallbackReason::SchemaVerificationFailed);
        }
        resp.json::<Value>()
            .map_err(|_| FallbackReason::SchemaVerificationFailed)
    }

    fn post_show(&self, tag: &str) -> Result<Value, FallbackReason> {
        let url = format!("{}/api/show", self.base);
        let resp = self
            .http
            .post(&url)
            .timeout(HANDSHAKE_TIMEOUT)
            .json(&json!({ "model": tag }))
            .send()
            .map_err(|_| FallbackReason::OllamaUnreachable)?;
        if !resp.status().is_success() {
            return Err(FallbackReason::SchemaVerificationFailed);
        }
        resp.json::<Value>()
            .map_err(|_| FallbackReason::SchemaVerificationFailed)
    }

    fn post_generate(&self, tag: &str, chunk: &str) -> Result<Value, FallbackReason> {
        let url = format!("{}/api/generate", self.base);
        let format = entity_json_schema();
        let resp = self
            .http
            .post(&url)
            .timeout(GENERATE_TIMEOUT)
            .json(&json!({
                "model": tag,
                "prompt": chunk,
                "stream": false,
                "format": format,
            }))
            .send()
            .map_err(|_| FallbackReason::OllamaUnreachable)?;
        if !resp.status().is_success() {
            return Err(FallbackReason::SchemaVerificationFailed);
        }
        resp.json::<Value>()
            .map_err(|_| FallbackReason::SchemaVerificationFailed)
    }

    /// Probe tags + show + allowlist + digest. Never sends document text.
    pub fn handshake(&self) -> Result<Handshake, FallbackReason> {
        let tags = self.get_tags()?;
        let models = tags
            .get("models")
            .and_then(Value::as_array)
            .ok_or(FallbackReason::SchemaVerificationFailed)?;

        let mut chosen: Option<(String, String)> = None;
        for model in models {
            let name = model
                .get("name")
                .and_then(Value::as_str)
                .ok_or(FallbackReason::SchemaVerificationFailed)?;
            let digest = model
                .get("digest")
                .and_then(Value::as_str)
                .ok_or(FallbackReason::SchemaVerificationFailed)?;
            if name.ends_with("-cloud") {
                continue;
            }
            if self.allowlist.iter().any(|e| e.tag == name) {
                chosen = Some((name.to_string(), digest.to_string()));
                break;
            }
        }
        let (tag, tags_digest) = chosen.ok_or(FallbackReason::ModelNotAllowlisted)?;

        let show = self.post_show(&tag)?;
        if !show.is_object() || show.get("details").and_then(Value::as_object).is_none() {
            return Err(FallbackReason::SchemaVerificationFailed);
        }
        let digest = show
            .get("digest")
            .and_then(Value::as_str)
            .unwrap_or(tags_digest.as_str());
        let expected = self
            .allowlist
            .iter()
            .find(|e| e.tag == tag)
            .map(|e| e.digest)
            .ok_or(FallbackReason::ModelNotAllowlisted)?;
        if digest != expected {
            return Err(FallbackReason::DigestMismatch);
        }
        Ok(Handshake {
            model_tag: tag,
            digest: digest.to_string(),
        })
    }
}

/// architecture §10.1.4: `chunk[start..start+length] == text`, byte-exact, never a search.
pub fn verify_chunk_entity(chunk: &str, start: u32, length: u32, text: &str) -> bool {
    let start = start as usize;
    let end = match start.checked_add(length as usize) {
        Some(e) => e,
        None => return false,
    };
    match chunk.get(start..end) {
        Some(slice) => slice == text,
        None => false,
    }
}

fn entity_json_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "entities": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "start": { "type": "integer" },
                        "length": { "type": "integer" },
                        "label": { "type": "string", "enum": ["person", "location", "organization"] },
                        "text": { "type": "string" }
                    },
                    "required": ["start", "length", "label", "text"]
                }
            }
        },
        "required": ["entities"]
    })
}

pub struct Handshake {
    pub model_tag: String,
    pub digest: String,
}

/// Selectable Ollama hybrid host. Pattern pack always runs; NER runs only after a
/// successful handshake and offset-verified generate.
pub struct HybridOllamaV1 {
    patterns: PatternsUkV1,
    client: OllamaClient,
}

impl HybridOllamaV1 {
    pub fn new(client: OllamaClient) -> Self {
        Self {
            patterns: PatternsUkV1,
            client,
        }
    }

    pub fn detect_with_outcome(&self, doc: &Document) -> OllamaDetectOutcome {
        let pattern_fields = self.patterns.detect(doc);
        match self.client.handshake() {
            Err(reason) => OllamaDetectOutcome {
                fields: pattern_fields,
                fallback_reason: Some(reason),
                model_tag: None,
            },
            Ok(hs) => match self.ner_pass(doc, &hs) {
                Ok(ner) => {
                    let mut fields = pattern_fields;
                    fields.extend(ner);
                    OllamaDetectOutcome {
                        fields,
                        fallback_reason: None,
                        model_tag: Some(hs.model_tag),
                    }
                }
                Err(reason) => OllamaDetectOutcome {
                    fields: pattern_fields,
                    fallback_reason: Some(reason),
                    model_tag: None,
                },
            },
        }
    }

    fn ner_pass(
        &self,
        doc: &Document,
        hs: &Handshake,
    ) -> Result<Vec<DetectedField>, FallbackReason> {
        let mut fields = Vec::new();
        for page in &doc.pages {
            for span in &page.spans {
                for chunk in split_chunks(&span.text, CHUNK_SIZE_BYTES, CHUNK_OVERLAP_BYTES) {
                    let raw = self.client.post_generate(&hs.model_tag, &chunk.text)?;
                    let entities = parse_entities(&raw)?;
                    let total = entities.len();
                    let mut rejected = 0usize;
                    for ent in entities {
                        if !verify_chunk_entity(&chunk.text, ent.start, ent.length, &ent.text) {
                            rejected += 1;
                            continue;
                        }
                        let abs = span.byte_offset + chunk.abs_start as u64 + u64::from(ent.start);
                        fields.push(DetectedField {
                            id: uuid::Uuid::new_v4().to_string(),
                            label: ent.label,
                            classification: "ner".to_string(),
                            span: TextSpan {
                                byte_offset: abs,
                                byte_length: u64::from(ent.length),
                                text: ent.text,
                                page_index: span.page_index,
                            },
                            parent_field_id: None,
                        });
                    }
                    if total > 0 {
                        let rate = rejected as f64 / total as f64;
                        if rate > OFFSET_REJECT_THRESHOLD {
                            return Err(FallbackReason::OffsetVerificationFailed);
                        }
                    }
                }
            }
        }
        Ok(fields)
    }
}

impl Detector for HybridOllamaV1 {
    fn id(&self) -> &'static str {
        HYBRID_OLLAMA_V1_ID
    }

    fn detect(&self, doc: &Document) -> Vec<DetectedField> {
        self.detect_with_outcome(doc).fields
    }
}

struct Chunk {
    abs_start: usize,
    text: String,
}

struct Entity {
    start: u32,
    length: u32,
    label: String,
    text: String,
}

fn split_chunks(text: &str, size: usize, overlap: usize) -> Vec<Chunk> {
    if text.is_empty() || size == 0 {
        return Vec::new();
    }
    let overlap = overlap.min(size.saturating_sub(1));
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < bytes.len() {
        let mut end = (start + size).min(bytes.len());
        while end < bytes.len() && !text.is_char_boundary(end) {
            end += 1;
        }
        let mut adj = start;
        while adj > 0 && !text.is_char_boundary(adj) {
            adj -= 1;
        }
        out.push(Chunk {
            abs_start: adj,
            text: text[adj..end].to_string(),
        });
        if end >= bytes.len() {
            break;
        }
        start = end.saturating_sub(overlap);
        while start < bytes.len() && !text.is_char_boundary(start) {
            start += 1;
        }
        if start <= adj {
            start = end;
        }
    }
    out
}

fn parse_entities(raw: &Value) -> Result<Vec<Entity>, FallbackReason> {
    let payload = if let Some(s) = raw.get("response").and_then(Value::as_str) {
        serde_json::from_str::<Value>(s).map_err(|_| FallbackReason::SchemaVerificationFailed)?
    } else {
        raw.clone()
    };
    let arr = payload
        .get("entities")
        .and_then(Value::as_array)
        .ok_or(FallbackReason::SchemaVerificationFailed)?;
    let mut out = Vec::new();
    for v in arr {
        let start = v
            .get("start")
            .and_then(Value::as_u64)
            .ok_or(FallbackReason::SchemaVerificationFailed)? as u32;
        let length = v
            .get("length")
            .and_then(Value::as_u64)
            .ok_or(FallbackReason::SchemaVerificationFailed)? as u32;
        let label = v
            .get("label")
            .and_then(Value::as_str)
            .ok_or(FallbackReason::SchemaVerificationFailed)?
            .to_ascii_lowercase();
        if !matches!(label.as_str(), "person" | "location" | "organization") {
            return Err(FallbackReason::SchemaVerificationFailed);
        }
        let text = v
            .get("text")
            .and_then(Value::as_str)
            .ok_or(FallbackReason::SchemaVerificationFailed)?
            .to_string();
        out.push(Entity {
            start,
            length,
            label,
            text,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_chunk_entity_accepts_an_exact_slice() {
        assert!(verify_chunk_entity("Alice in Wonderland", 0, 5, "Alice"));
    }

    #[test]
    fn verify_chunk_entity_rejects_a_wrong_slice() {
        // "Alice" is at 0; claiming start=6 ("in Wo") must not search for Alice.
        assert!(!verify_chunk_entity("Alice in Wonderland", 6, 5, "Alice"));
    }

    #[test]
    fn fallback_reason_wire_strings_match_decision_0009() {
        assert_eq!(FallbackReason::OllamaUnreachable.as_str(), "ollama_unreachable");
        assert_eq!(
            FallbackReason::SchemaVerificationFailed.as_str(),
            "schema_verification_failed"
        );
        assert_eq!(
            FallbackReason::ModelNotAllowlisted.as_str(),
            "model_not_allowlisted"
        );
        assert_eq!(FallbackReason::DigestMismatch.as_str(), "digest_mismatch");
        assert_eq!(
            FallbackReason::OffsetVerificationFailed.as_str(),
            "offset_verification_failed"
        );
    }
}
