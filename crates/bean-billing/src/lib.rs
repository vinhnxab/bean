//! # bean-billing
//!
//! Domain **tài chính read-only** (Plan.md M22a): đọc chi phí cloud, tách biệt hoàn toàn khỏi
//! tag `infra-*`.
//!
//! # Vì sao "generic"
//!
//! `Plan.md` M22a ghi *"chọn theo nhà cung cấp cloud công ty đang dùng"*. Vì chưa chốt được
//! nhà cung cấp, milestone này dựng tool theo hướng **generic**: endpoint và credential do
//! cấu hình quyết định, không hard-code AWS/Azure/GCP. Đổi provider chỉ cần sửa
//! `bean.toml`, không phải sửa hay build lại binary (D13.1).
//!
//! # Chế độ stub (mặc định)
//!
//! Khi `base_url` chưa được đặt, tool **không gọi mạng** và trả về thông báo nói rõ cần cấu
//! hình gì. Nhờ vậy `make check` và môi trường phát triển chạy được ngay, và người vận hành
//! không bao giờ tưởng đã đọc được chi phí thật (D13.3).
//!
//! # Bất biến an toàn
//!
//! * **Read-only**: tool này chỉ `GET`. Không có code path nào ghi/thay đổi hạ tầng hay chi phí.
//! * **Credential riêng**: `Config::validate_billing` từ chối cấu hình dùng chung biến môi
//!   trường với LLM/web/Telegram (D13.2) — vì credential quản trị hạ tầng không được tái sử dụng.
//! * **Chống SSRF**: URL đi qua `SafeHttpClient` (lọc IP trước khi kết nối, kiểm lại mỗi bước
//!   redirect) giống `web_fetch`.
//! * **Untrusted**: dữ liệu chi phí là dữ liệu ngoài lõi (mục 15.4) — tool bọc
//!   `<untrusted_content>` và bật cờ `untrusted_seen` qua `.untrusted()`.
//! * **Không rò secret**: `SecretString` không bao giờ vào `Display`/`Debug`/log.
#![forbid(unsafe_code)]

mod tool;

pub use tool::{
    BillingClient, BillingCostParams, BillingError, BillingSource, billing_read_cost, stub_source,
};
