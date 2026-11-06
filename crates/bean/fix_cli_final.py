p = 'crates/bean/src/cli.rs'
s = open(p).read()

# 找到 pub struct McpTokenArgs { 的位置
start = s.index('pub struct McpTokenArgs {\n')
# 找到 pub command: McpTokenCommand,\n 的位置
cmd_start = s.index('pub command: McpTokenCommand,\n', start)
# 找到 cmd_start 后的第一个 }
cmd_end = s.index('}', cmd_start)  # 这应该是 McpTokenArgs 结构体的结束
print(f'cmd_start: {cmd_start}, cmd_end: {cmd_end}')
print(f'Context: {repr(s[cmd_start:cmd_end+1])}')

# 验证：s[cmd_end] 应该是 '}'
print(f'Char at cmd_end: {repr(s[cmd_end])}')

# 现在在 cmd_end 之后插入新的类型定义
new_types = '''
/// Secret command for managing API keys.
#[derive(Debug, Args)]
pub struct SecretCommand {
    #[command(subcommand)]
    command: SecretCommandAction,
}

#[derive(Debug, Subcommand)]
pub enum SecretCommandAction {
    /// Add a new secret key.
    Set {
        /// Secret slot name (e.g. ANTHROPIC_API_KEY).
        name: String,
        /// Read value from stdin instead of interactive prompt.
        #[arg(long)]
        stdin: bool,
    },
    /// List all secret slots and their status.
    List,
    /// Delete a secret from the vault.
    Unset {
        /// Secret slot name.
        name: String,
    },
}

/// Configuration command for managing unified settings (M29).
#[derive(Debug, Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigCommandAction,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommandAction {
    /// Migrate default keys from bean.toml to secrets.env.
    MigrateSecrets {
        /// Apply changes immediately (default is dry-run).
        #[arg(long)]
        apply: bool,
        /// Read key from ~/.config/bean/env instead of typing.
        #[arg(long)]
        from_env_file: Option<PathBuf>,
    },
    /// List all settings.
    List,
    /// Get value of a key.
    Get {
        /// Setting key path (e.g. agent.timezone).
        key: String,
    },
    /// Set value of a key (tier A only).
    Set {
        /// Setting key path.
        key: String,
        /// New value.
        value: String,
    },
    /// Unset override of a key (to default).
    Unset {
        /// Setting key path.
        key: String,
    },
}
'''

# 插入新代码在 McpTokenArgs 结构体关闭括号之后
s = s[:cmd_end+1] + new_types + s[cmd_end+1:]
open(p, 'w').write(s)
print('Fixed!')
