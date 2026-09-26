# BeanAgent — Kế hoạch tổng hợp (thay thế mọi bản nháp trước trong phiên hôm nay)

Tài liệu này **hợp nhất và thay thế** nội dung milestone trong 4 file trước:
`va-S1-va-cap-nhat-tai-lieu.md` (giữ nguyên, không đổi — xem mục 1),
`phan-tich-da-kenh-va-ca-nhan-hoa.md`, `phan-tich-developer-va-secops.md`,
`rbac-theo-vai-tro.md`, `marketing-va-rang-buoc-ab.md`. Dùng file này làm nguồn duy nhất cho
milestone M18 trở đi; 4 file cũ chỉ còn giá trị tham khảo lý do/phân tích.

---

## 0. Tóm tắt quyết định đã chốt hôm nay

- **Kênh**: chỉ Telegram + Web (đã có). **Bỏ hẳn Discord, Slack và WhatsApp** (2026-09-26, xem mục 5.0).
- **S1**: bạn tự vá, theo prompt đã soạn ở `va-S1-va-cap-nhat-tai-lieu.md`. Mọi milestone hạ
  tầng chạm Nhóm 2 trở lên (quét chủ động) hoặc developer-mode cho dự án ngoài đều **chờ S1 vá
  xong** mới bật.
- **Mô hình tổng thể**: Bean là **Manager điều phối** một nhóm agent con chuyên trách, không
  phải một agent "toàn năng" ôm hết quyền. Định tuyến theo RBAC, không có quyền ngầm định.
- **Kiến trúc triển khai**: chọn phương án **A — một tiến trình, phân vai bằng role/project
  profile**, có giữ đường lui sang **B — nhiều tiến trình riêng** khi cần, bằng 3 ràng buộc kiến
  trúc bắt buộc (mục 4).
- **"Hacker mũ trắng"** đã đóng khung lại thành trợ lý SecOps/IT nội bộ **thiên phòng thủ**
  (giám sát → audit → quét trong phạm vi khai báo cứng), không phải agent tự khai thác chủ động.
- **Vai trò con người** đã xác nhận: bạn (admin), IT/an ninh mạng, bảo vệ vật lý (domain riêng,
  để sau), kế toán (chỉ xem chi phí hạ tầng).
- **Agent con** (do Bean điều phối) đã thêm: Developer, QA (tách khỏi Developer theo nguyên tắc
  four-eyes), Monitor, Security-scan, Marketing.

---

## 1. Vá S1 — không đổi so với bản đã soạn

Không lặp lại nội dung ở đây. Dùng nguyên `va-S1-va-cap-nhat-tai-lieu.md` — patch AGENTS.md/
README.md/known-issues.md/status-report.md + prompt vá S1. Bạn đã nói sẽ tự làm phần này.

---

## 2. Vai trò con người (RBAC) — bảng cuối cùng

| Vai trò | Tag được cấp | Duyệt rủi ro cao |
|---|---|---|
| Bạn (admin) | `*` (mọi tag) | Tự duyệt |
| IT/an ninh mạng | `infra-read`, `infra-scan` | Remediation (Nhóm 3) cần bạn duyệt |
| Bảo vệ vật lý | *(chưa cấp tag nào — domain riêng, thiết kế sau)* | — |
| Kế toán | `billing-read` | — (chỉ đọc) |

Người không có trong `agent.user_roles` → role mặc định **`no-access`** (không tag nào), không
phải admin. Đây là bất biến bắt buộc, không được đổi mặc định.

## 2b. Agent con do Bean điều phối — bảng cuối cùng

| Agent con | Việc | Tag | Duyệt |
|---|---|---|---|
| **Developer** | Viết code, chạy test cục bộ, mở PR | `dev-write` | Không tự merge — luôn Confirm |
| **QA** (tách riêng, không gộp Developer) | Review diff, chạy test độc lập | `dev-read`, `test-run` — **không có quyền ghi code** | Báo cáo lên Manager |
| **Monitor** | Giám sát log/CVE/uptime, read-only | `infra-read` | — |
| **Security-scan** | Quét trong `[[infra_scope]]`, audit cấu hình | `infra-scan` | Remediation luôn qua bạn (Nhóm 3) |
| **Marketing** | Nghiên cứu, soạn nháp, đăng công khai | `marketing-read`, `marketing-draft`, `marketing-publish` | `marketing-publish` **luôn Confirm, mọi lần, không session-wide** |

Nguyên tắc four-eyes: Developer không tự QA code của chính nó. Security-scan cảnh báo mức cao
bắn thẳng cho bạn song song với báo Manager, không chỉ qua Manager lọc.

---

## 3. Bean-Manager: dispatcher hẹp, không toàn năng

- Manager **không đọc dữ liệu thô** của agent con — chỉ đọc báo cáo đã chuẩn hoá dạng
  `{status, summary, risks}` (schema cố định), để injection từ dữ liệu một agent con đọc phải
  (dependency độc, trang web độc) không lan thẳng vào ngữ cảnh Manager.
- **Shared workspace mặc định untrusted** — nội dung agent khác ghi vào đó được bọc
  `<untrusted_content>` như file/web, trừ khi định dạng có cấu trúc cố định (JSON schema), không
  phải văn bản tự do.
- Ngân sách token **tách riêng theo từng agent con**, không dùng chung một `daily_token_budget` —
  tránh Developer chạy vòng lặp dài ăn hết ngân sách khiến Monitor/Security-scan không chạy được
  job định kỳ.

---

## 4. Ba ràng buộc kiến trúc A→B (áp dụng cho MỌI milestone dưới đây, không chỉ M21)

1. Giao tiếp Manager ↔ agent con qua kiểu dữ liệu tuần tự hoá được (serializable request/
   response) — không truyền tham chiếu Rust nội bộ qua lại giữa các role.
2. Mọi truy cập SQLite tiếp tục qua đúng một lớp worker hiện có (D8.2) — không role nào tự mở
   kết nối `rusqlite` riêng.
3. RBAC check nằm ở đúng một điểm trong Router — không rải rác trong logic từng tool/role.

Nếu một thiết kế buộc phải phá 1 trong 3 điều trên, coding agent phải dừng và hỏi trước khi làm.

---

## 5. Roadmap milestone cuối cùng, theo thứ tự

**M21 → M22 → M22a → M23 → M24**. Bảo vệ vật lý và kiến trúc B: để backlog, không có số milestone.

### 5.0 — M18 (Discord adapter): **ĐÃ LOẠI BỎ** (2026-09-26)

Chủ dự án quyết **bỏ hẳn Discord** khỏi hệ thống để giữ đơn giản. M18 không bao giờ được code.

**Lý do** (rút ra khi khảo sát bất biến Telegram trước khi thiết kế Discord):

1. **Mất bất biến "một token/instance".** Telegram trả `409 Conflict` nên chặn trùng token bằng tín hiệu
   API. Discord **cho phép nhiều shard với cùng token**, không có tín hiệu tương đương ⇒ phải tự viết
   cơ chế claim mới (heartbeat trên đĩa) — thêm bề mặt lỗi chỉ để thêm một kênh chat.
2. **Vòng đời interaction khác hẳn.** Discord bắt buộc trả lời interaction trong **3 giây** nếu
   không `defer`, và interaction **hết hạn ~15 phút**. Phải thêm `defer` + TTL dọn rác cho bảng
   confirm — Telegram không cần vì Router tự quản lý hết hạn 300s.
3. **Sẽ phải port lại ~44 bất biến Telegram** (allowlist 2 lớp, chống giả mạo callback, dedup,
   rate limit, confirm 3 nút, backoff, redact secret…) — chi phí lớn hơn nhiều so với giá trị thêm.
4. M18 vốn đã ghi *"tuỳ chọn, chỉ làm nếu rẻ"*, chưa từng có nhu cầu vận hành thực tế.

**Hệ quả:** Slack/WhatsApp đã bỏ từ trước ⇒ **Telegram + Web là hai kênh cuối cùng**. Bảng tag ở mục
2/2b và toàn bộ milestone M22–M24 giữ nguyên; không milestone nào phụ thuộc Discord.

> Tài liệu `docs/telegram-invariants.md` (danh sách 44 bất biến + phân tích 3 điểm không port được ở
> trên) **đã bị xoá** cùng M18. Lý do bỏ đã rút gọn vào mục này.

### M21 — Project profile + RBAC + Developer/QA role (bản hợp nhất cuối cùng) — ✅ **ĐÃ XONG 2026-09-26**

Bằng chứng: `crates/beanagent-core/tests/rbac.rs` (7 test) + `store::tests::usage_by_role_is_independent_between_roles`.
Quyết định thiết kế: `docs/decisions.md` mục 11 (D11.1–D11.9). Điểm đã chốt trước khi code:
`no-access` là **deny-all** (D11.1) và RBAC chỉ bật khi `agent.user_roles` khác rỗng (D11.2).

```
Ràng buộc kiến trúc bắt buộc (giữ đường lui sang mô hình nhiều tiến trình sau này):
1. Giao tiếp giữa Manager (Bean) và mỗi role/agent con PHẢI đi qua kiểu dữ liệu tuần tự hoá được
   (struct derive Serialize/Deserialize) — KHÔNG truyền tham chiếu Rust nội bộ (&mut,
   Arc<RwLock<...>>) qua lại giữa Manager và agent con. Coi mỗi lời gọi như thể nó có thể là một
   HTTP/gRPC call trong tương lai, dù bây giờ là gọi hàm trực tiếp.
2. MỌI truy cập SQLite tiếp tục đi qua đúng một lớp worker hiện có (D8.2) — không role/agent con
   nào tự mở kết nối rusqlite riêng. Cần truy vấn mới thì thêm method vào lớp worker đó.
3. RBAC check (tag/role) nằm ở đúng một điểm trong Router, không rải rác trong logic từng tool.

Nếu thấy phần nào bắt buộc phải phá 1 trong 3 ràng buộc trên để chạy được, DỪNG lại và hỏi tôi
trước, đừng tự phá ràng buộc để xong việc nhanh hơn.

Việc cần làm:
1. Project profile: mỗi project có workspace/MEMORY.md/USER.md riêng, chọn qua config hoặc lệnh
   khi khởi tạo phiên. BeanAgent tự phát triển chính nó là một project profile mặc định, không
   đặc quyền hơn project khác.
2. Bảng roles trong cấu hình: mỗi role có tool_tags. Role "admin" có tag đặc biệt "*".
   Roles cần có ngay: admin, it-security (tag infra-read, infra-scan), finance-readonly (tag
   billing-read), developer (tag dev-write), qa (tag dev-read, test-run — KHÔNG có dev-write).
3. agent.user_roles map user identity (telegram:<id>...) -> role name. Người không có trong map
   dùng role mặc định "no-access" (không tag nào) — an toàn theo mặc định, không phải admin.
4. fn required_tags(&self) -> &[&str] vào trait Tool, mặc định &[] (tool không nhạy cảm, ai
   trong allowed_users cũng gọi được — giữ hành vi cũ cho tool chat thường).
5. Router lọc danh sách tool gửi cho model theo role của người gửi TRƯỚC khi build request tới
   LLM — không lọc sau khi model đã "chọn" tool.
6. Role qa không được có tag dev-write dưới bất kỳ hình thức nào (kể cả gián tiếp qua tool khác)
   — nguyên tắc four-eyes: agent review không được có quyền tự sửa code nó đang review.
7. Ngân sách token (context_budget_tokens/daily nếu áp dụng) tính riêng theo từng role/agent con,
   không dùng chung một số duy nhất cho toàn instance.

Test bắt buộc:
- User role finance-readonly không thấy tool nào ngoài tag billing-read trong payload gửi LLM.
- User không có trong user_roles là no-access, không gọi được tool nào kể cả tool "an toàn" cũ.
- Role qa không gọi được (và không thấy) bất kỳ tool có tag dev-write.
- Hai project không lẫn MEMORY.md của nhau.
make check xanh.
```

### M22 — Monitor agent (Nhóm 1, giám sát read-only) — ✅ **ĐÃ XONG 2026-09-26**

Bằng chứng: `crates/beanagent-mcp-test-server/tests/mcp_client.rs` (3 test M22, dùng MCP stdio thật).
Quyết định thiết kế: `docs/decisions.md` mục 12 (D12.1–D12.5).

```
Thêm role/agent con "monitor" (tag infra-read). Kết nối MCP server cho SIEM/log (Wazuh/ELK) và
CVE tracking, dùng đúng cơ chế MCP đã có (stdio, trust=false mặc định cần Confirm). KHÔNG thêm
tool quét chủ động trong milestone này — chỉ đọc.

Test: dữ liệu MCP trả về được bọc untrusted-content đúng như tool web/MCP hiện có. Role monitor
không thấy tool ngoài tag infra-read. make check xanh.
```

### M22a — Finance-readonly (Nhóm 0, billing) — ✅ **ĐÃ XONG 2026-09-26**

Đi hướng **generic** (chủ dự án chọn thay vì chốt hẳn AWS/Azure/GCP): endpoint + credential lấy
từ cấu hình, đổi provider chỉ sửa `BeanAgent.toml`. Bằng chứng:
`crates/beanagent-billing/tests/billing.rs` (4 test). Quyết định: `docs/decisions.md` mục 13
(D13.1–D13.5).

```
Thêm domain "billing-read" cho role finance-readonly, tách biệt hoàn toàn khỏi tag infra-*.

1. Tool đọc chi phí cloud (AWS Cost Explorer/Azure Cost Management/GCP Billing — chọn theo nhà
   cung cấp công ty đang dùng), tag required_tags = ["billing-read"].
2. Credential đọc billing PHẢI là API key/IAM role riêng, quyền tối thiểu chỉ đọc billing —
   KHÔNG dùng chung credential có quyền quản trị hạ tầng.
3. Không thêm tag infra-* nào vào role finance-readonly trong milestone này.

Test: role finance-readonly gọi được billing-read, không gọi/không thấy bất kỳ tool tag infra-*.
make check xanh.
```

### M23 — Security-scan agent (Nhóm 2, quét trong scope) — ✅ ĐÃ XONG

```
[Xác nhận S1 đã vá (2026-09-26) và `infra_scope` rỗng = fail-closed. Xem `docs/decisions.md` D14.x.]

Thêm role/agent con "security-scan" (tag infra-scan). Thêm [[infra_scope]] vào cấu hình, mọi
tool quét (nmap/OpenVAS/Trivy qua MCP hoặc tool nội bộ) phải kiểm tra target nằm trong targets
đã khai báo TRƯỚC KHI thực thi, từ chối nếu không khớp — kiểm tra ở tầng code, không dựa vào
model tự kiểm tra. Không có "cho phép trong phiên" cho tool quét — luôn Confirm từng lần. Output
scanner bọc untrusted-content.

Cảnh báo mức cao từ security-scan gửi thẳng cho bạn (kênh chính), song song với báo cáo chuẩn
hoá gửi Manager — không chỉ đi qua Manager lọc.

Test: target ngoài scope bị từ chối dù model "quyết" chạy; output scanner chứa chuỗi giống chỉ
dẫn injection không khiến agent hành động thêm mà không hỏi lại. make check xanh.
```

### M24 — Marketing agent

```
Thêm role "marketing" theo RBAC đã có (M21). Domain tách biệt hoàn toàn khỏi infra/dev/finance.

1. Tag marketing-read: dùng lại web_fetch/web_search đã có, chỉ gắn thêm tag cho role marketing.
2. Tag marketing-draft: tool lưu bản nháp nội dung vào workspace riêng của role marketing — chỉ
   ghi file, KHÔNG gọi API mạng xã hội nào ở bước này.
3. Tag marketing-publish: tool đăng thật lên 1-2 nền tảng chọn trước (vd X, LinkedIn qua API
   chính thức). BẮT BUỘC luôn Dangerous — không "cho phép trong phiên" dù cấu hình nói gì, kiểm
   tra cứng ở code.
4. Credential mỗi nền tảng lưu riêng biến môi trường, quyền API tối thiểu (chỉ post).
5. Hướng dẫn cho role marketing: không bịa số liệu/testimonial không có nguồn thật; nội dung tự
   sinh, không sao chép nguyên văn từ nguồn đã đọc qua web_fetch.

Test: role marketing không thấy/gọi tool ngoài tag marketing-*; gọi marketing-publish luôn yêu
cầu Confirm kể cả gọi liên tiếp cùng phiên; marketing-draft không có network call nào. make check
xanh.
```

---

## 6. Backlog — chưa có số milestone, chờ quyết định thêm

- **Bảo vệ vật lý**: domain hoàn toàn mới (camera, kiểm soát ra vào) — không dùng tag `infra-*`
  đã đặt cho server/network, thiết kế khi cần.
- **Kiến trúc B** (nhiều tiến trình): chỉ cân nhắc khi A đã chạy ổn và có nhu cầu cách ly mạnh
  hơn thực sự — 3 ràng buộc ở mục 4 đã giữ đường lui, không cần làm sớm.
- **Sub-agent song song** (kiểu Hermes) và **remote/serverless terminal backend**: để sau, không
  cấp thiết cho quy mô hiện tại.

---

## 7. Câu hỏi mở còn lại

**Không còn câu hỏi chặn tiến độ M21/M22/M22a/M23/M24.**

* **S1 đã vá xong** (commit `9415c5d`, `make check` xanh) ⇒ chốt chặn của **M23 đã mở**, không còn
  cần xác nhận thêm.
* **M18 (Discord) đã loại bỏ** (mục 5.0) — không phải câu hỏi mở nữa.

* **`[[infra_scope]]` đang RỖNG — cần bạn điền target thật khi muốn dùng.** Chủ dự án đã chốt
  (2026-09-26) khởi đầu với scope rỗng: code + test đầy đủ, mọi lần quét bị từ chối ở tầng
  code. Bỏ comment `[[infra_scope]]` trong `BeanAgent.example.toml` để mở.
* **`GET /api/status` chưa hiện hạn mức per-role.** Muốn UI thấy "role này còn bao nhiêu hạn mức"
  thì thêm endpoint đọc `usage_by_role`.
* **Cảnh báo mức cao M23 đã nối xong** (2026-09-26, đóng K23): `AlertSink` → `ToolCtx.alerts`
  → `Router::notify`. Cảnh báo `High` tới kênh chính, lỗi gửi rơi vào outbox. Cần `docker
  pull instrumentisto/nmap` + bỏ comment `[[infra_scope]]` mới dùng được thật.
