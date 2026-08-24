# [0014] W4 — Session gating table

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Replace the ad hoc per-command `if self.state() != X { ... }` checks in `core/src/session.rs`
with a single declarative table — [api.md §2](../specs/api.md)'s session-state ×
command-group matrix restated as data — and a table-driven test that walks every cell for
the commands that exist by the end of W3 ([`get_session_state`], `create_account`,
`unlock`, `lock`, `change_passphrase`, `get_account`).

Explicitly **not** in this chunk (dev-plan.md W4 "Do not: implement gated-but-unwritten
commands"): `get_integrity_report`, `list_audit_events`, and every document/approval/
share/config/variant command stay unregistered — they don't exist yet, so they get no row.

Per the [agent roster](../agent-roster.md), W4 is Sonnet, no mandatory second review
("state-machine logic against api.md, not novel crypto").

## Implementation

### `core/src/session.rs` — `SESSION_TABLE` and `command_allowed`

- `SESSION_TABLE: &[(&str, &[SessionState])]` — one row per registered command, listing
  the states api.md §2 allows it in. `get_session_state` deliberately has **no** row: api.md
  §2 says it's "callable in every state," i.e. it has no gate to encode.
- `command_allowed(command: &str, state: SessionState) -> bool` — looks the command up in
  the table; a name with no row returns `false` for every state. This is the literal
  mechanism behind dev-plan W4's "adding a command requires a new row in the table test
  (will fail until filled)": an unregistered command fails closed everywhere, not open.
- Every one of the six commands' gate checks now calls `command_allowed(name, self.state())`
  instead of comparing `SessionState` values inline. Where api.md documents a more specific
  error than the generic `not_in_session` for a disallowed cell — `create_account` →
  `account_exists` (api.md §3) — that mapping stays at the call site; the table only
  decides allowed/refused, not which error a refusal produces.
- `SessionState::DegradedIntegrity` has real rows in the table (`lock`, `get_account`
  allowed; `unlock`, `create_account`, `change_passphrase` refused) even though no code
  path in W2–W4 can put a live `SessionManager` into that state — that only becomes
  reachable in W5 (`SessionManager::verify_integrity_on_unlock`). Writing the table now
  means W5 only has to make the state reachable, not re-derive which commands accept it.
- Fixed a stale doc comment on `get_account` while touching it: it said "Unlocked-only",
  but api.md §2's row is `no | no | yes | yes (id + display_name only)` — `get_account` is
  also allowed in `degraded_integrity`. W2/W3's implementation already returns exactly
  `account_id`/`display_name`/`created_at` regardless of state, so there was nothing to
  change behaviourally, only the comment was wrong.

### `core/tests/session_gating_w4.rs` — the table-driven test

Two layers, because `degraded_integrity` isn't reachable end-to-end yet:

- `every_api_md_2_cell_is_covered` — a data-level test that transcribes api.md §2's table
  row by row and asserts `command_allowed` against all **20** cells (5 commands × 4
  states), including the three `degraded_integrity` cells no live session can reach.
- `an_unregistered_command_name_is_refused_in_every_state` — proves the fail-closed default
  for a command with no row.
- `get_session_state_has_no_gate_even_in_first_run` — the one command that must never be
  gated, confirmed end to end in the state where every other command is refused.
- 15 end-to-end tests, one per (command, reachable-state) pair, driving a real
  `SessionManager` through `create_account`/`lock`/`unlock` to reach `first_run`, `locked`,
  and `unlocked`, then asserting the actual `ApiError` — not just "some error", the specific
  code api.md §3 documents for that cell (`account_exists` vs. generic `not_in_session`).

## Resolution

- `cargo test -p pg-core` green: **18/18** new in `session_gating_w4.rs`, all pre-existing
  W1/W2/W3 tests (35 + 53 + 10 + 6 lib, 1 ignored) unmodified and green — the refactor
  changed how six gate checks decide "allowed or not," not what any of them return, so no
  existing test needed touching.
- Full workspace `cargo test` and `npm run check` both green.
- `cargo clippy -p pg-core --all-targets`: zero warnings on `session.rs` or
  `session_gating_w4.rs`.
- dev-plan W4 "Done when: adding a command requires a new row in the table test" — verified
  directly by `an_unregistered_command_name_is_refused_in_every_state` rather than only
  claimed: a command absent from `SESSION_TABLE` is refused in all four states today, and
  the same test will still pass (and still prove the point) once a real command is added
  without its row.
- Scope held: no `get_integrity_report`, no `list_audit_events`, no document/approval/
  share/config/variant command, no Tauri IPC, no UI.

Next: W5 — audit chain and integrity, which is the first thing to make
`SessionState::DegradedIntegrity` reachable through `SessionManager::unlock` — at which
point `every_api_md_2_cell_is_covered`'s `degraded_integrity` column stops being a
data-only assertion and gets end-to-end siblings alongside the other three states.

## Related Documentation

- [Development Plan — W4 specification](../dev-plan.md#w4--session-gating-table-commands-that-exist)
- [Agent roster — W4](../agent-roster.md)
- [Spec — API §2 (session model, the exact matrix this table encodes), §3 (error model)](../specs/api.md)
- [Spec — Testing "Session table" row (§6)](../specs/testing.md)
- [Dev log 0013 — W3 empty vault](./0013-w3-empty-vault.md)
