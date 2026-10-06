#![allow(clippy::panic, clippy::expect_used)]
//! Chặn hồi quy kiến trúc ở tầng bộ nhớ (đừng để God file quay lại).
//!
//! # Vì sao cần
//!
//! `store.rs` từng là **một** file 4879 dòng chứa ba bản của cùng một danh sách 51
//! thao tác (khai báo trait + impl `MemoryStore` + impl `SqliteStore`) cộng ~1100 dòng
//! SQL và 579 dòng test. Số dòng lớn **không** phải do Rust — đo thật thì comment chỉ
//! chiếm 6,7% file.
//!
//! Điều đáng lo là file đó **không có** lưới an toàn nào, trong khi `router.rs` đã có.
//! Đây là rủi ro khó bảo trì nhất: file lớn nhất lại ít được bảo vệ nhất.
//!
//! Việc tách là **thủ công** — không có gì tự ngăn file phình lại sau vài tính năng
//! mới. Test này **đo số dòng thật** và **đếm module thật**, nên lời hứa trong comment
//! không có tác dụng gì. Nâng hạn mức khi tách hợp lý — và khi đó sửa cả hằng số lẫn
//! lý do.

use std::path::{Path, PathBuf};

/// Trần số dòng cho **từng** file `.rs` dưới `src/store/`.
/// File lớn nhất sau khi tách là `memory.rs` (~910 dòng) — dưới trần này.
const FILE_LINE_BUDGET: usize = 1000;

/// Số module tối thiểu trực tiếp dưới `src/store/`.
/// Mỗi module là **một trách nhiệm đã tách**; gộp lại về một file là đi hướng ngược.
const MIN_STORE_MODULES: usize = 8;

/// Số module tối thiểu dưới `src/store/sqlite/`.
const MIN_SQLITE_MODULES: usize = 12;

/// Số file truy vấn (`q_*.rs`) tối thiểu — mỗi file một nhóm nghiệp vụ.
const MIN_QUERY_GROUPS: usize = 7;

/// Trần số dòng cho **mỗi** file truy vấn; lớn nhất là `q_session.rs` (~256).
const QUERY_LINE_BUDGET: usize = 300;

/// `mod.rs` phải nhắc tới các module này trong doc — bản đồ để người mới đọc.
const MOD_DOC_MUST_MENTION: &[&str] = &[
    "trait_def.rs",
    "types.rs",
    "compaction.rs",
    "memory.rs",
    "sqlite/",
];

/// Thư mục gốc `src/store`.
fn store_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/store")
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

/// Các file trong `dir` vượt `budget`, dạng `"tên (N dòng)"`.
fn over_budget(dir: &Path, budget: usize, prefix: Option<&str>) -> Vec<String> {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("không đọc được `{}`: {error}", dir.display()));
    let mut over = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        if prefix.is_some_and(|p| !name.starts_with(p)) {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("không đọc được `{name}`: {error}"));
        let count = text.lines().count();
        if count >= budget {
            over.push(format!("{name} ({count} dòng)"));
        }
    }
    over
}
#[test]
fn no_store_file_exceeds_the_line_budget() {
    let over = over_budget(&store_dir(), FILE_LINE_BUDGET, None);
    assert!(
        over.is_empty(),
        "file vượt trần {FILE_LINE_BUDGET} dòng: {over:?}. \
         Tách tiếp theo ranh giới nghiệp vụ thay vì nhét thêm vào file cũ."
    );
}

#[test]
fn store_keeps_its_extracted_modules() {
    let modules = count_rs(&store_dir());
    assert!(
        modules >= MIN_STORE_MODULES,
        "`src/store/` chỉ còn {modules} module (tối thiểu {MIN_STORE_MODULES}). \
         Đừng gộp các module đã tách ngược về một file."
    );
}

#[test]
fn sqlite_keeps_its_extracted_modules() {
    let modules = count_rs(&store_dir().join("sqlite"));
    assert!(
        modules >= MIN_SQLITE_MODULES,
        "`src/store/sqlite/` chỉ còn {modules} module (tối thiểu {MIN_SQLITE_MODULES})."
    );
}

#[test]
fn queries_stay_split_by_business_domain() {
    let entries = std::fs::read_dir(store_dir().join("sqlite")).expect("đọc `sqlite/`");
    let groups: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("q_").then_some(name)
        })
        .collect();
    assert!(
        groups.len() >= MIN_QUERY_GROUPS,
        "chỉ còn {} nhóm truy vấn ({groups:?}), tối thiểu {MIN_QUERY_GROUPS}. \
         Truy vấn phải tách theo nhóm nghiệp vụ, không gộp hết vào một file.",
        groups.len()
    );
}

#[test]
fn no_query_file_exceeds_the_line_budget() {
    let over = over_budget(&store_dir().join("sqlite"), QUERY_LINE_BUDGET, Some("q_"));
    assert!(
        over.is_empty(),
        "file truy vấn vượt trần {QUERY_LINE_BUDGET} dòng: {over:?}. \
         Tách theo nhóm nghiệp vụ tiếp theo."
    );
}

/// Doc của `mod.rs` phải mô tả cây module — bản đồ để người mới đọc không phải dò file.
#[test]
fn mod_doc_documents_the_module_tree() {
    let text = std::fs::read_to_string(store_dir().join("mod.rs")).expect("đọc `mod.rs`");
    for name in MOD_DOC_MUST_MENTION {
        assert!(
            text.contains(name),
            "`mod.rs` không nhắc tới `{name}` — bản đồ module đã lỗi thời."
        );
    }
}

/// `now_rfc3339` phải nằm ở `shared.rs`, **không** nhân bản trong `sqlite/`.
///
/// Nhân bản là cách hỏng âm thầm đáng sợ nhất ở đây: hai bản ghi timestamp lệch định
/// dạng rồi so sánh chuỗi trong SQL cho kết quả sai mà không có test nào đỏ.
#[test]
fn timestamp_helper_lives_in_exactly_one_place() {
    let root = store_dir();
    let mut found: Vec<String> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("đọc file .rs");
            if text.contains("fn now_rfc3339(") {
                found.push(
                    path.strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    assert_eq!(
        found,
        vec![String::from("shared.rs")],
        "`now_rfc3339` phải tồn tại đúng một chỗ (`shared.rs`), tìm thấy: {found:?}. \
         Hai bản cài đặt phải ghi timestamp cùng định dạng."
    );
}
