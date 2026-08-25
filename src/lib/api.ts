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
  | 'retention_policy_unset'
  | 'retention_loosen_forbidden'
  | 'unsupported_document'
  | 'already_approved'
  | 'approval_busy'
  | 'approval_bad_state'
  | 'not_approved'
  | 'preview_expired'
  | 'variant_name_conflict'
  | 'cloud_ai_network'
  | 'cloud_ai_refused'
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

// ---------------------------------------------------------------------------
// W32 — api.md §5.3 (import and catalog)
// ---------------------------------------------------------------------------

/** data-model / catalog.rs: `"pdf" | "text"` (core `SourceFormat`,
 * `#[serde(rename_all = "snake_case")]`). Only the subset needed to render a row; unknown
 * values still satisfy `string`. */
export type SourceFormat = 'pdf' | 'text' | (string & {})

/** `EffectiveRetention` (core/src/session.rs): only two values ever land on a document —
 * `never_retain` is a *global default* concept (config.rs `RetentionPolicy`), never a
 * per-document `retention` value (data-model §6.1: "Import under never_retain always
 * writes retention: discard here"). */
export type EffectiveRetention = 'retain' | 'discard'

/** `DocumentSummary` (api.md §5.3 / core/src/session.rs). No span text, no field labels —
 * `detected_field_count` is a number only. */
export interface DocumentSummary {
  doc_id: string
  /** Basename only (api.md §5.3). */
  source_filename: string
  source_format: SourceFormat
  /** RFC 3339 UTC. */
  imported_at: string
  retention: EffectiveRetention
  has_approved_version: boolean
  has_retained_original: boolean
  detected_field_count: number
}

/** `import_document` In (api.md §5.3). `retention_override: null` means "use the global
 * default". */
export interface ImportDocumentIn {
  filename: string
  bytes: number[]
  retention_override: EffectiveRetention | null
}

export interface ImportDocumentOut {
  summary: DocumentSummary
  over_budget: boolean
}

export function importDocument(input: ImportDocumentIn): Promise<ImportDocumentOut> {
  return invoke('import_document', { input })
}

export interface ListDocumentsOut {
  documents: DocumentSummary[]
}

export function listDocuments(): Promise<ListDocumentsOut> {
  return invoke('list_documents')
}

export interface GetDocumentOut {
  summary: DocumentSummary
}

export function getDocument(docId: string): Promise<GetDocumentOut> {
  return invoke('get_document', { input: { doc_id: docId } })
}

export interface DeleteDocumentOut {
  ok: true
}

export function deleteDocument(docId: string): Promise<DeleteDocumentOut> {
  return invoke('delete_document', { input: { doc_id: docId } })
}

/** `pg://detect-progress` payload (api.md §6). `phase` is additive (decision 0009); only
 * `fraction` is rendered in v1 (ui.md §7.2: "show `{ fraction }` as a determinate bar"). */
export interface DetectProgressEvent {
  doc_id: string
  fraction: number
  phase: 'detecting' | 'warming_model'
}

export const DETECT_PROGRESS_EVENT = 'pg://detect-progress'

// ---------------------------------------------------------------------------
// W33 — api.md §5.4 (approval / consent)
// ---------------------------------------------------------------------------

/** api.md §4 `FieldDecisionKind`. `#[serde(rename_all = "snake_case")]` on the core
 * enum (`KeepVisible` → `"keep_visible"`). */
export type FieldDecisionKind = 'keep_visible' | 'redact'

/** api.md §5.4 `ApprovalLifecycle` (the two values a live view can carry; submit/abort
 * Out use `"committed"` / `"aborted"`). */
export type ApprovalLifecycle = 'awaiting_decisions' | 'decided' | 'committed' | 'aborted'

/** api.md §4 `DetectedFieldDto.span`. `text` is present on approval commands only
 * (C-API-2). */
export interface DetectedFieldSpanDto {
  byte_offset: number
  byte_length: number
  text: string | null
  page_index: number
}

/** api.md §4 `DetectedFieldDto`. */
export interface DetectedFieldDto {
  id: string
  label: string
  classification: string
  span: DetectedFieldSpanDto
  parent_field_id: string | null
}

/** api.md §4 `FieldDecisionDto`. */
export interface FieldDecisionDto {
  field_id: string
  decision: FieldDecisionKind
}

/** One page of `ApprovalView` (api.md §5.4 / `session::ApprovalPage`). */
export interface ApprovalPage {
  page_index: number
  spans: ApprovalPageSpan[]
}

export interface ApprovalPageSpan {
  byte_offset: number
  text: string
  page_index: number
}

/** `open_approval` / `get_approval_view` Out (api.md §5.4). */
export interface ApprovalView {
  approval_session_id: string
  doc_id: string
  lifecycle: ApprovalLifecycle
  pages: ApprovalPage[]
  fields: DetectedFieldDto[]
}

export function openApproval(docId: string): Promise<ApprovalView> {
  return invoke('open_approval', { input: { doc_id: docId } })
}

export function getApprovalView(approvalSessionId: string): Promise<ApprovalView> {
  return invoke('get_approval_view', { input: { approval_session_id: approvalSessionId } })
}

export interface SetFieldDecisionsOut {
  lifecycle: ApprovalLifecycle
  unresolved_field_ids: string[]
}

export function setFieldDecisions(
  approvalSessionId: string,
  decisions: FieldDecisionDto[],
): Promise<SetFieldDecisionsOut> {
  return invoke('set_field_decisions', {
    input: { approval_session_id: approvalSessionId, decisions },
  })
}

export interface SubmitApprovalOut {
  summary: DocumentSummary
  lifecycle: ApprovalLifecycle
}

export function submitApproval(approvalSessionId: string): Promise<SubmitApprovalOut> {
  return invoke('submit_approval', { input: { approval_session_id: approvalSessionId } })
}

export interface AbortApprovalOut {
  lifecycle: ApprovalLifecycle
}

export function abortApproval(approvalSessionId: string): Promise<AbortApprovalOut> {
  return invoke('abort_approval', { input: { approval_session_id: approvalSessionId } })
}

// ---------------------------------------------------------------------------
// W34 — api.md §5.6 (share preview / commit, person-export)
// ---------------------------------------------------------------------------

/** api.md §4 `ShareKind`. */
export type ShareKind = 'export_to_person' | 'share_to_ai'

/** api.md §4 `ShareRequestDto`. Maps are JSON objects (Rust `HashMap`). */
export interface ShareRequestDto {
  kind: ShareKind
  doc_ids: string[]
  per_doc_overrides: Record<string, FieldDecisionDto[]>
  applied_variant_ids: Record<string, string>
  recipient_note: string | null
  ai_instruction: string | null
}

/** api.md §5.6 `manifest[]`. Field **ids** only — no span text (C-API-2). */
export interface ShareManifestEntry {
  doc_id: string
  visible_field_ids: string[]
  redacted_field_ids: string[]
}

/** `preview_share` Out. `pdf_bytes` is `Vec<u8>` on the wire (number[] under JSON invoke). */
export interface SharePreview {
  preview_token: string
  expires_at: string
  kind: ShareKind
  overrides_in_effect: boolean
  suggested_filename: string | null
  pdf_bytes: number[] | null
  ai_payload_preview: string | null
  manifest: ShareManifestEntry[]
  no_originals_left_device: boolean[]
}

export function previewShare(request: ShareRequestDto): Promise<SharePreview> {
  return invoke('preview_share', { input: { request } })
}

/** `commit_share` Out. Export fills `pdf_bytes` + `suggested_filename`; AI fills `output_text`. */
export interface CommitShareOut {
  kind: ShareKind
  pdf_bytes: number[] | null
  suggested_filename: string | null
  output_text: string | null
  audit_event_id: number
}

export function commitShare(previewToken: string): Promise<CommitShareOut> {
  return invoke('commit_share', { input: { preview_token: previewToken } })
}

// ---------------------------------------------------------------------------
// W35 — api.md §5.8 (audit list)
// ---------------------------------------------------------------------------

/** api.md §4 `EventType`. */
export type EventType =
  | 'import'
  | 'detect'
  | 'approve'
  | 'share'
  | 'discard_original'
  | 'delete'

/** api.md §5.8 `AuditEventDto`. `payload` is ids/labels only — never span text (C-API-2). */
export interface AuditEventDto {
  sequence: number
  event_type: EventType
  doc_id: string | null
  produced_at: string
  no_originals_left_device: boolean | null
  payload: Record<string, unknown>
}

/** `list_audit_events` In. `limit` default on the core is 50; 1..=200. */
export interface ListAuditEventsIn {
  doc_id: string | null
  event_type: EventType | null
  after_sequence: number | null
  limit: number
}

/** `list_audit_events` Out. */
export interface ListAuditEventsOut {
  events: AuditEventDto[]
  next_sequence: number | null
}

export function listAuditEvents(input: ListAuditEventsIn): Promise<ListAuditEventsOut> {
  return invoke('list_audit_events', { input })
}

// ---------------------------------------------------------------------------
// W36 — api.md §5.5 (variants)
// ---------------------------------------------------------------------------

/** One row of `list_variants` / `save_variant` Out. */
export interface VariantSummary {
  variant_id: string
  name: string
  created_at: string
}

export interface ListVariantsOut {
  variants: VariantSummary[]
}

export function listVariants(docId: string): Promise<ListVariantsOut> {
  return invoke('list_variants', { input: { doc_id: docId } })
}

export interface GetVariantOut {
  variant_id: string
  name: string
  created_at: string
  overrides: FieldDecisionDto[]
}

export function getVariant(docId: string, variantId: string): Promise<GetVariantOut> {
  return invoke('get_variant', { input: { doc_id: docId, variant_id: variantId } })
}

export interface SaveVariantIn {
  doc_id: string
  name: string
  overrides: FieldDecisionDto[]
}

export function saveVariant(input: SaveVariantIn): Promise<VariantSummary> {
  return invoke('save_variant', { input })
}

export function deleteVariant(docId: string, variantId: string): Promise<{ ok: true }> {
  return invoke('delete_variant', { input: { doc_id: docId, variant_id: variantId } })
}
