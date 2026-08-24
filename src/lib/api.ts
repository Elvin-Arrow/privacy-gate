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
  | 'passphrase_mismatch'
  | 'cloud_ai_not_configured'
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

// ---------------------------------------------------------------------------
// W31 — api.md §5.1 (account/passphrase), §5.2 (retention), §5.7 (Cloud AI config)
// ---------------------------------------------------------------------------

/** `get_account` Out (api.md §5.1). */
export interface GetAccountOut {
  account_id: string
  display_name: string
  /** RFC 3339 UTC. */
  created_at: string
}

export function getAccount(): Promise<GetAccountOut> {
  return invoke('get_account')
}

/** `change_passphrase` In (api.md §5.1). */
export interface ChangePassphraseIn {
  current: string
  new_passphrase: string
}

/** `change_passphrase` Out: always `{ ok: true }`; failures are `ApiError`s. */
export interface ChangePassphraseOut {
  ok: boolean
}

export function changePassphrase(input: ChangePassphraseIn): Promise<ChangePassphraseOut> {
  return invoke('change_passphrase', { input })
}

/** data-model / config.rs: `"retain" | "discard" | "never_retain"` (snake_case on the wire
 * — `RetentionPolicy` derives `#[serde(rename_all = "snake_case")]`). */
export type RetentionPolicy = 'retain' | 'discard' | 'never_retain'

/** `get_retention_default` Out, and `set_retention_default` Out (api.md §5.2 — identical
 * shape; `confirmed` is always `true` on the `set` response). */
export interface RetentionDefaultOut {
  policy: RetentionPolicy
  confirmed: boolean
}

export function getRetentionDefault(): Promise<RetentionDefaultOut> {
  return invoke('get_retention_default')
}

export function setRetentionDefault(policy: RetentionPolicy): Promise<RetentionDefaultOut> {
  return invoke('set_retention_default', { input: { policy } })
}

/** `cloud_ai_set_config` In (api.md §5.7). The frontend may hold `api_key` only until this
 * command returns — never store it in component state beyond the call itself. */
export interface CloudAiSetConfigIn {
  endpoint_url: string
  model: string
  api_key: string
}

/** `cloud_ai_set_config` Out. Never `api_key`. */
export interface CloudAiSetConfigOut {
  configured: boolean
  endpoint_host: string
  model: string
  key_last4: string
}

export function cloudAiSetConfig(input: CloudAiSetConfigIn): Promise<CloudAiSetConfigOut> {
  return invoke('cloud_ai_set_config', { input })
}

/** `cloud_ai_get_config` Out. Never `api_key`. */
export interface CloudAiGetConfigOut {
  configured: boolean
  endpoint_url: string | null
  endpoint_host: string | null
  model: string | null
  key_last4: string | null
}

export function cloudAiGetConfig(): Promise<CloudAiGetConfigOut> {
  return invoke('cloud_ai_get_config')
}

export interface CloudAiClearConfigOut {
  configured: boolean
}

export function cloudAiClearConfig(): Promise<CloudAiClearConfigOut> {
  return invoke('cloud_ai_clear_config')
}

/** `cloud_ai_test` Out. Sends no vault document content (api.md §5.7). */
export interface CloudAiTestOut {
  ok: boolean
  error_class: string | null
}

export function cloudAiTest(): Promise<CloudAiTestOut> {
  return invoke('cloud_ai_test')
}
