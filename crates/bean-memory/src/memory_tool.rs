//! Tool `memory_save` và `memory_search` (agents.md mục 7.3, 8.4).
//!
//! Cả hai đều là `Risk::Safe` và chỉ chạm vào bộ nhớ của Bean (không đụng file,
//! không ra mạng). Doc comment của struct tham số **chính là** description gửi cho model
//! (agents.md mục 7.1) nên phải nói rõ khi nào dùng.

use std::sync::Arc;

use async_trait::async_trait;
use bean_tools::{Tool, ToolCtx, ToolError, typed};
use bean_types::{Risk, ToolSpec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::store::Store;

/// Lưu một thông tin bền vững vào bộ nhớ dài hạn của agent. Dùng khi người dùng nói một
/// sở thích, một quyết định, hoặc một dữ kiện cần nhớ ở các phiên hội thoại sau.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MemorySaveParams {
    /// Nội dung cần nhớ, viết thành một câu đầy đủ (plain text, không markdown).
    text: String,
    /// Nhãn phân loại cách nhau bởi dấu phẩy, ví dụ `sở thích,âm nhạc`. Để rỗng nếu chưa rõ.
    tags: String,
}

/// Tìm trong bộ nhớ dài hạn và lịch sử hội thoại (FTS5, xếp theo độ liên quan). Dùng
/// trước khi trả lời câu hỏi về thông tin đã trao đổi hoặc đã ghi nhớ trước đây.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MemorySearchParams {
    /// Từ khoá tìm kiếm; nhiều từ cách nhau bởi dấu cách đều phải xuất hiện.
    query: String,
}

/// `memory_save` — ghi vào bảng `memories`.
struct MemorySaveTool {
    store: Arc<dyn Store>,
}

#[async_trait]
impl Tool for MemorySaveTool {
    fn spec(&self) -> ToolSpec {
        typed::typed_spec::<MemorySaveParams>("memory_save")
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Safe
    }

    async fn call(&self, _ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let params = typed::deserialize_params::<MemorySaveParams>(args, &self.spec().parameters)
            .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;
        let id = self
            .store
            .memory_save(&params.text, &params.tags)
            .await
            .map_err(|err| ToolError::Internal(err.to_string()))?;
        Ok(format!("đã lưu ghi nhớ (id={id})."))
    }
}

/// `memory_search` — tìm trong `memories` và `messages`.
struct MemorySearchTool {
    store: Arc<dyn Store>,
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn spec(&self) -> ToolSpec {
        typed::typed_spec::<MemorySearchParams>("memory_search")
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Safe
    }

    async fn call(&self, _ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let params = typed::deserialize_params::<MemorySearchParams>(args, &self.spec().parameters)
            .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;
        let hits = self
            .store
            .memory_search(&params.query)
            .await
            .map_err(|err| ToolError::Internal(err.to_string()))?;
        if hits.is_empty() {
            return Ok(format!("không tìm thấy gì cho `{}`.", params.query));
        }
        let json =
            serde_json::to_string(&hits).map_err(|err| ToolError::Internal(err.to_string()))?;
        Ok(json)
    }
}

/// Đăng ký cả hai tool bộ nhớ dùng **cùng** một store.
#[must_use]
pub fn memory_tools(store: Arc<dyn Store>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(MemorySaveTool {
            store: store.clone(),
        }),
        Arc::new(MemorySearchTool { store }),
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::memory_tools;
    use crate::store::MemoryStore;
    use std::sync::Arc;

    #[test]
    fn specs_have_names_and_descriptions() {
        let tools = memory_tools(Arc::new(MemoryStore::new()));
        let mut names: Vec<String> = tools.iter().map(|tool| tool.spec().name).collect();
        names.sort();
        assert_eq!(names, vec!["memory_save", "memory_search"]);
        for tool in &tools {
            let spec = tool.spec();
            assert!(
                !spec.description.trim().is_empty(),
                "{} thiếu mô tả",
                spec.name
            );
            assert_eq!(spec.parameters["additionalProperties"], false);
        }
    }
}
