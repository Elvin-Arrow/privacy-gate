## A. Alignment design→SRS
- **Missing FR/NFR:**
    - **FR-7.2 (Audit Confirmation):** SRS requires recording "confirmation that no private originals left the device". Design §2.6 records an "assertion" and defers semantics/verification to OQ-6 (Testing/Design). The design does not specify *how* the confirmation is derived (e.g., validation logic), only that it is logged.
    - **FR-4.1/4.2 (Variant Storage):** SRS FR-5.5 allows saving variants. Design §3.4 states variants are "stored encrypted alongside the approved version". However, Design §2.4 (Vault Responsibilities) and §4 (Interfaces) only specify storing/retrieving `ApprovedVersion`. Variant persistence is missing from Vault responsibilities and interfaces.
    - **FR-6.1 (Preview):** SRS requires pre-share preview. Design §2.8 says "preview is rendered by the core". Design §2.5 (Share Engine) lists "render... to a redacted file" (Export) but does not explicitly list "Generate preview artifact" in responsibilities, though §4 Interfaces implies it.
- **Contradictions:** None.
- **Added behavior:** None.

## B. Alignment design↔idea
- **clean**

## C. Design quality issues
- **Audit Trail (§2.6):** Responsibility says "tamper-evident" but data structure (§3) and interfaces (§4) do not define the mechanism (e.g., hash chains, signatures). Deferring the *mechanism* entirely to Architecture Spec (OQ-3) makes this component unimplementable from this spec alone. It must define the *data fields* required for integrity (e.g., `prev_entry_hash`).
- **Variant Persistence (§3.4 vs §2.4/§4):** §3.4 defines variants as stored encrypted artifacts. §2.4 (Vault) and §4 (Interfaces) do not include Variant storage/retrieval operations. Share Engine cannot save/load variants without Vault support.
- **Preview Responsibility (§2.5 vs §2.8):** §2.8 states core renders preview; §4 Interfaces assigns this to Share Engine. §2.5 Share Engine responsibilities omit preview generation, listing only export rendering. This creates ambiguity on which component handles FR-6.1.
- **Plugin Host Content (§2.7 vs §2.5):** §2.5 says Share Engine applies overrides/variants before sharing. §2.7 Plugin Host interface says it receives "Approved content only". It should specify "Approved content (including applied overrides/variants)" to ensure redacted fields are not leaked via plugin interface.
- **Overlap Resolution (§3.5):** The rule "innermost... wins" is deterministic, but the data structure `DetectedField` (§3.1/3.2) does not explicitly include a `nesting_level` or `parent_id` to facilitate this logic. Implementation detail missing.

## D. Scope discipline
- **Plugin Runtime (§2.7):** States "v1 runtime hosts first-party code in-process". This resolves part of OQ-13 (Plugin security/sandbox), which is listed as Architecture Spec ownership in §5 and §10. While acceptable as a v1 constraint, it borders on leaking an architecture decision (trust model) into the design spec.
- **Performance Budgets (§7):** Resolves OQ-2. This is appropriate for Design Spec (implementation constraints), not API/Arch/UI/Test.
- **Clean:** No leaked API commands, UI layouts, or test plans.

## E. Deferral health
- **OQ-3 (Audit Integrity):** **At Risk.** Genuinely deferred to Arch Spec, but the Design Spec claims the "tamper-evident" responsibility without defining the supporting data structure. The component design is incoherent without the Arch Spec's mechanism definition.
- **OQ-6 (Originals Left Device):** **Partially Resolved.** Design records an "assertion" but defers verification semantics to Testing Spec. Design remains coherent but cannot fully satisfy FR-7.2 ("confirmation") until verification logic is added.
- **OQ-13 (Plugin Sandbox):** **Healthy.** Defers third-party sandbox to Arch Spec; constrains v1 to in-process first-party. Design functions without the third-party resolution.
- **OQ-1, 2, 7, 15, 16:** **Resolved.** Resolutions are specific and testable within this spec.
- **OQ-5, 12, 14, 17, 18:** **Healthy.** Genuinely deferred; design functions with stubs/local-only assumptions.

## F. Top 5 changes you would make
1.  **Audit Trail Data Structure:** Add `prev_entry_hash` and `entry_signature` (or placeholder) fields to the Audit Trail entry structure in §3 to support the "tamper-evident" responsibility without waiting for the Arch Spec's crypto primitive selection.
2.  **Vault Interface:** Update §2.4 (Vault Responsibilities) and §4 (Interfaces) to explicitly include `StoreVariant` and `LoadVariant` operations, aligning with §3.4.
3.  **Share Engine Responsibilities:** Add "Generate redacted preview artifact (ephemeral)" to §2.5 Share Engine responsibilities to match §4 Interfaces and satisfy FR-6.1 unambiguously.
4.  **Overlap Data Model:** Update `DetectedField` in §3.1/3.2 to include `span_hierarchy` or `parent_field_id` to support the overlap resolution logic defined in §3.5.
5.  **Originals Verification:** In §2.6 (Audit Trail) or §2.5 (Share Engine), specify the logic for the "no originals left" assertion (e.g., "Share Engine validates `Document.retention_status == discarded` before emitting share event") to move from unverified assertion to SRS-compliant confirmation.