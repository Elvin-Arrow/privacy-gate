.PHONY: help build test check test-ui mutants clean

help:
	@echo "Privacy Gate development targets:"
	@echo "  make build    - Build the Tauri app (debug)"
	@echo "  make test     - Run cargo test for the Rust core"
	@echo "  make check    - Run frontend typecheck (svelte-check)"
	@echo "  make test-ui  - Run frontend unit tests (Vitest)"
	@echo "  make mutants  - PR mutation gate (testing.md §5.3; needs cargo-mutants)"
	@echo "  make clean    - Remove build artifacts"

build:
	npm run tauri build

test:
	cargo test

check:
	npm run check

test-ui:
	npm run test

mutants:
	command -v cargo-mutants >/dev/null || cargo install cargo-mutants --locked
	CARGO_BUILD_JOBS=1 ./scripts/mutation-gate.sh all

clean:
	cargo clean && rm -rf node_modules target
