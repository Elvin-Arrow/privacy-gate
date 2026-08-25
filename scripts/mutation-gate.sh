#!/usr/bin/env bash
# testing.md §5.3 / §5.6 — PR mutation gate.
#
# Explicit gated-file list (not "files changed on this branch"). Each shard
# passes `--lib` plus the colocated integration binary so cargo-mutants does
# not link every `core/tests/*.rs` target (that OOMs a 2 GiB Docker linker).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PACKAGE="pg-core"
MIN_TIMEOUT="${MUTANTS_MINIMUM_TEST_TIMEOUT:-30}"

if ! command -v cargo-mutants >/dev/null 2>&1; then
  echo "cargo-mutants is not on PATH. Install with:" >&2
  echo "  cargo install cargo-mutants --locked" >&2
  exit 1
fi

in_place=()
if [[ "${MUTANTS_IN_PLACE:-}" == "1" || "${CI:-}" == "true" ]]; then
  # Ephemeral CI checkout: reuse target/ instead of copying the tree per mutant.
  # Incompatible with --jobs; shards run sequentially inside one file.
  in_place+=(--in-place)
fi

run_file() {
  local file="$1"
  local examine_re="$2"
  shift 2
  local args=(
    cargo mutants
    -p "$PACKAGE"
    --minimum-test-timeout "$MIN_TIMEOUT"
    --file "$file"
    --cargo-arg --lib
    "${in_place[@]}"
  )
  if [[ -n "$examine_re" ]]; then
    args+=(--re "$examine_re")
  fi
  local t
  for t in "$@"; do
    args+=(--cargo-arg --test --cargo-arg "$t")
  done
  echo "+ ${args[*]}"
  "${args[@]}"
}

run_shard() {
  case "$1" in
    overlap)
      run_file core/src/overlap.rs "" overlap_w17 overrides_w26
      ;;
    export)
      run_file core/src/export.rs "" export_w23 oq6_w25
      ;;
    share)
      run_file core/src/share.rs "" share_w24 oq6_w25
      ;;
    audit)
      run_file core/src/audit.rs "" audit_w5
      ;;
    aad)
      run_file core/src/crypto/aad.rs "" crypto_w1
      ;;
    dek)
      run_file core/src/crypto/dek.rs "" crypto_w1
      ;;
    ollama)
      run_file core/src/detector/ollama.rs "" ollama_w15b
      ;;
    vault)
      run_file core/src/vault.rs \
        'open_raw_key_pragma|zeroize_key_material|overwrite_artifact_key_material|destroy_document_in_tx|verify_key' \
        vault_w3 delete_w20
      ;;
    session)
      run_file core/src/session.rs \
        'command_allowed|retention_override_forbidden' \
        session_gating_w4 retention_gate_w11
      ;;
    *)
      echo "unknown shard: $1" >&2
      echo "usage: $0 <overlap|export|share|audit|aad|dek|ollama|vault|session|all|nightly>" >&2
      exit 2
      ;;
  esac
}

PR_SHARDS=(overlap export share audit aad dek ollama vault session)

case "${1:-all}" in
  all)
    for shard in "${PR_SHARDS[@]}"; do
      run_shard "$shard"
    done
    ;;
  nightly)
    # testing.md §5.6: whole pg-core crate minus §5.4. Tauri shims live in
    # src-tauri (not this package). No include_bytes! model weights in pg-core.
    # Non-gated modules: S ≥ 0.70 after annotated equivalents (cargo-mutants
    # exits non-zero on missed mutants; review the JSON summary if this fails).
    echo "+ cargo mutants -p ${PACKAGE} --minimum-test-timeout ${MIN_TIMEOUT} ${in_place[*]:-}"
    cargo mutants -p "$PACKAGE" --minimum-test-timeout "$MIN_TIMEOUT" "${in_place[@]}"
    ;;
  *)
    run_shard "$1"
    ;;
esac
