#!/usr/bin/env bash
# Commit + push Giai đoạn 2 (hot-swap provider). Tự xoá mình TRƯỚC khi git add.
set -euo pipefail
rm -f "$0"
git add -A
git commit -m "feat(llm): hot-swap provider runtime with persisted selection"
git push origin master
