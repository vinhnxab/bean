//! Ngân sách token cho khối **ảnh** (M26).
//!
//! Bằng chứng bắt buộc: ảnh KHÔNG bị tính bằng công thức `chars/4` của text, và
//! việc chụp một màn hình không loại sạch lịch sử của lượt đó.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

use bean_core::context::{IMAGE_BUDGET_TOKENS, message_token_cost, trim_history};
use bean_types::{ImageBlock, Message, Role};

fn image(data_len: usize) -> ImageBlock {
    ImageBlock::new("image/png", "A".repeat(data_len), "abc123").expect("media type hợp lệ")
}

/// Ảnh nhỏ và ảnh rất lớn phải có **cùng** chi phí — định phí là hằng số, không
/// phụ thuộc kích thước.
#[test]
fn image_cost_is_flat_regardless_of_size() {
    // Dùng **cùng một caption** cho cả hai: `message_token_cost` cộng cả phần văn
    // bản (công thức `chars/4` nguyên vẹn), nên caption khác nhau sẽ làm tổng lệch
    // và che mất điều đang kiểm.
    const CAPTION: &str = "Ảnh chụp màn hình";
    let small = Message::tool_with_image("c1", CAPTION, image(1_000));
    let huge = Message::tool_with_image("c2", CAPTION, image(2_000_000));
    let small_cost = message_token_cost(&small);
    let huge_cost = message_token_cost(&huge);
    assert_eq!(
        small_cost, huge_cost,
        "ảnh 2 MB không được tốn gấp 2000 lần ảnh 1 KB"
    );

    // Tổng = phần văn bản + hằng ảnh. Tách riêng từng phần để test chứng minh đúng
    // cái cần chứng minh, không chỉ "tổng có vẻ hợp lý".
    let text_only = message_token_cost(&Message::tool("c0", CAPTION));
    assert_eq!(
        huge_cost - text_only,
        usize::try_from(IMAGE_BUDGET_TOKENS).expect("vừa usize"),
        "phần ảnh phải đúng bằng hằng IMAGE_BUDGET_TOKENS, không phụ thuộc kích thước"
    );
}

/// Đây là test **quyết định** cho toàn bộ thiết kế: nếu ảnh bị tính theo
/// `chars/4`, một ảnh 200 KB sẽ thành ~67.500 token và đẩy hết lịch sử ra khỏi
/// ngân sách — chính là lỗi mà `IMAGE_BUDGET_TOKENS` sinh ra để tránh.
#[test]
fn image_is_not_charged_by_the_chars_over_four_formula() {
    // 200.000 ký tự base64 — ảnh PNG khoảng 150 KB, rất bình thường.
    let base64_len = 200_000;
    const CAPTION: &str = "Ảnh chụp màn hình";
    let message = Message::tool_with_image("c1", CAPTION, image(base64_len));
    let naive_chars_over_four = base64_len / 4;

    let actual = message_token_cost(&message);
    assert!(
        actual < naive_chars_over_four / 10,
        "ảnh đang bị tính gần như theo chars/4: thật {actual}, chars/4 sẽ là {naive_chars_over_four}"
    );

    // So với message **không ảnh** cùng caption: chênh lệch phải đúng bằng hằng
    // ảnh, không phải bằng `chars/4` của payload.
    let text_only = message_token_cost(&Message::tool("c0", CAPTION));
    assert_eq!(
        actual - text_only,
        usize::try_from(IMAGE_BUDGET_TOKENS).expect("vừa usize"),
        "ảnh phải tốn đúng hằng số, không phải theo độ dài base64"
    );
}

/// Message không có ảnh thì chi phí **không đổi** so với trước M26.
#[test]
fn text_only_message_cost_is_unchanged_by_the_image_feature() {
    let text = Message::tool("c1", "x".repeat(4_000));
    // 4.000 ký tự / 4 = 1.000 token — công thức chars/4 nguyên vẹn.
    assert_eq!(message_token_cost(&text), 1_000);
}

/// Một lượt có ảnh + lời thoại vẫn giữ được lịch sử khi ngân sách vừa phải.
#[test]
fn one_screenshot_does_not_evict_the_whole_turn() {
    // 3 lượt hội thoại dài, mỗi lượt kèm một ảnh.
    let mut history: Vec<Message> = vec![Message::user("bắt đầu")];
    for step in 0..3 {
        history.push(Message::assistant(
            Some(format!("trả lời bước {step}")),
            vec![bean_types::ToolCall::new(
                format!("c{step}"),
                "browser_screenshot",
                serde_json::json!({}),
            )],
        ));
        history.push(Message::tool_with_image(
            format!("c{step}"),
            "Ảnh chụp màn hình — 120 KB, image/png",
            image(160_000),
        ));
    }

    // Ngân sách rộng: giữ hết.
    assert_eq!(trim_history(&history, 100_000).len(), history.len());

    // Ngân sách hẹp nhưng vẫn phải giữ được vài lượt gần nhất. Nếu ảnh bị tính bằng
    // chars/4, mỗi ảnh ~40.000 token và lịch sử sẽ bị cắt còn đúng một message.
    let budget = 10 * usize::try_from(IMAGE_BUDGET_TOKENS).expect("vừa usize") as u32;
    let kept = trim_history(&history, budget);
    assert!(
        kept.len() >= 5,
        "phải giữ được nhiều hơn một cặp tool, chỉ giữ {} message",
        kept.len()
    );
    // Bất biến cặp tool vẫn phải giữ (mục 22.1): không có tool result mồ côi.
    assert!(
        bean_memory::safe_cut::check_no_orphan_result(&kept, 0),
        "cắt lịch sử không được tách cặp tool_use/tool_result"
    );
    // Không được bắt đầu bằng một tool result mồ côi.
    assert_ne!(kept.first().map(|m| m.role), Some(Role::Tool));
}

/// Cắt lịch sử không được sinh `Message` mất ảnh ở phần giữ lại.
#[test]
fn retained_image_messages_keep_their_image_block() {
    let history = vec![
        Message::user("x"),
        Message::tool_with_image("c1", "ảnh", image(1_000)),
    ];
    let kept = trim_history(&history, 100_000);
    let with_image = kept.iter().find(|m| m.image.is_some());
    assert!(
        with_image.is_some(),
        "message ảnh phải còn nguyên khối image sau khi cắt"
    );
}
