# [0004] API spec generation, Claude+Gemini review, and cross-doc alignment

- **Status:** Complete
- **Date:** 2026-08-23

## Objective

Produce the v1 Tauri 2 command/event surface (`docs/specs/api.md`) between the TypeScript
frontend and the Rust core. Resolve the API-owned remainders of OQ-4 (export filename + PDF
info dictionary) and OQ-12 (Cloud AI config commands). Review with Claude and Gemini only
(decision 0005; do not invoke Ollama). After the spec exists, walk every generated document
and make them say the same thing.

## Implementation

- Recorded decision 0005 (Claude + Gemini roster; 0001 method kept; Ollama not invoked).
- Drafted `docs/specs/api.md`: session gating including `degraded_integrity`, binary IPC for
  bytes, approval lifecycle, preview tokens, Cloud AI set/get/clear/test, export filename
  algorithm, capability ACL.
- Reviewed with Gemini (`agy --effort high`) and Claude (`claude -p`). Raw output:
  `docs/notes/reviews/api-gemini.md`, `docs/notes/reviews/api-claude.md`.
- Reconciled both reviews into the spec, then patched design.md / architecture.md /
  open-questions.md / indexes so parent docs match the API.
- Cross-verified idea → SRS → design → architecture → API → decisions 0001–0005.

## Problems Encountered

- **Approval `lifecycle` vs unresolved fields (both reviewers):** `"decided"` must mean
  `unresolved_field_ids` is empty; otherwise `submit_approval` is unimplementable.
- **Share-to-AI identity (Claude, Gemini):** FR-6.1 requires the previewed AI payload to be
  what is POSTed; `ai_instruction` must be on the request; audit stores
  `has_ai_instruction`, not the instruction text.
- **Span length (Gemini):** `DetectedFieldDto.span` needed `byte_length` to match design.md
  `TextSpan`.
- **Missing `get_variant` (Gemini):** list/save/delete without get left FR-5.5 incomplete.
- **JSON `number[]` for document bytes (Gemini):** 25 MB imports must use Tauri 2 binary IPC.
- **Re-approval after commit (Claude):** one canonical `ApprovedVersion` (decision 0002 / Q8)
  implies `already_approved`; later shares use overrides/variants.
- **Crypto in the API error table (Claude):** Argon2id/lockout rationale belongs in
  architecture, not `ApiError`.
- **Stale cross-links after draft:** parent specs still said “API not yet written,” OQ-12
  “command shape remains API spec,” OQ-4 naming still “UI/API,” and indexes omitted api.md.

## Resolution

- `set_field_decisions` returns `lifecycle: "decided"` iff `unresolved_field_ids` is empty.
- `ShareRequestDto.ai_instruction` required for share-to-AI (1..=4000 chars);
  `ai_payload_preview` is the approved body that `commit_share` POSTs (byte-identical for
  export PDFs). Plugin preamble wrapping is allowed; the preamble is not in the preview.
- `get_variant` added; span `byte_length` on the DTO and in design.md `TextSpan`.
- Bytes are `Vec<u8>` / `Uint8Array` over Tauri 2 binary IPC; originals inbound only on
  `import_document`.
- `open_approval` after a committed canonical version returns `already_approved`.
- Error table is machine codes only; KDF/lockout stay in architecture.
- Wire enums are snake_case; design PascalCase names map 1:1 (`Unlocked` → `"unlocked"`).
- OQ-4 API part and OQ-12 marked resolved; remainders are UI save-dialog, OQ-6 testing
  verification, and OQ-14 (product / `idea.md`).

Rejected: inventing OQ-14’s initial retention default; re-approval after commit; returning
originals to the webview; documenting Argon2id parameters in API errors.

## Verification (cross-doc alignment)

Walked idea.md, user-story.md, srs.md, design.md, architecture.md, api.md, decisions
0001–0005, open-questions.md, and area indexes.

**Aligned (no remaining contradiction):**

- Local-first vault; detection on-device; no off-device vault key (C-4 / C-ARCH-9); Cloud AI
  HTTP only from Rust with a user-supplied key in the vault (OQ-12).
- One canonical approved version; no v1 re-approval; ephemeral overrides + named variants;
  export = true-removal newly rendered PDF; multi-doc = one PDF bundle (0002).
- Account is local-only in v1 (OQ-5); idea.md’s “account for future backup/sync” is a later
  additive binding, not a v1 server.
- Idea.md “Gemma detected …” is illustrative; detector identity is `pg-hybrid-v1` (0004).
- No recovery in v1; passphrase change re-wraps the same master key (OQ-18).
- Audit: hash chain + HMAC, crash-window fast-forward, degraded session for FR-7.4 (OQ-3).
- Session gating: `degraded_integrity` cannot import/approve/share/read document content.
- Review roster going forward: Claude + Gemini only (0005). Historical Ollama reviews for SRS
  and design stay; architecture’s 429 gap is not backfilled.

**Patched in this wrap-up so the tree matches:**

- Indexes now list api.md, decision 0005, and this log.
- Open-questions: duplicate OQ-12 removed; OQ-4/OQ-12 status matches the specs.
- Design §3.7 / §8 / §9 no longer defer filename/metadata or OQ-5/OQ-3 as if still open.
- Architecture C-ARCH-2 / §12 allow the UI-spec save-dialog exception that api.md §8 already
  named, so “no filesystem” and “save previewed PDF” are one rule.

**Still open (intentional):**

- OQ-14 — initial retention default (`idea.md` amendment).
- OQ-4 remainder — save-dialog chrome → UI spec.
- OQ-6 remainder — independent verification of `no_originals_left_device` → testing spec.
- UI spec and testing spec not yet written.

## Lessons

- Parent specs that say “X remains the API spec” become stale the day api.md lands; the
  wrap-up pass has to rewrite those sentences, not only add a link in the index.
- Wire snake_case vs internal PascalCase looks like a contradiction unless one sentence
  states the mapping.
- “No filesystem in the webview” and “native save dialog for already-previewed bytes” need
  to be the same constraint in architecture and API, or implementers will pick one.

## Related Documentation

- [Spec — API](../specs/api.md)
- [Spec — architecture](../specs/architecture.md)
- [Spec — design](../specs/design.md)
- [Spec — SRS](../specs/srs.md)
- [Decision 0005 — Claude + Gemini review](../decisions/0005-review-claude-gemini.md)
- [Decision 0004 — v1 architecture](../decisions/0004-v1-architecture.md)
- [Decision 0001 — review method](../decisions/0001-multi-model-spec-review.md)
- [Open questions](../notes/open-questions.md)
- [Raw reviews](../notes/reviews/)
