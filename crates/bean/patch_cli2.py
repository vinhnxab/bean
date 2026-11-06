p = 'crates/bean/src/cli.rs'
s = open(p).read()
# Xóa bản Secret trùng ngay sau Mcp(McpArgs),
old = 'Mcp(McpArgs),\n    Secret(SecretArgs),\n    Secret(SecretArgs),'
new = 'Mcp(McpArgs),\n    Secret(SecretArgs),'
assert s.count(old) == 1, ('count1', s.count(old))
s = s.replace(old, new, 1)
# Thêm Config/Doctor/Setup/Model sau Secret(SecretArgs), trước } đóng enum
old2 = 'Secret(SecretArgs),\n}'
assert s.count(old2) == 1, ('count2', s.count(old2))
new2 = '''Secret(SecretArgs),
    Config(ConfigArgs),

    /// Kiểm tra sức khoẻ cấu hình: quyền file, key sai chỗ, slot bị env che, proxy
    /// (K29), tiến trình/port (K27), sandbox Docker, Chrome snap — không bao giờ in
    /// giá trị secret. Mã thoát: 0 ok, 1 lỗi, 2 cảnh báo.
    Doctor {
        /// Thử request tối thiểu tới LLM/voice/Telegram/web search (tốn một call nhỏ,
        /// opt-in).
        #[arg(long)]
        online: bool,
    },

    /// Cài đặt tương tác ban đầu (idempotent): nhà cung cấp + key, mật khẩu web,
    /// Telegram, tìm kiếm web, giọng nói; mỗi bước có thể bỏ qua.
    Setup,

    /// Chọn nhà cung cấp/mô hình tương tác, lưu vào `llm-state.json` (cùng nơi
    /// với giao diện web) và áp dụng khi khởi động lại Bean.
    Model,
}'''
assert s.count(old2) == 1
s = s.replace(old2, new2, 1)
open(p, 'w').write(s)
print('done')
