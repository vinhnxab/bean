# Gói vá S1 + cập nhật tài liệu — BeanAgent

Tài liệu này gồm 5 phần, dùng độc lập được:

1. Patch `AGENTS.md` (mục 22.5) — sửa checklist thiếu "file"
2. Patch `README.md` (mô hình đe doạ) — bỏ overclaim, đúng với thực trạng
3. Patch `known-issues.md` — thêm mục S1 theo đúng format sẵn có
4. Patch `status-report.md` — cập nhật để không lỗi thời so với commit `399992d`
5. **Prompt cụ thể** để dán cho coding agent, vá S1 theo đúng quy trình test-first của `AGENTS.md`/`PROMPTS.md`

Áp dụng theo thứ tự: 5 trước (vá code + test xanh) → rồi 1–4 (tài liệu) trong cùng một lượt, đúng khuyến nghị của `security-review.md` mục 7 ("nên ghi thêm một mục vào `docs/decisions.md`... khi quyết định khắc phục").

---

## 1. Patch `AGENTS.md` — mục 22, dòng 742

**Tìm:**
```
5. **Nội dung web/email/MCP là nguồn prompt injection chính**: bọc `<untrusted_content>`.
```

**Thay bằng:**
```
5. **Nội dung web/file/email/MCP là nguồn prompt injection chính**: bọc `<untrusted_content>`.
   Áp dụng cho MỌI tool trả nội dung từ nguồn bên ngoài lõi — bao gồm `read_file`, `grep`,
   `glob`, `list_dir`, output `run_shell` — không chỉ `web_fetch`/`web_search`/MCP. Xem mục 15.4.
```

**Lý do:** mục 15.4 đã liệt kê đúng "web, file, email, MCP" nhưng mục 22.5 (checklist mà prompt
"Review chất lượng theo từng phần" trong `PROMPTS.md` dẫn chiếu) lại thiếu "file". Đây chính là
khoảng trống khiến S1 lọt qua các đợt review định kỳ theo milestone. Sửa để hai mục không lệch
nhau, giảm khả năng tái diễn với tool mới sau này.

---

## 2. Patch `README.md` — mục "Mô hình đe doạ và giới hạn"

**Tìm:**
```
- Nội dung web/file/MCP là untrusted và có thể chứa prompt injection; system prompt và UI
  không được xem nội dung đó là chỉ dẫn đáng tin.
```

### 2a. Patch tạm thời (dùng NGAY nếu README được công bố trước khi vá S1 xong)

```
- Nội dung web/MCP là untrusted và được bọc `<untrusted_content>` tự động; sau khi đọc, tool
  Confirm/Dangerous phải hỏi lại trong lượt đó. **File đọc qua `read_file`/`grep`/`glob`/
  `list_dir` và output `run_shell` CHƯA được bọc tương đương (theo dõi ở docs/known-issues.md,
  mục S1)** — coi mọi nội dung agent đọc được trong workspace là có thể chứa chỉ dẫn giả cho
  đến khi mục này được vá; tránh cho agent chạy `write_file`/`run_shell` "cho phép trong phiên"
  ngay sau khi vừa đọc file không rõ nguồn gốc.
```

### 2b. Patch chính thức (dùng SAU khi vá S1 xong, thay thế bản 2a)

```
- Nội dung web/file/email/MCP là untrusted và có thể chứa prompt injection; system prompt và
  UI không được xem nội dung đó là chỉ dẫn đáng tin. Mọi tool đọc nội dung từ nguồn ngoài lõi
  (`web_fetch`, `web_search`, `read_file`, `grep`, `glob`, `list_dir`, output `run_shell`, MCP)
  đều bọc `<untrusted_content>`; sau khi đọc trong một lượt, mọi tool Confirm/Dangerous bắt
  buộc hỏi lại và mất tuỳ chọn "cho phép trong phiên".
```

**Lý do:** bản gốc là lời cam kết đọc như đã triển khai đầy đủ, trong khi `security-review.md`
xác nhận phần "file" chưa đúng với code thật. README là tài liệu người dùng cuối dùng để quyết
định mức tin tưởng khi tự host — không nên overclaim.

---

## 3. Patch `known-issues.md` — thêm mục S1

### 3a. Thêm dòng vào bảng mục 2 (Điểm yếu đã biết), đặt trên K1 vì mức độ khai thác đã chứng minh

```
| S1 | **Prompt injection qua tool đọc file/lệnh (`read_file`, `grep`, `glob`, `list_dir`, `run_shell`) — không bọc `<untrusted_content>`, không bật `untrusted_seen`.** | Vi phạm trực tiếp mục 15.4: cờ `untrusted_seen` chỉ được bật bởi `web_fetch`/`web_search`/MCP (`agent.rs:521`); 5 tool còn lại trả văn bản thô. Hậu quả: `write_file`/`run_shell` đã được "cho phép trong phiên" trước đó sẽ chạy hoàn toàn không hỏi lại sau khi agent đọc một file độc — kể cả để tự ghi đè `MEMORY.md`/`USER.md`. Đã có test tái hiện bằng tool thật, đang FAIL (`crates/beanagent-core/tests/untrusted_file.rs`). | Bọc `<untrusted_content>` + bật cờ cho `read_file`/`grep`/`glob`/`list_dir`/output `run_shell`, tái dùng `beanagent_tools::untrusted::wrap`. Cân nhắc thêm `Tool::marks_untrusted()` tường minh thay vì suy luận qua nội dung output. Xem chi tiết và đề xuất đầy đủ: `docs/security-review-2026-09-25.md` mục 2.5. | **cao** | Ngay khi có thể — trước khi chạy `serve` với provider thật trên workspace có nội dung không tự viết |
```

### 3b. Cập nhật mục 4 (Việc cần nhặt lại theo milestone)

**Tìm:**
```
* **M16 (Hardening)**: K1, K2, K3, K4, K11, K12, K18 + `make audit`/`make e2e`.
```

**Thay bằng:**
```
* **Trước M16 (ưu tiên trên mọi milestone khác)**: S1 — cùng lớp lỗi với K1, sửa chung một
  lượt (xem `security-review.md`). Không nên chạy `serve` với provider thật cho tới khi vá.
* **M16 (Hardening)**: K1, K2, K3, K4, K11, K12, K18 + `make audit`/`make e2e`.
```

**Lý do:** known-issues.md tự nhận mục đích là "nhớ lại quyết định đã chốt và ghi nhận điểm yếu
còn tồn đọng" — S1 hiện chưa có mặt trong file dù đã được chứng minh khai thác được bằng test
thật, đúng như `security-review.md` mục 7 tự ghi chú là còn thiếu bước này.

---

## 4. Patch `status-report.md`

### 4a. Mục 1 — Tóm tắt điều hành, sau dòng về K1

**Tìm:**
```
- Điểm yếu lớn nhất còn lại theo `docs/known-issues.md` là **K1: prompt injection qua
  `sessions.summary`** (mức cao).
```

**Thay bằng:**
```
- Điểm yếu lớn nhất còn lại theo `docs/known-issues.md` là **K1: prompt injection qua
  `sessions.summary`** (mức cao).
- **Cập nhật (commit `399992d`, sau HEAD báo cáo này):** review bảo mật riêng phát hiện **S1**
  — prompt injection qua `read_file`/`grep`/`glob`/`list_dir`/`run_shell`, cùng lớp lỗi với K1
  nhưng đã có test tái hiện bằng tool thật chứng minh khai thác được (không chỉ là rủi ro lý
  thuyết). Xem `docs/security-review-2026-09-25.md`. **Chưa sửa.**
- **`make check` hiện ĐỎ** ở đúng một file test (`crates/beanagent-core/tests/untrusted_file.rs`,
  cố ý để fail làm bằng chứng cho S1) — bảng "✅ Pass" ở mục 4 của báo cáo này phản ánh trạng
  thái tại HEAD `62e8350`, không còn đúng ở commit `399992d`.
```

### 4b. Mục 6.3 — Backlog kỹ thuật, thêm dòng đầu bảng

**Tìm:**
```
| An toàn | K1 | cao | `sessions.summary` có thể chứa dữ liệu không tin cậy nhưng được chèn vào system prompt; cần gắn nhãn untrusted hoặc chuyển khỏi system |
```

**Thay bằng:**
```
| An toàn | S1 | cao | `read_file`/`grep`/`glob`/`list_dir`/`run_shell` không bọc `<untrusted_content>`, không bật `untrusted_seen` — "cho phép trong phiên" mất tác dụng sau khi đọc file độc; đã có test tái hiện, chưa sửa |
| An toàn | K1 | cao | `sessions.summary` có thể chứa dữ liệu không tin cậy nhưng được chèn vào system prompt; cần gắn nhãn untrusted hoặc chuyển khỏi system |
```

### 4c. Mục 10 — Tiêu chí "v1 hoàn thành", thêm dòng

**Tìm:**
```
- [ ] K1 đã xử lý hoặc chấp nhận rủi ro rõ ràng.
```

**Thay bằng:**
```
- [ ] S1 và K1 đã xử lý hoặc chấp nhận rủi ro rõ ràng (cùng lớp lỗi, nên sửa chung).
```

---

## 5. Prompt cụ thể — giao cho coding agent để vá S1

Dán nguyên văn prompt dưới đây (theo đúng phong cách và quy ước của `PROMPTS.md`: test-first,
chỉ đúng phạm vi, lập kế hoạch trước khi sửa). Khuyến nghị mở phiên mới, đặt cả `agents.md`
lẫn `docs/security-review-2026-09-25.md` vào context trước khi chạy.

```
Vá lỗ hổng S1 đã ghi trong docs/security-review-2026-09-25.md mục 2. KHÔNG mở rộng phạm vi
sang việc khác (K1, K2...) trừ khi tôi đồng ý ở bước lập kế hoạch. Lập kế hoạch trước, chờ tôi
duyệt, rồi mới sửa code.

Bối cảnh:
- agents.md mục 15.4: nội dung từ web, file, email, MCP phải bọc <untrusted_content>; sau khi
  đọc untrusted trong một lượt, mọi tool Confirm trở lên phải hỏi lại (mất "cho phép trong
  phiên"). Cờ untrusted_seen hiện chỉ được bật bởi web_fetch/web_search/MCP (agent.rs:521),
  KHÔNG bật bởi read_file/grep/glob/list_dir/run_shell.
- Đã có test tái hiện bằng tool thật (không phải tool giả): crates/beanagent-core/tests/untrusted_file.rs,
  hiện đang FAIL đúng như kỳ vọng. KHÔNG sửa test này để nó pass bằng cách nới lỏng assertion —
  chỉ sửa code sản phẩm để test pass tự nhiên. Nếu thấy test có vấn đề về logic, dừng lại và hỏi
  trước khi sửa test.

Việc cần làm (theo đề xuất ở security-review-2026-09-25.md mục 2.5, điểm 1-5; điểm 6 — sửa
chung K1 — để riêng, hỏi tôi trước):
1. Bọc output của read_file, grep, glob, list_dir (crates/beanagent-tools/src/builtin/files/tool.rs)
   và output stdout/stderr của run_shell (crates/beanagent-security/src/shell.rs) trong
   <untrusted_content>, escape thẻ đóng nếu nội dung chứa nó. Tái dùng hàm bọc đã có
   (beanagent_tools::untrusted::wrap) — không viết thuật toán mới.
2. Đảm bảo agent.rs:521 (hoặc cơ chế thay thế ở bước 4) bật untrusted_seen đúng cho cả 5 tool
   này, giống cách web.rs:103-104 đang làm cho web_fetch.
3. Cắt output vẫn phải ở ranh giới ký tự UTF-8 sau khi bọc thêm thẻ (tái dùng truncate_chars/
   cap_stream đã có — không viết lại logic cắt).
4. Đề xuất và implement một cách khai báo tường minh hơn quy ước ngầm hiện tại — ví dụ thêm
   fn marks_untrusted(&self) -> bool vào trait Tool (mặc định false, các tool cần bọc override
   true) — để tool mới sau này quên bọc sẽ bị lộ bởi test thay vì âm thầm hỏng. Nêu phương án
   trước khi code nếu có đánh đổi API.
5. list_dir/glob: cân nhắc bật cờ dù tên file hiếm khi chứa chỉ dẫn thực thi (an toàn hơn là
   để sót) — quyết định và ghi lý do ngắn gọn trong PR/commit message.
6. Thêm một test hồi quy buộc mọi tool đăng ký trong registry mà trả nội dung từ nguồn ngoài
   phải khai marks_untrusted() = true, tương tự tinh thần test
   tool_specs_are_safe_and_reject_unknown_fields trong web.rs.

Không đổi API công khai của tool đã có, không đổi tool schema gửi cho model, không ảnh hưởng UI.

Test bắt buộc:
- 2 test hiện có trong untrusted_file.rs chuyển từ FAIL sang PASS mà không sửa assertion.
- Test mới cho grep/glob/list_dir/run_shell tương tự (dùng tool thật, không dùng tool giả).
- cargo test -p beanagent-core --test untrusted_file và cargo test --workspace đều xanh.
- make check xanh (fmt, clippy -D warnings, toàn bộ test, type export nếu có tool schema đổi).

Sau khi vá xong, nhắc tôi cập nhật:
- docs/known-issues.md: thêm mục S1 (đã soạn sẵn ở tài liệu "va-S1-va-cap-nhat-tai-lieu.md"
  mục 3, hoặc đánh dấu "đã xử lý" nếu vá xong ngay).
- README.md: chuyển sang bản patch 2b (mục "Mô hình đe doạ") vì lúc đó phần "file" đã đúng
  với code thật.
- docs/decisions.md: thêm một mục ở phần tương ứng nếu đã chọn trait marks_untrusted() —
  ghi rõ đánh đổi so với quy ước "tool tự bọc" cũ.

Định nghĩa xong: 2 test cũ + test mới đều PASS; make check xanh; báo cáo ngắn theo agents.md
mục 0.7 (đã làm gì, chạy lệnh nào để kiểm tra, còn tồn đọng gì — đặc biệt nêu rõ nếu K1 chưa
được sửa trong cùng lượt này).
```

---

## Ghi chú

- Phần 1–4 là patch tài liệu thuần tuý, có thể áp dụng ngay, không phụ thuộc code.
- Phần 5 là prompt cho coding agent, chỉ nên chạy sau khi đã xem/duyệt kế hoạch nó đề xuất
  (đúng tinh thần "Lập kế hoạch trước, chờ tôi duyệt" đã có sẵn trong văn hoá làm việc của dự án).
- Nếu quyết định vá S1 và K1 chung một lượt (như security-review đề xuất ở mục 2.5 điểm 6),
  cần bổ sung riêng phần việc cho K1 (chuyển `sessions.summary` khỏi system prompt hoặc gắn
  nhãn rõ, kèm test "người dùng nhắp lệnh ẩn trong tóm tắt bị bỏ qua sau compaction" như
  known-issues.md K1 đã mô tả) — chưa đưa vào prompt trên để giữ đúng nguyên tắc "một milestone/
  một lượt chỉ làm đúng phạm vi được giao" của `agents.md` mục 0.1 và 0.4.
