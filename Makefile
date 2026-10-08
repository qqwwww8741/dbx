.DEFAULT_GOAL := dev

PNPM ?= pnpm
TAURI_DEV_PORT ?= 1420

.PHONY: help install check-tauri-dev-port dev dev-fast dev-web dev-backend build package clean docs-build check test cargo-check-fast cargo-test-fast db db-list db-verify db-down db-reset db-check db-completion

export DB
export DB_VERSION
export DB_BIND_ADDRESS
export DB_PORT
export DB_PASSWORD
export FOLLOW
export CONFIRM

node_modules/.modules.yaml: package.json pnpm-lock.yaml
	$(PNPM) install --frozen-lockfile

help:
	@printf '%s\n' 'DBX development targets:'
	@printf '%s\n' ''
	@printf '%s\n' 'App:'
	@printf '  %-23s %s\n' 'make' 'Start the local desktop development environment'
	@printf '  %-23s %s\n' 'make dev' 'Start the local desktop development environment'
	@printf '  %-23s %s\n' 'make dev-fast' 'Start lightweight MySQL Tauri development'
	@printf '  %-23s %s\n' 'make dev-web' 'Start the web frontend development server'
	@printf '  %-23s %s\n' 'make dev-backend' 'Start the web backend development server'
	@printf '  %-23s %s\n' 'make build' 'Run type checks and build the desktop frontend'
	@printf '  %-23s %s\n' 'make package' 'Build the desktop app package'
	@printf '  %-23s %s\n' 'make clean' 'Remove local Rust build artifacts and caches'
	@printf '%s\n' ''
	@printf '%s\n' 'Docs:'
	@printf '  %-23s %s\n' 'make docs-build' 'Build the database documentation exporter'
	@printf '%s\n' ''
	@printf '%s\n' 'Checks:'
	@printf '  %-23s %s\n' 'make check' 'Run project checks'
	@printf '  %-23s %s\n' 'make test' 'Run project tests'
	@printf '  %-23s %s\n' 'make cargo-check-fast' 'Run Rust check without default features'
	@printf '  %-23s %s\n' 'make cargo-test-fast' 'Run Rust tests without default features'
	@printf '%s\n' ''
	@printf '%s\n' 'Database test environments:'
	@printf '  %-23s %s\n' 'make db-list' 'List available database versions'
	@printf '  %-23s %s\n' 'make db DB=mysql@8.4' 'Start and print DBX connection fields'
	@printf '  %-23s %s\n' 'make db-verify DB=mysql@8.4' 'Start and run smoke checks'
	@printf '  %-23s %s\n' 'make db-down DB=mysql@8.4' 'Stop an environment'
	@printf '  %-23s %s\n' 'make db-reset DB=mysql@8.4 CONFIRM=1' 'Delete containers and data'
	@printf '  %-23s %s\n' 'make db-check' 'Validate every recipe and Compose file'
	@printf '  %-23s %s\n' 'make db-completion' 'Show Bash/Zsh completion setup'
	@printf '%s\n' ''
	@printf '%s\n' 'Setup:'
	@printf '  %-23s %s\n' 'make install' 'Install root project dependencies'

install:
	$(PNPM) install --frozen-lockfile


ifeq ($(OS),Windows_NT)
check-tauri-dev-port:
	@powershell -NoProfile -Command "if (Get-NetTCPConnection -LocalPort $(TAURI_DEV_PORT) -State Listen -ErrorAction SilentlyContinue) { Write-Host 'Port $(TAURI_DEV_PORT) is already in use. DBX Tauri dev requires http://localhost:$(TAURI_DEV_PORT).'; Write-Host ''; Get-NetTCPConnection -LocalPort $(TAURI_DEV_PORT) -State Listen -ErrorAction SilentlyContinue | Format-Table LocalAddress,LocalPort,OwningProcess -AutoSize; Write-Host 'Stop the process above, then run make dev again.'; exit 1 }"
else
check-tauri-dev-port:
	@if lsof -nP -iTCP:$(TAURI_DEV_PORT) -sTCP:LISTEN >/dev/null 2>&1; then \
		echo "Port $(TAURI_DEV_PORT) is already in use. DBX Tauri dev requires http://localhost:$(TAURI_DEV_PORT)."; \
		echo ""; \
		lsof -nP -iTCP:$(TAURI_DEV_PORT) -sTCP:LISTEN; \
		echo ""; \
		echo "Stop the process above, then run make dev again. Example: kill <PID>"; \
		exit 1; \
	fi
endif

dev: node_modules/.modules.yaml check-tauri-dev-port
	$(PNPM) dev:tauri

dev-fast: node_modules/.modules.yaml check-tauri-dev-port
	# os-keyring must stay in sync with src-tauri defaults: without it the keychain
	# key is unreadable and the secret-store migration wizard reappears on every launch.
	RUST_MIN_STACK=16777216 $(PNPM) dev:tauri -- --no-default-features --features sqlite-bundled,os-keyring

dev-web: node_modules/.modules.yaml
	$(PNPM) dev:web

dev-backend: node_modules/.modules.yaml
	$(PNPM) dev:backend

build: node_modules/.modules.yaml
	$(PNPM) build:checked

package: node_modules/.modules.yaml
	$(PNPM) tauri build

clean:
	cargo clean

docs-build: node_modules/.modules.yaml
	$(PNPM) build:docs-export

check: node_modules/.modules.yaml
	$(PNPM) check

test: node_modules/.modules.yaml
	$(PNPM) test

cargo-check-fast:
	cargo check --workspace --all-targets --no-default-features --features dbx-core/sqlite-bundled

cargo-test-fast:
	cargo test -p dbx-driver-mysql -p dbx-sql-core -p dbx-sql-data -p dbx-sql-dialect -p dbx-sql-schema -p dbx-types --lib

db-list:
	@$(PNPM) db:env -- list

db:
	@$(PNPM) db:env -- start

db-verify:
	@$(PNPM) db:env -- verify

db-down:
	@$(PNPM) db:env -- down

db-reset:
	@$(PNPM) db:env -- reset

db-check:
	@$(PNPM) db:env -- check

db-completion:
	@$(PNPM) db:env -- completion
