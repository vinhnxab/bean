p = 'crates/bean/src/cli.rs'
s = open(p).read()
start = s.index('pub struct McpTokenArgs {\n')
cmd_start = s.index('pub command: McpTokenCommand,\n', start)
block_start = cmd_start + len('pub command: McpTokenCommand,\n')
cmd_end = s.index('}', cmd_start)  # '}' đóng struct McpTokenArgs
block = s[block_start:cmd_end]  # khối new_types bị nhầm vị trí
s = s[:block_start] + s[cmd_end+1:]  # gỡ block, giữ lại '}'
insert_at = block_start + 1  # ngay sau '}'
s = s[:insert_at] + block + s[insert_at:]
open(p, 'w').write(s)
print('done')
