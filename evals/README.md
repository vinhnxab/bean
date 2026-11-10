# evals/ — bộ eval đo chất lượng agent

Đo **tác vụ có hoàn thành hay không** (tỉ lệ pass, số bước, token, số lần
confirm) thay vì chỉ tính đúng đắn của code. Bổ sung cho `make check`: test
đơn vị chứng minh code làm đúng, eval chứng minh *agent* làm tốt.

## Cấu trúc

- `tasks/*.toml` — mỗi file một task: prompt, file setup, kịch bản
  `FakeProvider` (task xác định, chạy được trong CI) hoặc cấu hình provider
  thật (opt-in, key **chỉ** từ biến môi trường).
- `out/` — kết quả mỗi lượt chạy: `metrics.jsonl` (một dòng JSON mỗi task) và
  `trajectory/<task_id>.json` (băng ghi sự kiện để debug vì sao model chọn
  sai tool). Thư mục này không commit.

## Chạy

```sh
bean eval run                                   # mọi task trong evals/tasks
bean eval run --only fake_multistep_fix_note    # một task
bean eval run --provider real                   # ép provider thật (bỏ qua script)
bean eval report evals/out/metrics.jsonl        # tổng hợp một file metrics
```

## Task hiện có

| Task | Đo gì | Gắn với |
|---|---|---|
| `fake_multistep_fix_note` | Vòng lặp nhiều bước, confirm `write_file`, hiệu ứng file trong workspace | máy eval |
| `fake_compaction_long_history` | Lịch sử 30 message vượt 70% ngân sách ⇒ compaction lưu summary, run không lỗi | K12 (chạy `--provider real` để kiểm chứng với model thật) |
| `fake_reflection_skill_draft` | Run ≥5 tool call thành công ⇒ reflection sinh skill nháp, không tự kích hoạt | K17 (như trên) |

## Task provider thật (opt-in, KHÔNG chạy trong CI)

`tasks-real/` chứa task đòi hỏi model thật — đóng K12/K17 bằng số đo thay vì
giả định:

| Task | Đo gì | Gắn với |
|---|---|---|
| `real_compaction_long_history` | Hội thoại đủ dài kích hoạt compaction với model thật: summary lưu, run không lỗi API | K12 — đã đóng 2026-11-09 |
| `real_reflection_skill_draft` | Run thật ≥6 tool call ⇒ reflection trả JSON đúng schema ⇒ draft SKILL.md hợp lệ chờ duyệt | K17 — đã đóng 2026-11-09 (openai_compat) |

Chạy (eval **chỉ đọc key từ biến môi trường** — không bao giờ nằm trong file,
không in giá trị):

```sh
set -a; . ~/.bean/secrets.env; set +a   # hoặc export OPENROUTER_KEY=… / NVIDIA_KEY=…
bean eval run --tasks evals/tasks-real --provider real
bean eval run --tasks evals/tasks-real --provider real --only real_reflection_skill_draft
```

Lưu ý với model free (quan sát thật 2026-11-09):

- **Flaky là dữ liệu, không phải lỗi harness.** Cùng một task có thể PASS hoặc
  FAIL vì model free hay 503, chậm, hoặc trả JSON sai escape. Chạy lại vài lượt
  trước khi kết luận; mọi đường im lặng của reflection đều đã có log
  (`bean-core/src/learning.rs`, `router/run.rs`).
- Lỗi parse reflection kèm **lỗi serde + nguyên văn model trả** (cắt 400 ký
  tự) — đọc log để biết phải sửa prompt hay chấp nhận model yếu.
- Cửa sổ chờ draft với provider thật là 180s (`REFLECTION_WAIT_REAL_MS` trong
  `bean-eval/src/runner.rs`); model free chậm có khi vượt — tăng nếu cần.
- Gán K17 cho model khác: sửa `[llm]` trong TOML (provider/model/base_url/
  `api_key_env` là **tên** biến), giữ nguyên phần setup/assert.
- **Giảm flaky (2026-11-10):** khai báo `[llm].fallback_profiles = ["profile-dự-phòng"]`
  trong TOML của task — model chính trả 503/401 sau khi hết retry là provider tự
  chuyển sang profile kế thay vì chết giữa task. Xem `[llm]` trong `bean.example.toml`.

## Viết task mới

1. Copy một task TOML, đổi `id` + `prompt`.
2. Kịch bản fake: `llm.script` là chuỗi JSON `{"responses": [...]}` (định dạng
   `bean_llm::FakeScript`). Provider thật: khai báo `llm.provider/model/
   base_url/api_key_env` thay cho `script` — `api_key_env` là **tên biến môi
   trường**, không bao giờ đặt giá trị key vào file.
3. Khai báo `[assert]`: xem `bean_eval::Assertions` (end_reason, tools_called,
   files_equal, summary_saved, draft_created...). Task không có assert nào vẫn
   chạy — dùng để lấy đường cơ sở metrics.
4. Chạy `cargo test -p bean-eval` — task fake mới phải pass như một regression.

Nguyên tắc: task eval không đụng dữ liệu thật — runner dựng workspace/SQLite/
skills trong thư mục tạm và xoá sau khi chạy.
