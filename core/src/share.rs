//! Share Engine helpers (api.md §5.6–§7; W24).
//!
//! Filename sanitization and export-PDF assembly. Session commands own the preview token
//! lifecycle. Ephemeral overrides / applied variants are W26; Cloud AI is W27.

use crate::catalog::RedactedPage;
use crate::export::{render_redacted_pages, PdfExportInfo};

const PREVIEW_TTL_MS: u64 = 10 * 60 * 1000;

/// How long a preview token lives (api.md §5.6).
#[must_use]
pub const fn preview_ttl_ms() -> u64 {
    PREVIEW_TTL_MS
}

/// api.md §7.1: Unicode letters/digits kept, other characters become `-`, collapse,
/// max 40, empty → `document`.
#[must_use]
pub fn sanitize_stem(source_filename: &str) -> String {
    let stem = source_filename
        .rsplit_once('.')
        .map(|(s, ext)| {
            if ext.is_empty() {
                source_filename
            } else {
                s
            }
        })
        .unwrap_or(source_filename);
    let mut out = String::new();
    let mut last_dash = false;
    for c in stem.chars() {
        if c.is_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed: String = out.trim_matches('-').chars().take(40).collect();
    let trimmed = trimmed.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "document".to_string()
    } else {
        trimmed
    }
}

/// UTC `YYYYMMDD` from Unix milliseconds.
#[must_use]
pub fn yyyymmdd_utc(unix_ms: u64) -> String {
    let rfc = crate::account::format_rfc3339((unix_ms / 1000) as i64);
    format!(
        "{}{}{}",
        rfc.get(0..4).unwrap_or("1970"),
        rfc.get(5..7).unwrap_or("01"),
        rfc.get(8..10).unwrap_or("01"),
    )
}

/// api.md §7.1 suggested filename.
#[must_use]
pub fn suggested_filename(source_filenames: &[String], unix_ms: u64) -> String {
    let date = yyyymmdd_utc(unix_ms);
    match source_filenames {
        [] => format!("privacy-gate-0docs-redacted-{date}.pdf"),
        [one] => format!("{}-redacted-{date}.pdf", sanitize_stem(one)),
        many => format!("privacy-gate-{}docs-redacted-{date}.pdf", many.len()),
    }
}

/// Title for the PDF info dictionary: suggested filename without `.pdf`.
#[must_use]
pub fn title_from_filename(filename: &str) -> String {
    filename
        .strip_suffix(".pdf")
        .unwrap_or(filename)
        .to_string()
}

/// Assemble a newly rendered export PDF (architecture §11) with api.md §7.2 metadata.
#[must_use]
pub fn assemble_export_pdf(
    pages: &[RedactedPage],
    filename: &str,
    created_unix_ms: u64,
) -> Vec<u8> {
    let info = PdfExportInfo {
        title: title_from_filename(filename),
        created_unix_ms,
    };
    render_redacted_pages(pages, Some(&info))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_stem_collapses_punctuation_and_caps_length() {
        assert_eq!(sanitize_stem("letter.txt"), "letter");
        assert_eq!(sanitize_stem("My Letter (1).pdf"), "My-Letter-1");
        assert_eq!(sanitize_stem("..."), "document");
        let long = "a".repeat(50) + ".txt";
        assert_eq!(sanitize_stem(&long).chars().count(), 40);
    }

    #[test]
    fn suggested_filename_single_and_bundle() {
        let t = 1_777_200_000_000; // well after the specs were written
        let day = yyyymmdd_utc(t);
        assert_eq!(
            suggested_filename(&["letter.txt".into()], t),
            format!("letter-redacted-{day}.pdf")
        );
        assert_eq!(
            suggested_filename(&["a.txt".into(), "b.txt".into()], t),
            format!("privacy-gate-2docs-redacted-{day}.pdf")
        );
    }
}
