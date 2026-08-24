# [0023] W13 — Pattern pack `pg-patterns-uk-v1`

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Ship the deterministic first stage of the hybrid detector: golden-string recognizers for
the identifier types architecture.md §10.1 pins (UK NI, sort code, account number, NHS,
email, phone, IBAN, Luhn card). Must hit testing.md §7.2's synthetic canaries, must not
treat PDF/JSON keywords as PII, and must not become `import_document`'s default (the W12
stub stays so AC-1..AC-4 cannot couple to pattern-pack behaviour).

Explicitly **not** in this chunk: ONNX (W15a), Ollama (W15b/W15c), `pg://detect-progress`
(W14), swapping `SessionManager`'s default detector.

## Implementation

### `core/src/detector/` — module split + `patterns_uk.rs`

`detector.rs` became `detector/mod.rs` so the pack can live beside the stub without
growing the W12 file. [`PatternsUkV1`] implements the same [`Detector`] trait; identity
string `pg-patterns-uk-v1`. Matching is per `TextSpan`, offsets shifted by
`span.byte_offset` — the same locatability contract as `StubDetector`.

NI is shape-only (`[A-Z]{2}\d{6}[A-D]`) so testing.md §7.2's `QQ123456C` is a hit; HMRC
prefix exclusions would have rejected it. NHS uses Mod-11; cards use Luhn. Added `regex`
to `pg-core`.

`SessionManager` still defaults to `StubDetector`. The pack is selected via
`with_detector` when a later host wants it.

## Resolution

- `core/tests/patterns_uk_w13.rs`: **9** tests — locatable goldens for NI `QQ123456C`,
  sort code `20-40-60`, 8-digit account; architecture §10.1 remaining types; Aisha-shaped
  triple in one document; PDF/JSON keywords empty; stub `PG-CANARY-` tokens empty; 8-digit
  account is not also NHS/card.
- Four checksum unit tests in `patterns_uk.rs` (NHS/Luhn accept the published test
  numbers, reject a one-digit flip).
- `cargo test -p pg-core` green including every W12 catalog/stub test unmodified.
- Clippy on `pg-core --all-targets` clean after replacing `sum % 10 == 0` with
  `is_multiple_of`.

Next: W14 — `pg://detect-progress`.

## Related Documentation

- [Development Plan — W13](../dev-plan.md#w13--pattern-pack-pg-patterns-uk-v1)
- [Agent roster — W13](../agent-roster.md)
- [Spec — architecture §10.1](../specs/architecture.md)
- [Spec — testing §7.2, §8 (pattern pack)](../specs/testing.md)
- [Dev log 0022 — W12 detector stub](./0022-w12-detector-stub.md)
