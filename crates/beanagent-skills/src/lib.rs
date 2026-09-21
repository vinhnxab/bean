//! # BeanAgent-skills
//!
//! Loader và tool cho skill (agents.md mục 9).
//!
//! * **M6**: quét `./skills/` và `~/.BeanAgent/skills/`, parse frontmatter (chỉ 2 trường
//!   `name`/`description` — tự parse, không dùng crate YAML nào; xem `docs/decisions.md` D3.8),
//!   validate kebab-case + khớp tên thư mục + `description` ≤ 300 ký tự, bỏ qua và log skill lỗi.
//! * Progressive disclosure: system prompt chỉ chứa danh sách `name: description`;
//!   `load_skill` (Safe) mới trả nội dung đầy đủ; `create_skill` (Confirm) chặn ghi đè.
#![forbid(unsafe_code)]
