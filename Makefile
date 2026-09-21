# BeanAgent — Makefile (agents.md mục 3.3)
#
# Mục tiêu: `make check` là cổng chất lượng duy nhất của mỗi milestone.
SHELL := /bin/bash
CARGO ?= cargo
PNPM ?= pnpm
WEB_DIR := web
BIN := BeanAgent

.DEFAULT_GOAL := help

.PHONY: help types check-rust check-web check audit build build-headless e2e \
        fmt lint test dev-web run-chat clean

help: ## In danh sách target
	@grep -hE '^[a-zA-Z0-9_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

types: ## Sinh kiểu TypeScript từ Rust (ts-rs)
	@echo "==> make types: ts-rs được bật ở M9; hiện chưa có kiểu API nào để sinh"
	$(CARGO) test --workspace export_bindings

check-rust: ## cargo fmt + clippy + test toàn workspace
	$(CARGO) fmt --all --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) test --workspace

check-web: ## Biome + tsc + vitest + build production
	cd $(WEB_DIR) && $(PNPM) exec biome check .
	cd $(WEB_DIR) && $(PNPM) exec tsc --noEmit
	cd $(WEB_DIR) && $(PNPM) exec vitest run
	cd $(WEB_DIR) && $(PNPM) build

check: check-rust check-web ## Cổng chất lượng của mỗi milestone

audit: ## cargo audit + pnpm audit --prod (mục 15.10)
	@if command -v cargo-audit >/dev/null 2>&1; then \
		$(CARGO) audit; \
	else \
		echo "!! cargo-audit chưa cài — bỏ qua phần Rust (xem README/PROMPTS.md phần Chuẩn bị)"; \
	fi
	cd $(WEB_DIR) && $(PNPM) audit --prod

build: ## Build web -> build Rust release (nhúng UI, feature `ui`)
	cd $(WEB_DIR) && $(PNPM) build
	$(CARGO) build --release -p $(BIN)

build-headless: ## Build Rust release KHÔNG UI, không cần Node
	$(CARGO) build --release --no-default-features

e2e: ## Bộ test end-to-end (mục 20, làm ở M16)
	@echo "make e2e: chưa có (M16)"

fmt: ## Định dạng code
	$(CARGO) fmt --all
	cd $(WEB_DIR) && $(PNPM) exec biome check --write .

lint: ## Chỉ lint
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	cd $(WEB_DIR) && $(PNPM) exec biome check .

test: ## Chạy test
	$(CARGO) test --workspace
	cd $(WEB_DIR) && $(PNPM) exec vitest run

dev-web: ## Vite dev server (proxy /api -> 127.0.0.1:7878)
	cd $(WEB_DIR) && $(PNPM) dev

run-chat: ## REPL CLI (dùng FakeProvider/echo khi chưa có API key)
	$(CARGO) run -p $(BIN) -- chat

clean:
	$(CARGO) clean
	rm -rf $(WEB_DIR)/dist $(WEB_DIR)/coverage
