# Architecture spec review — qwen-3.5

Reviewer intended: `qwen3.5:cloud` via Ollama, per decision 0001.

**Cloud not obtained.** Same Ollama Cloud 429 session-usage limit as gpt-oss
(ref `eb7765d1-e2d2-4ed5-8a54-ae0ad44842a7`).

**Local substitute attempted:** `qwen3.5:9b`. The model did not follow the review
prompt. Apparent cause: context/stdin truncation — its chain-of-thought treated
the input as starting at architecture.md §12 ("Commands that accept document
bytes…") and produced a summary of §§13–20 instead of sections A–G. The raw
local output is not a valid architecture review and is not reproduced here.

Reconciliation proceeded from the Gemini review plus an author implementability
pass. This file exists so the review set is complete and the gap is visible.
