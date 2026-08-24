//! OQ-6 egress oracle (testing.md §7.2). Test harness only.

use flate2::read::{DeflateDecoder, ZlibDecoder};
use std::io::Read;

/// High-entropy redacted canary from testing.md §7.2 (must be ≥ 8 codepoints).
pub const REDACT_CANARY: &str = "PG-CANARY-REDACT-7F3A";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleFailure {
    pub canary: String,
    pub place: &'static str,
}

/// Scan egress bytes for redacted canaries (must be absent) and keep canaries
/// (must appear in extracted PDF text).
pub fn check(egress: &[u8], redacted: &[&str], kept: &[&str]) -> Result<(), Vec<OracleFailure>> {
    let mut failures = Vec::new();
    let extracted = pdf_extract::extract_text_from_mem(egress).unwrap_or_default();
    let inflated = inflate_flate_streams(egress);

    for s in redacted {
        assert!(
            s.chars().count() >= 8,
            "testing.md §7.2: redacted oracle canaries must be ≥ 8 codepoints, got {s:?}"
        );
        if contains_utf8(egress, s) {
            failures.push(OracleFailure {
                canary: (*s).to_string(),
                place: "raw-utf8",
            });
        }
        if contains_utf16(egress, s) {
            failures.push(OracleFailure {
                canary: (*s).to_string(),
                place: "raw-utf16",
            });
        }
        if extracted.contains(s) {
            failures.push(OracleFailure {
                canary: (*s).to_string(),
                place: "extracted-text",
            });
        }
        if contains_utf8(&inflated, s) || contains_utf16(&inflated, s) {
            failures.push(OracleFailure {
                canary: (*s).to_string(),
                place: "flate-stream",
            });
        }
    }
    for s in kept {
        if !extracted.contains(s) {
            failures.push(OracleFailure {
                canary: (*s).to_string(),
                place: "keep-missing",
            });
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

fn contains_utf8(haystack: &[u8], needle: &str) -> bool {
    let n = needle.as_bytes();
    haystack.windows(n.len()).any(|w| w == n)
}

fn contains_utf16(haystack: &[u8], needle: &str) -> bool {
    let le: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let be: Vec<u8> = needle.encode_utf16().flat_map(u16::to_be_bytes).collect();
    haystack.windows(le.len()).any(|w| w == le.as_slice())
        || haystack.windows(be.len()).any(|w| w == be.as_slice())
}

/// Concatenate inflated payloads of every `stream`…`endstream` that follows `/FlateDecode`.
fn inflate_flate_streams(pdf: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let bytes = pdf;
    let mut i = 0;
    while i < bytes.len() {
        if let Some(rel) = find(bytes, i, b"/FlateDecode") {
            i = rel + b"/FlateDecode".len();
            if let Some(stream_at) = find(bytes, i, b"stream") {
                let after = skip_stream_header(bytes, stream_at);
                if let Some(end) = find(bytes, after, b"endstream") {
                    let payload = &bytes[after..end];
                    out.extend(try_inflate(payload));
                    i = end + b"endstream".len();
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

fn skip_stream_header(bytes: &[u8], stream_at: usize) -> usize {
    let mut i = stream_at + b"stream".len();
    if bytes.get(i) == Some(&b'\r') {
        i += 1;
    }
    if bytes.get(i) == Some(&b'\n') {
        i += 1;
    }
    i
}

fn try_inflate(payload: &[u8]) -> Vec<u8> {
    let mut zlib = Vec::new();
    if ZlibDecoder::new(payload).read_to_end(&mut zlib).is_ok() && !zlib.is_empty() {
        return zlib;
    }
    let mut raw = Vec::new();
    if DeflateDecoder::new(payload).read_to_end(&mut raw).is_ok() {
        return raw;
    }
    Vec::new()
}

/// Plant `canary` as a FlateDecode stream after `pdf` (oracle self-test fixture).
pub fn inject_flate_canary(pdf: &[u8], canary: &str) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(canary.as_bytes()).expect("compress");
    let compressed = encoder.finish().expect("finish");
    let mut out = pdf.to_vec();
    out.extend_from_slice(b"\n1 0 obj\n<< /Length ");
    out.extend_from_slice(compressed.len().to_string().as_bytes());
    out.extend_from_slice(b" /Filter /FlateDecode >>\nstream\n");
    out.extend_from_slice(&compressed);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    out
}
