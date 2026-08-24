## A. Alignment SRS→idea
- **Missing:** None significant. All core flows (Import, Detect, Approve, Store, Share, Audit, Plugin hooks) are mapped.
- **Contradictions:** 
    - **§10 Q4 vs User Story:** SRS §10 Q4 states export format is open for clarification, yet `docs/user-story.md` (listed as source of truth in SRS §1.4) explicitly states "exports them as a single PDF bundle". The SRS should reflect this as a requirement (FR-5.1), not a clarification.
    - **FR-9.5/NFR-E1 vs SRS Scope:** SRS §1 Introduction states architecture is produced separately. FR-9.5 and NFR-E1 mandate architectural outcomes ("without rework"), contradicting the SRS's own scope definition.
- **Scope creep:** 
    - **FR-1.2:** "Quarantine" is an implementation strategy not specified in the idea doc (which only says OCR is out of scope). 
    - **FR-9.5/NFR-E1:** Architectural constraints regarding future third-party plugin rework belong in an Architecture Specification, not Requirements.

## B. Alignment idea→SRS (clarifications)
- **§10 adequacy:** 
    - **Q4 is inadequate:** As noted in Section A, this should be resolved based on the User Story, not left for design. 
    - **Q1, Q2, Q3, Q5, Q6, Q7, Q8:** Appropriate gaps surfaced for design/spec resolution.
- **Missed clarifications:** 
    - **Plugin Security Model:** Idea doc requires architecture to support future third-party plugins "without rework." SRS does not clarify if v1 must include sandboxing/signing infrastructure to meet this, or if that is deferred. This is a critical architectural dependency.
    - **Retention Default Value:** Idea doc says "A global default sets this retention policy" but does not specify the *initial* value (Retain vs. Discard). User Story implies "Retain," but product default is undefined.
    - **AI Plugin Auth:** Idea doc says AI plugin sends content to cloud. SRS does not clarify how API keys/auth are managed in v1 (user-provided vs. bundled).

## C. SRS quality issues
- **FR-2.2:** "highlighted span" is UI design, not requirement. Should be "identified to the user".
- **FR-6.1:** "show the user a preview" is UI design. Should be "provide mechanism to review content before share".
- **FR-6.2:** "clearly marked" is subjective/untestable. Needs specific warning mechanism definition.
- **FR-9.5:** "without rework" is untestable in v1.
- **NFR-PERF1:** "interactive" is subjective/untestable. Needs latency thresholds (e.g., "<5s for 10pg PDF").
- **NFR-R1:** "not silently mutable in ways that defeat its verifiability" is ambiguous. Needs specific integrity mechanism (e.g., "append-only log with hash chain").
- **NFR-U2:** "comprehensible to a non-technical user" is untestable without usability testing criteria.
- **NFR-E1:** "without architectural rework" is untestable/architectural.
- **AC-4:** "confirm no redacted field left the device" is difficult to verify via acceptance testing without specific tooling requirements defined.

## D. Scope discipline
- **UI Leaks:** FR-2.2 ("highlighted"), FR-6.1 ("preview"), FR-6.2 ("marked"), FR-8.2 ("First run shall let the user...").
- **Architecture Leaks:** FR-9.5 ("architecture shall not preclude"), NFR-E1 ("without architectural rework").
- **Implementation Leaks:** FR-1.2 ("quarantine"), FR-4.4 ("stolen data file").
- **Verdict:** Not clean. Significant leakage of UI and architectural constraints into requirements.

## E. Top 5 changes you would make
1. **Resolve §10 Q4:** Update FR-5.1 to mandate "PDF bundle" export per `docs/user-story.md` and remove from Clarifications.
2. **Remove Architectural Constraints:** Delete FR-9.5 and NFR-E1; move to Architecture Specification or rephrase as functional capability ("System shall support external plugin modules").
3. **Make NFRs Testable:** Replace subjective terms in NFR-PERF1 ("interactive"), NFR-R1 ("verifiable"), and NFR-U2 ("comprehensible") with measurable criteria or reference specific usability test plans.
4. **Sanitize UI Language:** Rewrite FR-2.2, FR-6.1, FR-6.2, and FR-8.2 to describe user capabilities/outcomes rather than UI elements ("highlighted", "preview", "First run").
5. **Add Plugin Security Requirement:** Add a requirement specifying how plugin integrity is managed in v1 (e.g., "All plugins shall be digitally signed") to ensure the "future third-party" goal doesn't require security model rework.