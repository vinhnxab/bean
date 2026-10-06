#![allow(clippy::panic, clippy::expect_used)]
//! Chặn hồi quy kiến trúc ở adapter Telegram.
//!
//! # Vì sao cần
//!
//! `telegram.rs` từng là **một** file 1992 dòng trộn năm việc không liên quan: kiểu
//! dữ liệu, quy tắc giới hạn của kênh, transport `teloxide`, bảng đích callback và
//! vòng lặp polling.
//!
//! Telegram là kênh có **bề mặt tấn công**: allowlist user là điều kiện an toàn, và bỏ
//! nó là biến bot thành tài khoản đọc/ghi tin nhắn của bất kỳ ai. Sửa quy tắc đó phải
//! không cần mở file chứa vòng lặp `teloxide` và 777 dòng test.
//!
//! Đây là "guardrail", không phải đo chất lượng: nó chỉ nói "file này đã lớn quá mức dễ
//! đọc", còn `clippy`/`fmt`/`cargo test` lo phần còn lại.

use std::path::{Path, PathBuf};

/// Trần số dòng cho **từng** file dưới `src/telegram/`.
/// Lớn nhất là `tests.rs` (~786) — dưới trần này.
const FILE_LINE_BUDGET: usize = 850;

/// Số module tối thiểu trực tiếp dưới `src/telegram/`.
const MIN_TELEGRAM_MODULES: usize = 9;

/// Các module mà doc của `mod.rs` phải nhắc tới — bản đồ để người mới đọc.
const MOD_DOC_MUST_MENTION: &[&str] = &[
    "types.rs",
    "text_rate.rs",
    "transport.rs",
    "targets.rs",
    "handlers.rs",
    "polling.rs",
];

/// Thư mục gốc `src/telegram`.
fn telegram_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/telegram")
}

/// Đếm file `.rs` trong thư mục, không đệ quy.
fn count_rs(dir: &Path) -> usize {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("không đọc được `{}`: {error}", dir.display()));
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
        .count()
}

#[test]
fn no_telegram_file_exceeds_the_line_budget() {
    let dir = telegram_dir();
    let entries = std::fs::read_dir(&dir).expect("đọc `src/telegram`");
    let mut over = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let count = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("không đọc được `{name}`: {error}"))
            .lines()
            .count();
        if count >= FILE_LINE_BUDGET {
            over.push(format!("{name} ({count} dòng)"));
        }
    }
    assert!(
        over.is_empty(),
        "file vượt trần {FILE_LINE_BUDGET} dòng: {over:?}. \
         Tách tiếp theo ranh giới loại công việc."
    );
}

#[test]
fn telegram_keeps_its_extracted_modules() {
    let modules = count_rs(&telegram_dir());
    assert!(
        modules >= MIN_TELEGRAM_MODULES,
        "`src/telegram/` chỉ còn {modules} module (tối thiểu {MIN_TELEGRAM_MODULES}). \
         Đừng gộp ngược về một file."
    );
}

/// Doc của `mod.rs` phải mô tả cây module.
#[test]
fn mod_doc_documents_the_module_tree() {
    let text = std::fs::read_to_string(telegram_dir().join("mod.rs")).expect("đọc `mod.rs`");
    for name in MOD_DOC_MUST_MENTION {
        assert!(
            text.contains(name),
            "`mod.rs` không nhắc tới `{name}` — bản đồ module đã lỗi thời."
        );
    }
}

/// Seam `TelegramTransport` phải tồn tại — đó là thứ cho phép test kiểm tra allowlist
/// và confirm **không cần mạng, không cần bot token**.
///
/// Mất seam này thì mọi test buộc phải gọi Telegram thật: chậm, tốn tiền, và **flaky**
/// — một test bằng chứng duy nhất cho quy tắc an toàn sẽ hỏng theo hạ tầng của nhà cung
/// cấp. Đây là lý do test không được "tạm" gọi mạng.
#[test]
fn transport_seam_stays_in_types_module() {
    let types = std::fs::read_to_string(telegram_dir().join("types.rs")).expect("đọc `types.rs`");
    assert!(
        types.contains("pub trait TelegramTransport"),
        "`types.rs` mất `trait TelegramTransport` — test sẽ phải gọi Telegram thật."
    );
}
