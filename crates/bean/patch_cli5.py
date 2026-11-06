p = 'crates/bean/src/cli.rs'
s = open(p).read()
start = s.index('pub struct McpTokenArgs {\n')
cmd_start = s.index('pub command: McpTokenCommand,\n', start)
cmd_end = s.index('}', cmd_start)  # '}' đóng struct McpTokenArgs
block = s[cmd_start + len('pub command: McpTokenCommand,\n'):cmd_end]
s = s[:block_start] if False else s  # no-op placeholder
# chèn block mới ngay sau '}' đóng McpTokenArgs
insert_at = cmd_end + 1
s = s[:insert_at] + block + s[insert_at:]
open(p, 'w').write(s)
print('done')
