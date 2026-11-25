# HomeServer Assistant OS — build automation
#
# Every target works with plain `cargo`/`pnpm`; this file only encodes the
# supported paths. Linux is required for ISO and Incus targets.
#
# Run `make help` for the list of targets.

SHELL := /bin/bash
CARGO ?= cargo
PNPM ?= pnpm
CARGO_FLAGS ?= --offline
DATA_DIR ?= ./.hs-data
ADMIN_BIND ?= 127.0.0.1:8080
KIOSK_BIND ?= 127.0.0.1:3000
STATIC_DIR ?= ui/dist
ISO_OUTPUT ?= iso/output/homeserver-assistant-amd64.iso

.DEFAULT_GOAL := help

.PHONY: help
help: ## Show this help
	@grep -E '^[a-zA-Z0-9_-]+:.*?## ' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

.PHONY: build
build: build-daemon build-vision build-ui ## Build all artifacts

.PHONY: build-daemon
build-daemon: ## Build the hs-daemon release binary
	$(CARGO) build --release $(CARGO_FLAGS) -p hs-daemon

.PHONY: build-vision
build-vision: ## Build the hs-vision release binary
	$(CARGO) build --release $(CARGO_FLAGS) -p hs-vision

.PHONY: build-ui
build-ui: ## Build the React SPA into ui/dist
	cd ui && $(PNPM) install && $(PNPM) build

.PHONY: check
check: ## Type-check the whole workspace (fast)
	$(CARGO) check --workspace --all-targets $(CARGO_FLAGS)

# ---------------------------------------------------------------------------
# Quality
# ---------------------------------------------------------------------------

.PHONY: fmt
fmt: ## Format Rust sources
	$(CARGO) fmt --all

.PHONY: fmt-check
fmt-check: ## Verify Rust formatting
	$(CARGO) fmt --all -- --check

.PHONY: clippy
clippy: ## Lint Rust sources with warnings as errors
	$(CARGO) clippy --workspace --all-targets $(CARGO_FLAGS) -- -D warnings

.PHONY: test
test: test-rust test-ui check-assets ## Run every automated check

.PHONY: test-rust
test-rust: ## Run Rust unit and integration tests
	$(CARGO) test --workspace $(CARGO_FLAGS) --lib --tests

.PHONY: test-ui
test-ui: ## Type-check and unit-test the UI
	cd ui && $(PNPM) typecheck && $(PNPM) test

.PHONY: check-assets
check-assets: ## Validate infra/ISO templates and repository assets
	python3 scripts/check-assets.py

.PHONY: lint
lint: fmt-check clippy ## Formatting + clippy (warnings are errors)

# ---------------------------------------------------------------------------
# Development
# ---------------------------------------------------------------------------

.PHONY: run-dev
run-dev: ## Run the daemon in fake/dev mode (no root, no Incus)
	$(CARGO) run $(CARGO_FLAGS) -p hs-daemon -- serve --dev \
		--data-dir $(DATA_DIR) \
		--bind $(ADMIN_BIND) \
		--kiosk-bind $(KIOSK_BIND) \
		--static-dir $(STATIC_DIR)

.PHONY: ui-dev
ui-dev: ## Run the Vite dev server
	cd ui && $(PNPM) dev

.PHONY: reset-dev
reset-dev: ## Remove the development data directory
	rm -rf $(DATA_DIR)

# ---------------------------------------------------------------------------
# Appliance
# ---------------------------------------------------------------------------

.PHONY: iso
iso: ## Build the installable ISO (Linux host with xorriso)
	bash iso/build-iso.sh --output $(ISO_OUTPUT)

.PHONY: test-vm
test-vm: ## Boot the ISO in QEMU and run smoke tests (Linux host)
	bash iso/test-vm.sh --iso $(ISO_OUTPUT)

.PHONY: clean
clean: ## Remove Rust and UI build artifacts
	$(CARGO) clean
	rm -rf ui/dist ui/node_modules
