# Detector model artifacts

Privacy Gate's always-available detector is `pg-hybrid-v1` (architecture.md §10.1): the
UK pattern pack plus on-device NER. The NER artifact is **GLiNER-small-v2.1** (INT8
ONNX), shipped as `ner-pii.onnx` next to this file — or inside the Tauri app bundle
once packaging lands.

## Integrity pin (architecture §4.2)

Load-time check is SHA-256 of the exact bytes on disk. A mismatch is a **hard failure of
the NER stage**, never a network fetch. The pattern pack still runs.

The pin is recorded in two places that must agree:

| Where | Constant / file |
|---|---|
| Rust | `pg_core::detector::NER_PII_ONNX_SHA256` (`core/src/detector/hybrid.rs`) |
| This README | the hex digest below |

**Current pin:** *not recorded.* `NER_PII_ONNX_SHA256` is `None` because `ner-pii.onnx`
is not vendored in this tree (testing.md: PRs may skip heavy weights). When the file
lands:

1. `shasum -a 256 models/ner-pii.onnx`
2. Set `NER_PII_ONNX_SHA256` to that digest (`Some([...])`).
3. Replace this paragraph with the 64-character hex.
4. Nightly (`.github/workflows/nightly.yml`) will fail if the file and constant disagree.

Do not download weights at detection time. Do not re-pin silently on mismatch.

## Ollama tag pin (architecture §10.1.2)

Optional backend `pg-hybrid-ollama-v1` allowlists **`gemma4:e2b`**. The Ollama-reported
digest is `pg_core::detector::OLLAMA_GEMMA4_E2B_DIGEST` (`None` until a nightly golden
with a real daemon records it). `GEMMA4_E2B_CONTEXT_TOKENS` is likewise `None` until
that job verifies the tag's context window (architecture §10.1.5).

## Layout

```
models/
  README.md       (this file; tracked)
  ner-pii.onnx    (gitignored until explicitly vendored for nightly/release)
```
