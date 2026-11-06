p = 'crates/bean/src/cli.rs'
s = open(p).read()
old = '    Secret(SecretArgs),\n    Config(ConfigArgs),\n'
new = '    Secret(SecretCommand),\n    Config(ConfigCommand),\n'
assert s.count(old) == 1, ('count1', s.count(old))
s = s.replace(old, new, 1)

old2 = 'pub struct McpTokenArgs {'
assert s.count(old2) == 1, ('count2', s.count(old2))
idx = s.index(old2)
end_idx = s.index('\n}', idx)
new_types = '''
/// Quản lý kho secret thống nhất `data.dir/secrets.env` (M29).
#[derive(Debug, Args)]
pub struct SecretCommand {
    #[command(subcommand)]
    command: SecretCommandAction,
}

#[derive(Debug, Subcommand)]
pub enum SecretCommandAction {
    /// Nhập ẩn API key vào kho secret.
    Set {
        /// Tên slot secret (ví dụ ANTHROPIC_API_KEY).
        name: String,
        /// Đọc giá trị từ stdin thay vì nhập ẩn từ terminal.
        #[arg(long)]
        stdin: bool,
    },
    /// Liệt kê các slot secret và trạng thái.
    List,
    /// Xoá một secret khỏi kho.
    Unset {
        /// Tên slot secret.
        name: String,
    },
}

/// Quản lý bảng thiết lập thống nhất (M29).
#[derive(Debug, Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigCommandAction,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommandAction {
    /// Di chuyển key mặc định từ bean.toml vào secrets.env.
    MigrateSecrets {
        /// Áp dụng thay đổi ngay (mặc định chỉ xem trước).
        #[arg(long)]
        apply: bool,
        /// Đọc key từ file ~/.config/bean/env thay vì nhập tay.
        #[arg(long)]
        from_env_file: Option<PathBuf>,
    },
    /// Liệt kê bảng thiết lập.
    List,
    /// Xem giá trị một khoá.
    Get {
        /// Đường dẫn khoá (ví dụ agent.timezone).
        key: String,
    },
    /// Ghi giá trị một khoá (chỉ tầng A).
    Set {
        /// Đường dẫn khoá.
        key: String,
        /// Giá trị mới.
        value: String,
    },
    /// Xoá ghi đè một khoá (về mặc định).
    Unset {
        /// Đường dẫn khoá.
        key: String,
    },
}

'''
s = s[:end_idx] + new_types + s[end_idx:]
open(p, 'w').write(s)
print('done')
