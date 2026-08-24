// Thin typed wrapper over the Tauri IPC surface this chunk (W30) consumes
// (api.md §5.1). Field names are the exact wire shapes: pg-core's DTOs derive
// `Serialize`/`Deserialize` with no `rename_all = "camelCase"` on the structs (only the
// `SessionState` enum uses `rename_all = "snake_case"`, which matches its own variant
// names anyway), so JSON keys stay snake_case — do not camelCase these fields.

import { invoke } from '@tauri-apps/api/core'

/** api.md §2: `SessionState = "first_run" | "locked" | "unlocked" | "degraded_integrity"`. */
export type SessionState = 'first_run' | 'locked' | 'unlocked' | 'degraded_integrity'

/** api.md §3: stable, machine-readable error codes. Only the subset W30 can see listed
 * explicitly; unknown codes still satisfy the `string` wire type. */
export type ErrorCode =
  | 'not_in_session'
  | 'invalid_input'
  | 'unlock_failed'
  | 'account_exists'
  | 'internal'
  | (string & {})

/** api.md §3: `ApiError { code, message }`. `message` is always a non-secret, fixed class
 * string (core/src/api.rs) — safe to render verbatim. */
export interface ApiError {
  code: ErrorCode
  message: string
}

/** Type guard: `invoke` rejects with the raw `ApiError` object (Tauri serializes the `Err`
 * value as the rejection reason), not an `Error` instance. */
export function isApiError(value: unknown): value is ApiError {
  return (
    typeof value === 'object' &&
    value !== null &&
    'code' in value &&
    'message' in value &&
    typeof (value as { message: unknown }).message === 'string'
  )
}

export interface SessionStateOut {
  state: SessionState
}

export interface CreateAccountIn {
  display_name: string
  passphrase: string
}

export interface CreateAccountOut {
  account_id: string
  /** Always `"unlocked"` (api.md §5.1). */
  state: SessionState
}

export interface UnlockIn {
  passphrase: string
}

/** api.md §5.1: `integrity` is non-null iff `state === "degraded_integrity"`. */
export interface UnlockOut {
  state: SessionState
  integrity: IntegrityReport | null
}

export interface LockOut {
  /** Always `"locked"`. */
  state: SessionState
}

export interface IntegrityReport {
  ok: boolean
  kind: 'ok' | 'crash_window_fast_forwarded' | 'truncation' | 'modification'
  head_sequence: number
  tail_sequence: number
  first_bad_sequence: number | null
}

export function getSessionState(): Promise<SessionStateOut> {
  return invoke('get_session_state')
}

export function createAccount(input: CreateAccountIn): Promise<CreateAccountOut> {
  return invoke('create_account', { input })
}

export function unlock(input: UnlockIn): Promise<UnlockOut> {
  return invoke('unlock', { input })
}

export function lock(): Promise<LockOut> {
  return invoke('lock')
}

export function getIntegrityReport(): Promise<IntegrityReport> {
  return invoke('get_integrity_report')
}

/** `pg://session-changed` payload (src-tauri/src/commands.rs `emit_session_changed`):
 * identical shape to `get_session_state`'s Out. */
export const SESSION_CHANGED_EVENT = 'pg://session-changed'
