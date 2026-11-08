# Canonical entry points (see AGENTS.md). Run from the repo root.

CARGO      ?= cargo
PNPM       ?= pnpm
DEV_CONFIG := config/gargoyle.dev.toml
LOCAL_CONFIG ?= config/gargoyle.local.toml
# Release target triple. x86_64-unknown-linux-musl gives a static binary (see docs/DEPLOYMENT.md).
TARGET     ?= x86_64-unknown-linux-gnu
# cargo-deb is installed per user (`cargo install --locked cargo-deb`), never system-wide.
CARGO_DEB  ?= $(shell command -v cargo-deb 2>/dev/null || echo $(HOME)/.cargo/bin/cargo-deb)

.PHONY: check test fmt dev run-local web build release deb test-live web-deps

## check: everything that must be green before a hand-off
check: web-deps
	$(CARGO) fmt --all --check
	$(CARGO) clippy --all-targets -- -D warnings
	$(CARGO) test
	$(PNPM) -C web check
	$(PNPM) -C web test
	$(PNPM) -C web format:check
	$(PNPM) -C web build
	$(PNPM) -C web size

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

## release: optimized binary with the embedded UI, in target/$(TARGET)/release/gargoyle
release: web
	$(CARGO) build -p gargoyle --release --target $(TARGET)

## deb: Debian package from the release binary, in target/debian/
deb: release
	@test -x $(CARGO_DEB) || { echo "cargo-deb not found; install it with: cargo install --locked cargo-deb"; exit 1; }
	$(CARGO_DEB) -p gargoyle --no-build --target $(TARGET)

## test-live: #[ignore] smoke tests against the real local host
test-live:
	$(CARGO) test -- --ignored

# Installs web dependencies once; later runs reuse web/node_modules.
web-deps:
	@test -d web/node_modules || $(PNPM) -C web install --frozen-lockfile
