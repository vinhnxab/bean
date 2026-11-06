p = 'crates/bean/src/cli.rs'
s = open(p).read()
# 找到 'pub command: SecretCommandAction,\n' 后面应该跟着 '}'
old = '    provider: SecretCommandAction,\n    }\n\n/// Quản lý kho secret thống nhất (M29).\n#[derive(Debug, Args)]\npub struct SecretSlotRegistry {'
# 等等，让我先看看实际内容
print(s[s.index('pub command: SecretCommandAction,'):s.index('pub command: McpTokenCommand,\n')+50])
