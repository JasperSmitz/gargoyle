# Canonical entry points (see AGENTS.md). Run from the repo root.

CARGO      ?= cargo
PNPM       ?= pnpm
DEV_CONFIG := config/gargoyle.dev.toml
LOCAL_CONFIG ?= config/gargoyle.local.toml

.PHONY: check test fmt dev run-local web build release test-live web-deps

## check: everything that must be green before a hand-off
check: web-deps
	$(CARGO) fmt --all --check
	$(CARGO) clippy --all-targets -- -D warnings
	$(CARGO) test
	$(PNPM) -C web check
	$(PNPM) -C web test
	$(PNPM) -C web format:check

## test: Rust and web tests only
test: web-deps
	$(CARGO) test
	$(PNPM) -C web test

## fmt: format Rust and web sources
fmt: web-deps
	$(CARGO) fmt --all
	$(PNPM) -C web format

## dev: backend in mock mode + Vite dev server (proxies /api to 127.0.0.1:9077)
# The binary is started directly (not via `cargo run`) so the trap can stop it when Vite exits.
dev: web-deps
	$(CARGO) build -p gargoyle
	@target/debug/gargoyle --config $(DEV_CONFIG) & pid=$$!; \
	trap 'kill $$pid 2>/dev/null' EXIT INT TERM; \
	$(PNPM) -C web dev

## run-local: real Linux collector on this machine (read-only); override LOCAL_CONFIG=...
run-local:
	$(CARGO) run -p gargoyle -- --config $(LOCAL_CONFIG)

## web: build the frontend into web/dist
web: web-deps
	$(PNPM) -C web build

## build: debug binary with the embedded UI
build: web
	$(CARGO) build -p gargoyle

## release: optimized binary with the embedded UI
release: web
	$(CARGO) build -p gargoyle --release

## test-live: #[ignore] smoke tests against the real local host
test-live:
	$(CARGO) test -- --ignored

# Installs web dependencies once; later runs reuse web/node_modules.
web-deps:
	@test -d web/node_modules || $(PNPM) -C web install --frozen-lockfile
