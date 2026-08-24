.PHONY: help shell build test check clean

help:
	@echo "Privacy Gate development targets:"
	@echo "  make shell    - Open a shell in the dev container"
	@echo "  make build    - Build the Tauri app (debug)"
	@echo "  make test     - Run cargo test for the Rust core"
	@echo "  make check    - Run frontend typecheck (svelte-check)"
	@echo "  make clean    - Remove build artifacts"

shell:
	docker compose run --rm dev /bin/bash

build:
	docker compose run --rm dev npm run tauri build

test:
	docker compose run --rm dev cargo test

check:
	docker compose run --rm dev npm run check

clean:
	docker compose run --rm dev sh -c "cargo clean && rm -rf node_modules target"
