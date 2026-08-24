## A. Alignment design→SRS

- **Missing FR/NFR:**
  - **FR-1.4 (Global Retention Default & Config):** No component, data structure, or state is assigned to store, manage, or query global user configuration (including the paranoid "never retain originals" default).
  - **FR-4.6 & NFR-R2 (Irrevocable Deletion Flow):** Vault lists deletion as a responsibility, but no deletion interface, data flow, or state transition is specified in §3 or §4.
  - **FR-7.4 (Audit Trail Inspection):** §4 defines only event appending (`core → Audit Trail: Event append`). There is no query or read interface exposing audit events to the Frontend for inspection.
  - **FR-8.1 & FR-8.2 (Account & First-Run Setup):** No component in the Rust core is assigned responsibility for first-run account creation, passphrase initialization, or initial key generation.

- **Contradictions:**
  - **C-DES-1 vs. FR-2.2 / FR-3.1 (Review/Approve Data Flow):** C-DES-1 asserts that the Frontend receives only *"approved-but-not-redacted content for review"*. During the consent/approval step, the document is not yet approved; the Frontend must display unapproved text and detected sensitive spans so the user can make keep/redact decisions.
  - **§2.5 & §3.3 vs. FR-1.3 / FR-5.1 / FR-5.2 (Export after Discard):** When retention is set to "discard", `raw_bytes` is dropped. However, `ApprovedVersion` (§3.2) only contains `Vec<FieldDecision>` (spans/offsets) without document text or a rendered redacted artifact. The Share Engine cannot render an export file or extract approved text for Cloud AI once the original is discarded.
  - **§10 vs. Document Ownership (Circular Deferral):** §10 defers OQ-4 (single-document export format, bundle ordering) to *"design + UI specs"* and OQ-6 to *"design + testing specs"*, deferring design-owned requirements to the design spec itself.

- **Added behavior:**
  - **§7 (Arbitrary 25 MB Warning Boundary):** Introduces an unrequested 25 MB document size threshold with active warning behavior (*"Importer warns but still processes"*), which was not specified in FR-1.1 or NFR-PERF1.
  - **§3.4 (Explicit Variant Edit Prohibition):** Explicitly forbids variant editing (*"delete and save a new one"*). While a viable design simplification, it adds an operational constraint not specified in FR-5.5.

---

## B. Alignment design↔idea

- **Broken Core Loop on Discarded Originals:** In `idea.md` and `user-story.md` (Aisha's GP letter), discarding the original after redaction is a primary use case. Because the design's `ApprovedVersion` stores only span coordinates and discards `raw_bytes`, any subsequent export or AI share of a document whose original was discarded will fail for lack of underlying text/content.

---

## C. Design quality issues

- **§3.1 & §3.2 (ApprovedVersion Data Model Flaw):** `ApprovedVersion` does not store sanitized text, transformed document AST, or redacted output bytes. Once `Document.raw_bytes` is dropped, the model cannot support downstream sharing or AI reasoning.
- **§3.5 (OQ-16 Resolution Incomplete for Partial Overlaps):** The "innermost decision wins" rule only works for strictly nested spans. For non-nested, partially intersecting spans (e.g., Span A `[10..30]` = Redact, Span B `[20..40]` = Keep), neither is "innermost", leaving resolution on the overlapping range `[20..30]` undefined and untestable.
- **§3.4 (OQ-7 Variant Data Structure Missing):** §3.4 resolves variant lifecycle in prose but fails to define the internal data structure for `Variant` and `Override` in §3, leaving the storage format and override representation ambiguous.
- **§2.3 (Approval Engine State Isolation):** Approval Engine holds state *"for one document at a time"* without defining lifecycle states (e.g., active session, abort, multi-document queuing, or navigation transitions).
- **§2.5 & §3.4 (Multi-Document Share with Overrides):** Multi-document bundle export (FR-5.1) is supported, but variants and ephemeral overrides are modeled strictly per single `DocId`. The data structure representing a multi-document export request containing per-document override sets is missing.
- **§4 (Unidirectional Audit Trail Interface):** The Audit Trail interface is write-only in §4 (`Event append`). It lacks query/read methods (`get_events`, `filter_by_doc`) necessary to support the Audit Trail view and NFR-U1.

---

## D. Scope discipline

- **Leaked Architecture Decisions:**
  - §2.1, §2.2, and §2.7 mandate in-process execution topology and specific process memory lifetimes (*"dropped from process memory"*, *"held only in process memory"*), which belong in the Architecture Spec (OQ-17/memory architecture).
  - §3.1 specifies concrete Rust primitive types (`u64`, `u32`, `Vec<u8>`) rather than abstract component data structures.
- **Leaked UI Decisions:**
  - §7 specifies frontend UI rendering performance metrics (*"first paint ≤ 1 s"*), which belongs in the UI Specification.
- **Circular Deferrals:**
  - §10 defers OQ-4 (remainder) and OQ-6 to *"design + UI specs"* and *"design + testing specs"*, punting design-owned decisions instead of resolving them.

---

## E. Deferral health

- **OQ-3 (Audit-trail integrity mechanism):** Genuinely deferred to Architecture Spec. Design is coherent with an abstract tamper-evident sink.
- **OQ-4 remainder (Export format, ordering, naming):** **Not cleanly deferred.** Claimed deferred to *"design + UI specs"*, which is circular. Single-doc format (e.g., text vs. PDF) and bundle ordering must be resolved in design so Share Engine responsibilities are complete.
- **OQ-5 (Account network role):** Genuinely deferred to Architecture Spec regarding network sync/auth, but leaves a component ownership gap for local first-run account/key initialization.
- **OQ-6 ("No originals left device" semantics):** **Not cleanly deferred.** Claimed deferred to *"design + testing specs"*. The design emits the assertion on share events (§2.6) without defining what system state validates the assertion when originals are retained.
- **OQ-12 (Cloud AI auth):** Genuinely deferred to API and Architecture specs. Plugin Host interface boundary remains clean.
- **OQ-13 (Plugin security/sandbox):** Genuinely deferred to Architecture Spec. In-process first-party host does not preclude future WASM sandboxing.
- **OQ-14 (Retention default initial value):** Genuinely deferred to product decision. Design functions regardless of default value once a Config component is added.
- **OQ-17 (Transient plaintext handling):** Genuinely deferred to Architecture Spec (memory zeroization/locking).
- **OQ-18 (Key rotation / recovery):** Genuinely deferred to Architecture Spec. Vault relies only on abstract unlock key.

---

## F. Top 5 changes you would make

1. **Restructure `ApprovedVersion`:** Include sanitized document text / redacted document representation (or store a redacted intermediate document) so that exporting and AI sharing function when `raw_bytes` is discarded.
2. **Fix Pre-Approval Frontend Flow in C-DES-1:** Clarify that during the review/approval step, the Frontend receives the unapproved document structure and detected spans needed to render the consent UI.
3. **Complete OQ-16 for Partial Overlaps:** Specify deterministic precedence for non-nested, intersecting spans (e.g., "Redact wins on partial overlaps unless a sub-span is strictly nested and Kept").
4. **Add Configuration & Vault Lifecycle Interfaces:** Add a Configuration/Settings component (or state in Vault) for global retention defaults (FR-1.4), add Audit Trail read/query interfaces (FR-7.4), and specify the irrevocable deletion flow (FR-4.6).
5. **Eliminate Circular Deferrals (OQ-4 & OQ-6):** Specify single-document export formats and multi-doc bundle ordering directly in §2.5, and define the precise semantic condition under which the "no originals left device" audit assertion is generated.
