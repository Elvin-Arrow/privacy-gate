# Development Setup

Privacy Gate uses a **Docker-only** development environment. **Do not install Rust, Node, or Tauri CLI directly on your host machine.** All builds, tests, and project scaffolding happen inside a Docker container.

## Prerequisites

- Docker (running)
- `docker compose`
- `git`

## Quick Start

1. **Build the dev image:**
   ```bash
   docker compose build dev
   ```

2. **Open a shell in the container:**
   ```bash
   make shell
   ```

3. **Inside the container, run:**
   ```bash
   npm install        # First time only
   cargo test         # Run Rust core tests
   npm run check      # Run TypeScript frontend typecheck
   ```

## Available Make Targets

All commands below automatically run inside the dev container:

```bash
make shell          # Interactive shell in the dev container
make test           # cargo test for the Rust core (pg-core + privacy-gate crates)
make check          # npm run check for frontend typecheck
make build          # npm run tauri build (full app build)
make clean          # Remove build artifacts and node_modules
make help           # Show this list
```

## Running Tests Directly

You can also invoke Docker Compose directly:

```bash
docker compose run --rm dev cargo test
docker compose run --rm dev npm run check
docker compose run --rm dev npm install
```

## CI

The CI pipeline (`.github/workflows/ci.yml`) runs on GitHub Actions and installs its own native toolchain. The Docker-only constraint is local development on this machine, not CI runners.

## Project Structure

- `core/` — Rust core library (pg-core)
- `src-tauri/` — Tauri 2.x application code
- `src/` — Svelte 5 + Vite frontend
- `Dockerfile.dev` — Development container image definition
- `docker-compose.yml` — Docker Compose configuration
- `Makefile` — Build and test targets

## Notes

- The container mounts the repo at `/workspace` and caches Cargo registry and npm packages across runs.
- First `docker compose run` may take time as it builds the image; subsequent runs are fast due to volume caching.
- The frontend build outputs to `dist/` and is bundled by the Tauri app.
