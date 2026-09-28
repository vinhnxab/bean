//! Chuẩn hoá JSON Schema của tool cho từng provider (agents.md mục 5).
//!
//! Từ M3, `schemars` (draft 2020-12) sinh schema với `$defs` + `$ref`. Anthropic và nhiều
//! server OpenAI-compat không chấp nhận `$defs`/`$schema`, nên lớp này:
//! 1. inline mọi `$ref: "#/$defs/…"` thành schema thật (có giới hạn độ sâu chống `$ref` đệ quy);
//! 2. xoá `$schema`, `$defs`, `$id`;
//! 3. bảo đảm root khai báo `"type": "object"` (Anthropic yêu cầu).

use serde_json::{Map, Value};

/// Độ sâu inline tối đa — chống schema có `$ref` đệ quy làm treo.
const MAX_INLINE_DEPTH: usize = 16;

/// Chuẩn hoá schema **tại chỗ** (mutate) để gửi provider.
pub fn sanitize_tool_schema(schema: &mut Value) {
    let defs = collect_defs(schema);
    inline_refs(schema, &defs, 0);
    if let Some(obj) = schema.as_object_mut() {
        obj.remove("$schema");
        obj.remove("$defs");
        obj.remove("$id");
        if !obj.contains_key("type") {
            obj.insert("type".to_string(), Value::String("object".to_string()));
        }
    }
}

/// Lấy bản sao `$defs` **trước** khi mutate (các `$ref` trỏ vào đây).
fn collect_defs(schema: &Value) -> Map<String, Value> {
    schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// Thay `$ref` bằng schema thật rồi đi tiếp vào giá trị thay thế (nó có thể chứa `$ref` khác).
fn inline_refs(value: &mut Value, defs: &Map<String, Value>, depth: usize) {
    if depth > MAX_INLINE_DEPTH {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                inline_refs(item, defs, depth + 1);
            }
        }
        Value::Object(obj) => {
            if let Some(Value::String(reference)) = obj.get("$ref")
                && let Some(name) = reference.strip_prefix("#/$defs/")
                && let Some(def) = defs.get(name)
            {
                let mut replacement = def.clone();
                inline_refs(&mut replacement, defs, depth + 1);
                *value = replacement;
                return;
            }
            for child in obj.values_mut() {
                inline_refs(child, defs, depth + 1);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use serde_json::json;

    use super::sanitize_tool_schema;

    #[test]
    fn inlines_defs_and_strips_meta() {
        let mut schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": { "Address": { "type": "object", "properties": { "city": { "type": "string" } } } },
            "type": "object",
            "properties": {
                "home": { "$ref": "#/$defs/Address" },
                "work": { "$ref": "#/$defs/Address" }
            }
        });
        sanitize_tool_schema(&mut schema);
        assert!(schema.get("$schema").is_none());
        assert!(schema.get("$defs").is_none());
        let home = schema
            .get("properties")
            .and_then(|p| p.get("home"))
            .unwrap();
        assert_eq!(
            home.get("properties").and_then(|p| p.get("city")),
            Some(&json!({ "type": "string" }))
        );
        // Cả hai $ref đều được thay (không dùng chung tham chiếu nữa).
        let work = schema
            .get("properties")
            .and_then(|p| p.get("work"))
            .unwrap();
        assert!(work.get("properties").is_some());
    }

    #[test]
    fn nested_ref_is_inlined() {
        let mut schema = json!({
            "$defs": { "Id": { "type": "string", "minLength": 1 } },
            "type": "object",
            "properties": {
                "user": { "type": "object", "properties": { "id": { "$ref": "#/$defs/Id" } } }
            }
        });
        sanitize_tool_schema(&mut schema);
        let id = schema.pointer("/properties/user/properties/id").unwrap();
        assert_eq!(id.get("minLength"), Some(&json!(1)));
        assert!(id.get("$ref").is_none());
    }

    #[test]
    fn missing_type_defaults_to_object() {
        let mut schema = json!({ "properties": { "path": { "type": "string" } } });
        sanitize_tool_schema(&mut schema);
        assert_eq!(schema.get("type"), Some(&json!("object")));
    }

    #[test]
    fn recursive_ref_does_not_hang() {
        let mut schema = json!({
            "$defs": { "Node": { "type": "object", "properties": { "child": { "$ref": "#/$defs/Node" } } } },
            "type": "object",
            "properties": { "root": { "$ref": "#/$defs/Node" } }
        });
        sanitize_tool_schema(&mut schema);
        // Không treo và vẫn là JSON hợp lệ (đạt trần độ sâu thì giữ nguyên $ref).
        assert!(schema.is_object());
        let root = schema.pointer("/properties/root").unwrap();
        assert!(root.is_object());
    }
}
