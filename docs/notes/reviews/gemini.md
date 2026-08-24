## A. Alignment SRS→idea
- **Missing**:
  - *Multi-document export bundling*: The idea doc/user story specifies exporting multiple approved documents as a single bundle for handoff to a person. FR-5.1 restricts export phrasing to a single document ("from the approved version").
  - *Audit logging of discarded originals*: The user story explicitly requires audit trail confirmation that un-retained originals were purged after approval. FR-7.2 captures the retention decision, but omits the confirmation event of original file destruction.
- **Contradictions**:
  - *Logic of "tightening" zero-retention (FR-1.4, AC-6)*: Stating that a global "never retain originals" default can be "tightened per-import, never loosened" is logically impossible—zero retention is already the strictest state.
  - *Account network dependency vs. offline guarantee (FR-8.1, FR-8.2 vs §2.1, §2.3)*: §2.3 lists network access for "account identity", conflicting with the core principle in §2.1 and FR-8.3 that vault initialization and usage require no network identity.
- **Scope creep**:
  - *Input quarantine mechanism (FR-1.2)*: Introducing a "quarantine" state adds unprompted system workflow complexity not present in the idea doc (which only specifies rejection/unsupported scope).
  - *Irrevocable destruction standard (FR-4.6, NFR-R2)*: Formalizes a technical "irrevocable deletion" mandate beyond the idea doc’s basic file deletion.

---

## B. Alignment idea→SRS (clarifications)
- **§10 adequacy**: Q1–Q8 identify valid gaps from the idea doc, particularly around export file formats (Q4), local vs. remote account boundaries (Q5), audit-trail integrity mechanisms (Q3), and variant lifecycle rules (Q7).
- **Missed clarifications**:
  - *Manual redaction fallback*: Neither document specifies what happens when the on-device model misses sensitive fields (false negatives) or misidentifies boundaries. The SRS fails to clarify whether users can manually define/draw redaction spans.
  - *Export sanitization method*: Fails to surface the requirement ambiguity between visual black-box overlay and actual underlying text stream scrubbing in exported PDFs.
  - *Cloud AI authentication/proxy*: Fails to clarify how the Cloud AI plugin authenticates against cloud LLMs if the app has no intermediary server and holds no keys off-device (C-4).
  - *Re-import collisions*: Fails to clarify system behavior when importing an updated version of an existing document.

---

## C. SRS quality issues
- **FR-1.2**: "Quarantine" is undefined, and "clear message" is subjective and untestable.
- **FR-1.4 / AC-6**: "Tightened per-import" under a "never retain" default is untestable and semantically invalid.
- **FR-2.2**: Dictates UI presentation ("highlighted span") rather than specifying data outputs (span offsets, bounding coordinates).
- **FR-4.4 / NFR-S3**: "A stolen data file shall be unusable" is untestable without a formal cryptographic threat model and cipher specification.
- **FR-4.6 / NFR-R2**: "Irrevocable" deletion lacks verification criteria (e.g., cryptographic key erasure vs. multi-pass block overwrite).
- **FR-5.1**: Does not specify that exported files must have underlying text streams sanitized, leaving redaction data leakage unaddressed.
- **FR-7.3**: "Verifiable answer... without requiring trust in the app on faith" is aspirational product marketing language, not a testable functional requirement.
- **FR-9.5 / NFR-E1**: "Without architectural rework" is subjective and untestable in a v1 acceptance test.
- **NFR-PERF1**: Untestable; explicitly defers interactive performance latency and document size thresholds to design.
- **NFR-R1**: "Not silently mutable in ways that defeat its verifiability purpose" is vague and provides no testable integrity standard (e.g., hash chaining, digital signatures).
- **NFR-U2**: "Comprehensible to a non-technical user" lacks measurable usability metrics (e.g., completion rates, error rates).

---

## D. Scope discipline
- **Minor design leakage**:
  - *UI presentation*: FR-2.2 specifies visual highlighting in the UI rather than functional span extraction.
  - *Architectural mechanism*: FR-4.1 specifies "envelope encryption" (an architectural design pattern) rather than stating the functional requirement of user-authenticated data protection at rest.
  - *State design*: FR-1.2 introduces an architectural "quarantine" state for invalid files.

---

## E. Top 5 changes you would make
1. **Add manual redaction fallback (FR-3.x)**: Add a functional requirement allowing users to manually create, resize, and delete redaction spans when automated detection fails.
2. **Mandate true PDF data stream sanitization (FR-5.1, NFR-S4)**: Explicitly require that exported files physically strip redacted text and metadata from file streams rather than applying visual-only overlays.
3. **Quantify performance and usability NFRs (NFR-PERF1, NFR-U2)**: Replace vague descriptors with testable bounds (e.g., model detection latency < 3s for documents ≤ 10MB; 90% unassisted task completion on export).
4. **Resolve offline account creation contradiction (FR-8.1, FR-8.2, §2.3)**: Explicitly specify whether account creation on first run functions 100% offline, removing the network dependency in §2.3.
5. **Support multi-document export bundling (FR-5.1)**: Expand FR-5.1 to support exporting multiple approved documents as a combined redacted bundle, aligning with the user story and FR-5.2.
