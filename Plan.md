# Bean — Kế hoạch tổng hợp (thay thế mọi bản nháp trước trong phiên hôm nay)

Tài liệu này **hợp nhất và thay thế** nội dung milestone trong 4 file trước:
`va-S1-va-cap-nhat-tai-lieu.md` (giữ nguyên, không đổi — xem mục 1),
`phan-tich-da-kenh-va-ca-nhan-hoa.md`, `phan-tich-developer-va-secops.md`,
`rbac-theo-vai-tro.md`, `marketing-va-rang-buoc-ab.md`. Dùng file này làm nguồn duy nhất cho
milestone M18 trở đi; 4 file cũ chỉ còn giá trị tham khảo lý do/phân tích.

---

## 0. Tóm tắt quyết định đã chốt hôm nay

- **Kênh**: chỉ Telegram + Web (đã có). Discord cân nhắc nếu rẻ. **Bỏ hẳn Slack và WhatsApp.**
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

**M18 → M21 → M22 → M22a → M23 (chờ S1) → M24 → M25**. WhatsApp/Slack: loại khỏi roadmap. Bảo vệ
vật lý và kiến trúc B: để backlog, không có số milestone. M25 có thể làm song song M22-M24 vì
chỉ phụ thuộc M21 (RBAC).

### M18 — Discord adapter (tuỳ chọn, chỉ làm nếu rẻ)

```
Thêm Discord làm channel adapter mới, đi qua Router hiện có — KHÔNG tạo luồng xử lý riêng.
Trước khi code: đọc lại Router/Channel trait hiện tại và adapter Telegram, liệt kê lại cho tôi
các bất biến Telegram đang đảm bảo (allowlist, reconnect/backoff, xác nhận Confirm/Dangerous,
một token/instance) trước khi thiết kế Discord. Chờ tôi duyệt danh sách đó rồi mới code.

Việc cần làm:
1. Adapter Discord dùng thư viện Rust trưởng thành (serenity hoặc twilight — nêu lý do chọn).
2. Danh tính map vào agent.allowed_users với tiền tố discord:<id>, tách biệt hoàn toàn khỏi
   telegram:<id>.
3. Cơ chế Confirm/Dangerous dùng Discord button component tương đương inline keyboard Telegram;
   Dangerous vẫn không có nút trong phiên.
4. Người gửi không thuộc allowed_users bị Router chặn trước khi vào agent — test tương tự
   "callback người lạ bị bỏ qua" đã có cho Telegram.
5. Reconnect/backoff riêng theo rate limit Discord, không tái dùng tham số của Telegram.
6. KHÔNG tạo context builder riêng cho Discord — dùng đúng context builder chung.

Test bắt buộc: người lạ bị chặn, Confirm/Dangerous qua mock transport, reconnect sau khi rớt kết
nối giả lập. make check xanh. KHÔNG động vào S1/K1 hay bất kỳ tool file/shell nào.
```

### M21 — Project profile + RBAC + Developer/QA role (bản hợp nhất cuối cùng)

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
   khi khởi tạo phiên. Bean tự phát triển chính nó là một project profile mặc định, không
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

### M22 — Monitor agent (Nhóm 1, giám sát read-only)

```
Thêm role/agent con "monitor" (tag infra-read). Kết nối MCP server cho SIEM/log (Wazuh/ELK) và
CVE tracking, dùng đúng cơ chế MCP đã có (stdio, trust=false mặc định cần Confirm). KHÔNG thêm
tool quét chủ động trong milestone này — chỉ đọc.

Test: dữ liệu MCP trả về được bọc untrusted-content đúng như tool web/MCP hiện có. Role monitor
không thấy tool ngoài tag infra-read. make check xanh.
```

### M22a — Finance-readonly (Nhóm 0, billing)

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

### M23 — Security-scan agent (Nhóm 2, quét trong scope) — CHỈ làm sau khi S1 đã vá

```
[CHỜ xác nhận S1 đã vá và make check xanh trước khi bắt đầu milestone này.]

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

### M25 — Bean làm MCP server (chỉ read-only), để hiện diện trong IDE/tool khác (Cline/Cursor/OpenCode/Claude Code)

> ✅ **Đã hoàn thành 2026-09-27.** Cả hai transport (stdio + streamable-HTTP/SSE), token lưu
> hash, cổng expose cứng 3 tag + `Safe` chặn **trước** RBAC. Quyết định: `docs/decisions.md`
> D16.1–D16.13. K24 (rate-limit `/mcp` + log riêng) đã đóng 2026-09-27: giới hạn tần suất
> **chỉ** cho transport HTTP, tái dùng đúng thuật toán khoá của `POST /api/auth/login`;
> nhật ký riêng `audit/mcp.jsonl` ghi **song song** `audit.jsonl`. Tồn đọng còn lại: chưa
> thử với Cline/Cursor **thật**.

```
Thêm chế độ MCP server cho Bean, để các coding agent khác (Cline, Cursor, OpenCode, Claude
Code...) có thể gọi vào Bean như một MCP tool ngay trong phiên làm việc của họ. Chỉ làm SAU khi
M21 (RBAC theo tag) đã xong — milestone này tái dùng nguyên cơ chế role/tag, không viết lại.

Phạm vi cứng, không thương lượng trong milestone này:
- CHỈ expose tool thuộc tag infra-read, billing-read, và một tool memory-query (đọc MEMORY.md/
  USER.md hiện có, KHÔNG cho sửa).
- TUYỆT ĐỐI không expose bất kỳ tool nào có thể ghi/thực thi (dev-write, infra-scan, Nhóm 3
  remediation, marketing-publish...) qua đường MCP server trong milestone này, kể cả nếu client
  gọi vào tự xưng có quyền cao — việc đó để milestone sau, sau khi có xác thực client mạnh hơn
  và đã vận hành thử phần read-only ổn định.

Việc cần làm:
1. Chọn transport: nếu Bean chạy trên máy cùng IDE, hỗ trợ stdio như MCP server thông thường.
   Nếu Bean chạy remote (theo mô hình web đã có, sau reverse proxy) thì cần transport HTTP/SSE
   có xác thực — dùng lại đúng cơ chế auth đã có (không tạo hệ xác thực song song), ví dụ token
   dài hạn sinh riêng cho từng MCP client, lưu hash giống auth.toml hiện tại.
2. Danh tính mỗi MCP client map vào agent.user_roles với tiền tố riêng (mcp-client:<id>),
   tách biệt hoàn toàn khỏi telegram:<id>/discord:<id> — một client bị lộ token không tự động
   có quyền như bạn dùng qua Telegram.
3. Router lọc tool expose ra ngoài theo đúng role của client, TÁI DÙNG cơ chế required_tags đã
   có ở M21 — không viết lại logic lọc quyền riêng cho đường MCP server.
4. Tham số mà client bên ngoài (Cline/Cursor...) truyền vào khi gọi tool Bean PHẢI được coi là
   input không tin cậy — áp dụng đúng nguyên tắc "query của model là dữ liệu không tin cậy" đã
   có trong known-issues.md (làm sạch trước khi dùng trong truy vấn nội bộ), không tin tưởng chỉ
   vì request tới từ một coding agent "có vẻ đáng tin".
5. Log riêng mọi request qua MCP server (client nào, tool nào, thời điểm) — đây là bề mặt mới,
   cần audit tách biệt để dễ phát hiện nếu một token bị lộ.

Test bắt buộc:
- Client không có token hợp lệ bị từ chối ngay ở bước handshake.
- Client với role finance-readonly chỉ thấy đúng tool billing-read khi liệt kê tool khả dụng.
- Thử gọi trực tiếp một tool KHÔNG nằm trong danh sách expose (ví dụ cố gọi thẳng tên tool
  dev-write) bị từ chối, không phải chỉ ẩn khỏi danh sách.
- Tham số đầu vào từ client chứa chuỗi giống SQL/FTS injection bị làm sạch, không lọt xuống tầng
  dưới nguyên văn.
make check xanh.
```

---

## 6. Backlog — chưa có số milestone, chờ quyết định thêm

- **Bảo vệ vật lý**: domain hoàn toàn mới (camera, kiểm soát ra vào) — không dùng tag `infra-*`
  đã đặt cho server/network, thiết kế khi cần.
- **Kiến trúc B** (nhiều tiến trình): chỉ cân nhắc khi A đã chạy ổn và có nhu cầu cách ly mạnh
  hơn thực sự — 3 ràng buộc ở mục 4 đã giữ đường lui, không cần làm sớm.
- **Sub-agent song song** (kiểu Hermes) và **remote/serverless terminal backend**: để sau, không
  cấp thiết cho quy mô hiện tại.
- **LSP-bridge MCP server** (Bean làm MCP *client* nối vào Language Server để hiểu code sâu hơn,
  thu hẹp khoảng cách với OpenCode): rẻ hơn tự viết LSP integration, chỉ cần thêm một entry
  `[[mcp_servers]]` — cân nhắc làm sau M25 nếu Developer role cần sửa codebase lớn/phức tạp hơn.
- **Checkpoint/rollback cho Developer role** (học từ Cline): snapshot workspace trước khi agent
  chạy tác vụ nhiều bước, cho phép rollback — đáng làm, rẻ, tăng an toàn rõ rệt; chưa gán số
  milestone, nên gộp vào M21 hoặc làm milestone riêng ngay sau nếu muốn.
- **Extension mỏng cho VS Code** (chỉ là webview gọi lại Web adapter, không thêm trí tuệ): giá
  trị thấp so với M25, ưu tiên sau cùng nếu vẫn muốn có UI trong editor.

---

## 7. Câu hỏi mở còn lại

Không còn câu hỏi chặn tiến độ M18/M21/M22/M22a/M24/M25. **M23 vẫn chờ bạn xác nhận S1 đã vá
xong** trước khi giao cho coding agent. ~~M25 chờ M21 xong.~~ → M21 đã xong (`6a5d063`) và
M25 đã hoàn thành 2026-09-27.
