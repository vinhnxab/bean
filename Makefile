# Bean — Makefile (agents.md mục 3.3)
#
# Mục tiêu: `make check` là cổng chất lượng duy nhất của mỗi milestone.
SHELL := /bin/bash
CARGO ?= cargo
PNPM ?= pnpm
WEB_DIR := web
BIN := bean

.DEFAULT_GOAL := help

.PHONY: help types check-rust check-web check-brand check audit build build-headless e2e smoke-scheduler \
        fmt lint test dev-web run-chat clean

help: ## In danh sách target
	@grep -hE '^[a-zA-Z0-9_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

types: ## Sinh và kiểm tra kiểu TypeScript từ Rust (ts-rs)
	$(CARGO) test --workspace export_bindings --locked
	git diff --exit-code -- web/src/api/generated

check-rust: ## cargo fmt + clippy + test toàn workspace + kiểm tra kiểu sinh
	$(CARGO) fmt --all --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) test --workspace --locked
	$(MAKE) types

check-web: ## Biome + tsc + vitest + build production
	cd $(WEB_DIR) && $(PNPM) exec biome check .
	cd $(WEB_DIR) && $(PNPM) exec tsc --noEmit
	cd $(WEB_DIR) && $(PNPM) exec vitest run
	$(MAKE) check-brand
	cd $(WEB_DIR) && $(PNPM) build

# Tài nguyên thương hiệu là **sinh ra**, nên phải chống trường hợp commit ảnh cũ
# còn nằm trong repo: đổi `bean.png` mà quên chạy `pnpm brand` thì mọi thứ vẫn
# xanh, chỉ có mắt người thấy — cùng lớp lỗi với favicon bị cắt tai trước đây.
# Sinh lại rồi `git diff --exit-code` y hệt cách `make types` chặn kiểu TS lệch.
check-brand:
	cd $(WEB_DIR) && $(PNPM) brand
	cd $(WEB_DIR) && $(PNPM) gen:favicon
	git diff --exit-code -- web/public

check: check-rust check-web ## Cổng chất lượng của mỗi milestone

# RUSTSEC-2026-0173 là cảnh báo unmaintained của aquamarine, chỉ nằm trong
# teloxide 0.17 (latest hiện tại); không phải vulnerability và không có bản sửa.
# Giữ ignore tường minh để `make audit` không báo pass giả.

audit: ## cargo audit (strict) + pnpm audit --prod
	@set -e; \
	if command -v cargo-audit >/dev/null 2>&1; then \
		$(CARGO) audit -D warnings --ignore RUSTSEC-2026-0173; \
	elif command -v cargo-deny >/dev/null 2>&1; then \
		$(CARGO) deny check advisories; \
	else \
		echo "cần cài cargo-audit hoặc cargo-deny để chạy audit Rust" >&2; \
		exit 1; \
	fi
	cd $(WEB_DIR) && $(PNPM) audit --prod

build: ## Build web -> build Rust release (UI embedded, locked)
	cd $(WEB_DIR) && $(PNPM) build
	$(CARGO) build --release --locked -p $(BIN) --features ui

build-headless: ## Build Rust release without UI, no Node required
	$(CARGO) build --release --locked --no-default-features

e2e: ## Binary E2E: login -> chat -> confirm -> Stop -> reconnect Sync
	$(CARGO) test --locked -p $(BIN) --test m16_e2e -- --nocapture
	$(CARGO) test --locked -p bean-channels allowlist_blocks_unknown_user_without_reply -- --nocapture

smoke-scheduler: ## M16 scheduler smoke: 1 virtual hour with 1ms tick
	$(CARGO) test --locked -p bean-core --test scheduler one_hour_scheduler_smoke_with_fast_tick -- --nocapture


fmt: ## Định dạng code
	$(CARGO) fmt --all
	cd $(WEB_DIR) && $(PNPM) exec biome check --write .

lint: ## Chỉ lint
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	cd $(WEB_DIR) && $(PNPM) exec biome check .

test: ## Chạy test
	$(CARGO) test --workspace --locked
	cd $(WEB_DIR) && $(PNPM) exec vitest run

dev-web: ## Vite dev server (proxy /api -> 127.0.0.1:7878)
	cd $(WEB_DIR) && $(PNPM) dev

run-chat: ## REPL CLI (dùng FakeProvider/echo khi chưa có API key)
	$(CARGO) run -p $(BIN) -- chat

clean:
	$(CARGO) clean
	rm -rf $(WEB_DIR)/dist $(WEB_DIR)/coverage
