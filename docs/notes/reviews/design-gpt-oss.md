## A. Alignment design → SRS  

**Missing FR / NFR**  
| ID | Requirement | Why missing / not covered |
|----|-------------|---------------------------|
| FR‑6.2 | *Ephemeral‑override warning wording / UI treatment* (pre‑share preview) | The design mentions a *preview artifact* but leaves the UI‑level warning entirely to the UI spec; no component‑level responsibility is assigned to emit the warning. |
| NFR‑R1 | *Tamper‑evident mechanism* (exact integrity method) | The design states the Audit Trail must be tamper‑evident but delegates the concrete mechanism to the Architecture Spec; a component‑level design should at least name the required interface (e.g., “append‑only log with hash chaining”). |
| FR‑8.1 | *Account bound to on‑device key model* (account lifecycle) | The design references an “account” only in passing (Vault holds the key). It does not describe the component that creates, stores, or validates the account binding, nor the events that must be logged (account‑create, account‑delete). |

**Contradictions** – none identified. All behaviours described in the design are either required by the SRS or explicitly deferred.

**Added behaviour (not required / beyond SRS)**  
| Behaviour | Comment |
|-----------|----------|
| “Supported OSes: macOS, Windows, Linux” | OS list is a design decision not called for by the SRS. |
| “Re‑import of a file creates a **new** `DocId` (no revision merge)” | SRS is silent on re‑import semantics; this rule could impact future user expectations. |
| “Variants cannot be edited – you must delete & recreate” | SRS only requires the ability to *save* a variant; forbidding edit is an extra constraint. |
| “Variant lifecycle stored encrypted alongside the approved version” | Storage location is a design choice not mandated. |
| “Export redaction is *true removal* (no overlay)” | The SRS requires that redacted fields never leave the device, but does not prescribe the specific removal technique; the design commits to a particular implementation. |

---

## B. Alignment design ↔ idea  

- **Variant persistence** – The idea doc describes “optionally save them as a named variant to reuse next time.” The design implements this (Section 3.4) – aligned.  
- **Ephemeral overrides** – Idea says “overrides shall be ephemeral… discarded after that share.” Design matches (FR‑5.4).  
- **Export bundle** – Idea: “export‑only … a single PDF bundle.” Design enforces a combined PDF bundle for multi‑doc shares – aligned.  
- **No OCR** – Both agree OCR is out of scope – aligned.  
- **Local‑first, no network for detection** – Design enforces in‑process detection – aligned.  
- **No hosting/relay/key‑off‑device** – Design respects that (Vault key lives on device, no network for vault access).  

**Result:** *clean* – no contradictions to the product idea.

---

## C. Design quality issues  

| Component / Section | Issue |
|----------------------|-------|
| **Importer → Vault hand‑off** | Responsibility for *dropping* the original when “discard” is chosen is split: Importer “drops from process memory” but Vault is said to “store if retain”. The exact point of destruction (Importer vs Vault) is ambiguous, risking a window where the raw bytes remain in memory. |
| **Audit Trail** | The spec states “tamper‑evident” but does not define the data model (log entry format, hash linking, signing). Without this, the component cannot guarantee the property. |
| **Variant data model** | Only a high‑level description is given; the concrete structure (which fields are overridden, how conflicts with canonical decisions are resolved) is missing, making it hard to verify that an override correctly supersedes the canonical version. |
| **Overlap handling** | The rule “innermost field wins” is declared, but the algorithm for determining “innermost” (e.g., length, nesting depth) is not specified. Ambiguity could lead to nondeterministic redaction. |
| **Share Engine → Frontend preview** | The design says a “preview artifact” is streamed, but does not clarify whether the preview is a full PDF, a raster image, or a JSON description. This affects both memory usage and the ability of the frontend to display it without exposing redacted text. |
| **Re‑import behaviour (OQ‑15)** | Treating every import as a brand‑new document may cause user confusion (duplicate entries). The spec does not describe any deduplication or user notification, a potential usability gap. |
| **Key handling** | Vault is said to “hold the on‑device key”, but the precise component that unlocks the key (e.g., separate *Key Manager* service) is omitted, leaving a blurry boundary between Vault and the (undeclared) security layer. |
| **Plugin Host sandboxing** | The design notes that third‑party sandboxing is “later”, yet the current host runs first‑party plugins *in‑process*. This could expose the core to crashes or security bugs from a buggy first‑party plugin, a risk not mitigated in the component model. |

---

## D. Scope discipline  

| Leaked decision / scope breach | Explanation |
|--------------------------------|-------------|
| **OS support list** (C‑DES‑7) – appears in Design but belongs to the *Architecture* or *Platform* spec. |
| **Key‑rotation / recovery** (OQ‑18) – mentioned as “architecture spec”, but the design still states “Vault provides irrevocable deletion”. The deletion guarantee depends on key‑rotation semantics, blurring the boundary. |
| **Encryption‑scheme details** (e.g., envelope‑encryption library) – referenced but not specified; belongs to the Crypto Architecture spec. |
| **Export format specifics** (single‑PDF bundle rule) – partially resolved here (bundle) while the rest (file naming, metadata, MIME type) is deferred to UI/API specs. This split is acceptable, but the design should explicitly note the deferral. |
| **Self‑contradiction** – none found where the design both claims to defer an OQ and resolves it. |
| **Silent resolution of OQ‑13 (plugin sandbox)** – The design *states* that third‑party sandboxing will be later but also says “v1 plugin runtime is in‑process”. This is a conscious decision, not a hidden resolution, and is properly flagged as a deferred OQ. |

---

## E. Deferral health  

| Deferred OQ | Is it genuinely deferred? | Does the design remain a coherent component‑level spec without it? |
|-------------|---------------------------|-------------------------------------------------------------------|
| OQ‑3 (audit‑trail integrity mechanism) | Yes – design only mentions “tamper‑evident”; concrete algorithm is left to Architecture Spec. | The Audit Trail component still has a clear responsibility (log events, expose query API). Absence of the integrity detail does not break the component model. |
| OQ‑5 (account network role) | Yes – design assumes account exists but does not describe any network calls; network behaviour is left to Architecture/API specs. | Component responsibilities (Vault holds key, Importer uses account only for creation) stay intact. |
| OQ‑13 (plugin security/sandbox) | Yes – first‑party plugins run in‑process; third‑party sandbox is deferred. | Plugin Host component is still defined (expose hooks, load first‑party plugin). |
| OQ‑12 (Cloud AI auth) | Yes – the design treats the Cloud AI plugin as a black box; authentication is not detailed. | Share Engine simply passes approved content; authentication details can be added later. |
| OQ‑14 (Retention‑default initial value) | Yes – the spec says “implements whichever default the product owner sets”. | No impact on component boundaries. |
| OQ‑17 (Transient plaintext handling) | Yes – referenced as architecture concern; design does not expose raw bytes beyond Importer → Vault flow. | Component flow is still valid. |
| OQ‑18 (Key rotation / recovery) | Yes – mentioned as architecture‑only. | Vault’s core responsibilities are unchanged. |
| OQ‑4‑remainder (single‑doc export format, ordering, naming) | Partially resolved: multi‑doc bundling is defined, but single‑doc format, ordering, naming are deferred. | The Share Engine can still produce an export; the unspecified aspects will be filled by UI/API specs. |
| OQ‑6 (“no originals left device” semantics) | Yes – design records a per‑share assertion; verification left to testing spec. | Share Engine’s behaviour is defined; the meaning of the assertion can be clarified later. |

All deferred items are properly marked and the design remains functional without their details.

---

## F. Top 5 changes you would make  

1. **Clarify the audit‑trail integrity contract** – add a minimal definition (e.g., each entry includes a SHA‑256 hash of the prior entry, signed by the vault key) so the component can be implemented without waiting for the Architecture Spec.  
2. **Make the “preview artifact” type explicit** – specify that the Share Engine returns a *redacted PDF* (or raster image) together with a *metadata manifest*; this removes ambiguity for the Frontend and UI specs.  
3. **Resolve the overlap‑resolution algorithm** – define “innermost” as the field with the smallest byte‑range length; document tie‑breaking (e.g., earlier detection order).  
4. **Move OS‑support declaration to a platform‑choice section** – keep the component spec pure; replace the inline “Supported OSes” note with a reference to the Platform Specification.  
5. **Document the exact hand‑off point for raw‑bytes disposal** – assign the responsibility for zero‑ising the original bytes to the Vault after it acknowledges receipt, and state that the Importer must immediately overwrite its buffer. This eliminates the current ambiguity and reduces the attack surface.