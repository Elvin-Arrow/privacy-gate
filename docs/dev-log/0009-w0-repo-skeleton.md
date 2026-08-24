# [0009] W0 — Repo skeleton

- **Status:** Complete
- **Date:** 2026-08-23

## Objective

Scaffold the Privacy Gate v1 monorepo with Tauri 2 + Rust core + Svelte 5/Vite webview. Deliver a Docker-only dev environment, CI workflow, production CSP string, and Tauri capability JSON per the specs. Establish compile-level tests and typecheck pass gates for W1+.

## Implementation

### Docker-only dev environment
- **`Dockerfile.dev`**: Ubuntu 22.04 image with Rust stable, Node 20+, and all Tauri 2 Linux build prerequisites (libwebkit2gtk-4.1-dev, libgtk-3-dev, libayatana-appindicator3-dev, librsvg2-dev, patchelf, libxdo-dev, build-essential, pkg-config, libssl-dev, file, curl, wget).
- **`docker-compose.yml`**: Single `dev` service mounting repo root at `/workspace`, with cached volumes for cargo registry and npm packages. No host install of Rust/Node/Tauri required or expected.
- **`Makefile`**: Targets `shell`, `test`, `check`, `build`, `clean` all routed via `docker compose run --rm dev`.

### Project structure
- **`Cargo.toml`** (root): Workspace with members `["core", "src-tauri"]`.
- **`core/`**: Rust library crate `pg-core` v0.1.0 with one smoke test (`assert_eq!(2+2, 4)`).
- **`src-tauri/`**: Tauri 2.x binary crate `privacy-gate` with minimal `main.rs` entry point (no commands yet).
- **`src/`**: Svelte 5 + Vite frontend. `App.svelte` makes a stub call to `get_session_state` (returns error until W2) to verify Tauri IPC plumbing.
- **TypeScript config**: `tsconfig.json` and `tsconfig.node.json` for strict type checking.
- **Svelte/Vite config**: `svelte.config.js`, `vite.config.ts` for development and build pipelines.
- **`package.json`**: Pinned to Svelte 5, Vite 5, @sveltejs/vite-plugin-svelte 4, TypeScript 5, @tauri-apps/api 2, @tauri-apps/plugin-dialog 2.

### Configuration
- **`src-tauri/tauri.conf.json`**: Production CSP string from ui.md §3.1 copied verbatim:
  ```
  default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' blob: data:; media-src 'none'; connect-src 'self' ipc: http://ipc.localhost; frame-src blob:; object-src 'none'; base-uri 'none'; form-action 'none';
  ```
  Denies external fonts, analytics, service worker. Allows `blob:` for redacted PDF preview only; no `https:` in connect-src (Cloud AI is Rust-only in W27+).

- **`src-tauri/capabilities/default.json`**: Tauri capability file per api.md §8 and ui.md §3.2:
  - **Grant**: `core:default`, `core:event:allow-listen` (for events — will be wired in W2), `dialog:allow-save`, `fs:allow-write` (for saving preview bytes).
  - **Deny**: `fs:allow-read`, `fs:allow-read-dir`, `http:default`, `shell:*`, `dialog:allow-open`.
  - Note: No individual command grants yet (W0 has no commands; grants land as each command is implemented in W2+).

### CI workflow
- **`.github/workflows/ci.yml`**: Two jobs on `ubuntu-latest`:
  - **`test`**: Installs system deps, Rust stable, Node 20; runs `cargo test` on core, `npm run check` on frontend.
  - **`build`**: Runs after test; builds frontend to `dist/` as a sanity check (unused until Tauri dev/release builds are gated).
  - No native Rust/Node on host (constraint noted in docstring).

### Documentation
- **`CONTRIBUTING.md`**: Explicit statement: "Do not install Rust, Node, or Tauri CLI directly. All builds/tests happen inside Docker." Cites the three core Make targets (`make test`, `make check`, `make shell`) and notes that CI runners have their own native toolchain (OK; constraint is local-only).
- **`Makefile`**: `make help` prints available targets; all invoke `docker compose run --rm dev <cmd>`.

### Tests
- **`core/src/lib.rs`**: Smoke test `it_works` (add 2+2=4) already present from `cargo new`.
- **Frontend**: `App.svelte` invokes `get_session_state` IPC; error handling in place (command doesn't exist until W2, error is expected).
- Both `cargo test` and `npm run check` pass locally inside the container.

### Notes on scaffold details
- Icons placeholder files created (minimal 1×1 PNGs) to satisfy Tauri's build.rs validation.
- `dist/` directory created with `.gitkeep` as required by tauri.conf.json.
- Workspace profiles warning in cargo (profiles on src-tauri, not root) — harmless; can be moved to root in a later refactor if noisy.
- SVG icon assets not yet in repo; icon bundling deferred to later design phase (W37+).

## Problems Encountered

1. **Tauri config field mismatch**: Initial tauri.conf.json used old kebab-case field names (`devPath` instead of `devUrl`). Fixed to camelCase per Tauri 2 spec.
2. **CSP and capability format**: Early drafts conflated CSP string (which goes in tauri.conf.json security.csp) with capability grant/deny (separate JSON file). Spec clarified; both now in place.
3. **Icon validation**: `tauri-build` proc macro fails on missing icon files. Created minimal valid PNG placeholders.
4. **Svelte plugin version skew**: svelte-check and @sveltejs/vite-plugin-svelte@3 incompatible with Svelte 5.56+. Bumped to @4, which fixed diagnostics.
5. **npm package resolution**: `@tauri-apps/cli` and related @tauri-apps packages didn't resolve with caret ranges. Locked to exact major versions (2).

## Resolution

All four objectives met:
- ✅ Docker image builds, dev container mounts repo, all prerequisites present.
- ✅ `make test` and `make check` pass locally.
- ✅ CI workflow added; runs on ubuntu-latest.
- ✅ CSP string and capability JSON in source; reviewed against specs (fixture tested in W38 mutation gate, not yet).

Repo is ready for W1. Next: envelope crypto primitives (HKDF, AEAD, AAD, DEK lifecycle).

## Related Documentation

- [Development Plan — W0 specification](../dev-plan.md#w0--repo-skeleton)
- [Decision 0003 — Tech stack](../decisions/0003-v1-tech-stack.md)
- [Decision 0008 — Svelte frontend](../decisions/0008-frontend-svelte.md)
- [Spec — Architecture §12 (IPC/capabilities)](../specs/architecture.md)
- [Spec — API §8 (Tauri capability allowlist)](../specs/api.md)
- [Spec — UI §3 (CSP and capabilities)](../specs/ui.md)
- [CONTRIBUTING.md](../../CONTRIBUTING.md)
