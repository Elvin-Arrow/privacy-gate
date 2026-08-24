## A. Alignment SRS → idea  

| Category | Details |
|----------|---------|
| **Missing from SRS** | • **Export file format** – idea doc (user story) says “export as a single PDF bundle”. The SRS only says “produce a redacted file (export)” (FR‑5.1) without specifying the expected PDF‑bundle format or that multiple documents may be merged.  <br>• **Definition of “source identity”** – FR‑1.5 records a “source identity” of the document, a notion not present in the idea doc and not defined elsewhere. <br>• **Variant lifecycle** – idea mentions “named variant to reuse next time” but SRS does not state how variants are stored, edited or deleted. |
| **Contradictions** | None that change product intent. The only tension is **FR‑4.1‑4.2** explicitly calls out *envelope encryption*; the idea only says “encrypted at rest”. This is a design‑level detail, not a functional conflict. |
| **Scope creep** | • **Audit‑trail integrity model** (NFR‑R1) – the SRS adds a durability / non‑mutability guarantee that the idea only describes as “a live audit trail”. The guarantee is reasonable but extends the idea’s functional description. <br>• **Explicit “no network identity required”** (FR‑8.3) – the idea says “no network identity is required day‑to‑day” but the SRS restates it as a requirement; still within scope. No other extra functionality is introduced. |

---

## B. Alignment idea → SRS (clarifications)

| Item | Assessment |
|------|------------|
| **§10 adequacy** | The eight questions (Q1‑Q8) correctly surface the most salient ambiguities left by the idea doc (OS support, performance thresholds, audit‑trail integrity, export format, account network role, meaning of “no private originals left the device”, variant lifecycle, multiple approved versions). |
| **Missed clarifications** | 1. **Export format & bundling rules** (PDF single‑file, ordering, naming). <br>2. **Exact semantics of “ephemeral” overrides** (lifetime, persistence across sessions). <br>3. **Tamper‑evidence mechanism for the audit trail** (cryptographic hash chain, signed log, etc.). <br>4. **Variant management** (creation, edit, delete, scope to a document or globally). <br>5. **Handling of overlapping or nested detected fields** (which decision wins?). <br>6. **Definition of “source identity”** (file path, user‑provided label, intrinsic metadata). <br>7. **Whether any temporary plaintext is ever written to disk** (e.g., during detection). <br>8. **Policy for key rotation / recovery** (future‑proofing). |

---

## C. SRS quality issues  

| ID | Problem |
|----|---------|
| **FR‑1.5** | *Ambiguous*: “source identity of the document” not defined → untestable. |
| **FR‑3.2 / FR‑5.5** | *Potential conflict*: “exactly one canonical approved version” vs. “named variants” – need clear rule that variants are *overrides*, not separate approved versions. |
| **FR‑5.4** | *Ephemeral* not quantified – how long does an override persist? (session, until next share, until user discards?) |
| **FR‑5.5** | No acceptance criteria for variant creation, deletion, or reuse. |
| **FR‑6.2** | UI wording (“clearly marked”) is vague; testability depends on design spec. |
| **FR‑7.2 (bullet 6)** | “Confirmation that no private originals left the device (where applicable)” – how is this proven? Needs measurable check. |
| **NFR‑PERF1** | No concrete performance metric → untestable at requirements level. |
| **NFR‑R1** | “Not silently mutable in ways that defeat its verifiability purpose” – needs concrete integrity guarantee (e.g., append‑only hash chain). |
| **NFR‑U2** | “Comprehensible to a non‑technical user” – vague; requires usability testing criteria. |
| **FR‑9.5** | Architecture constraint appears in a requirements document; might belong to design docs. |
| **FR‑4.1‑4.2** | Mention of “envelope encryption” is a design choice, not a pure requirement; could be considered scope leakage. |
| **FR‑8.1** | “Account bound to an on‑device key model for unlock‑credential binding and future backup/sync” – the future backup part is outside v1 scope, making the requirement partially out‑of‑scope. |

---

## D. Scope discipline  

- **Leaked design decisions**:  
  *Envelope encryption* (FR‑4.1‑4.2) and the *append‑only audit‑trail* idea implied in NFR‑R1 are design‑level specifics not required by the idea doc.  
- **Otherwise**: The SRS stays at the “what” level; no UI mock‑ups, API signatures, or architectural component names are exposed.  

**Verdict:** Minor leakage (encryption method, audit‑trail integrity model) but overall disciplined.

---

## E. Top 5 changes you would make  

1. **Add an explicit functional requirement for the export format** – e.g., “FR‑5.1a The exported file shall be a single PDF bundle containing the selected documents in the order chosen by the user.”  
2. **Define “ephemeral override” lifetime** – a requirement such as “FR‑5.4a Ephemeral overrides persist only for the current share operation and are automatically discarded after the export is completed.”  
3. **Specify variant lifecycle** – requirements covering creation, storage (encrypted), editing, and deletion (e.g., FR‑5.5a–d).  
4. **Replace “envelope encryption” with a higher‑level requirement** – “All stored content shall be encrypted at rest using a cryptographically strong algorithm; the exact scheme is a design decision.”  
5. **Clarify audit‑trail integrity** – add NFR‑R1a “The audit trail shall be cryptographically signed or hash‑chained to detect any post‑creation modification.” and define how this is verified in acceptance criteria.  

These changes close the identified gaps, remove design leakage, and make the SRS fully testable against the idea doc.