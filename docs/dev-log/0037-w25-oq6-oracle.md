# [0037] W25 — OQ-6 egress oracle

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Independent verification that redacted canaries do not leave in export bytes. Do not trust
`no_originals_left_device` alone. Cloud AI body checks wait for W27; ephemeral-override
AC-2 is W26.

## Implementation

- Test harness `core/tests/common/oracle.rs`: UTF-8 / UTF-16 scans, `pdf-extract` text,
  inflate of `/FlateDecode` streams.
- Self-test plants `PG-CANARY-REDACT-7F3A` in raw bytes and in a zlib stream; both must
  fail the oracle.
- W24 person-export commit path now runs the oracle (`PG-CANARY-X1` absent, `X2` present).

## Resolution

- `core/tests/oq6_w25.rs` + updated `share_w24.rs`.
- `cargo test -p pg-core --test oq6_w25 --test share_w24` green; clippy clean.

Next: W26 — ephemeral overrides + variants on share.

## Related Documentation

- [Development Plan — W25](../dev-plan.md#w25--oq-6-egress-oracle)
- [Spec — testing.md §7](../specs/testing.md)
- [Dev log 0036 — W24 share export](./0036-w24-share-export.md)
