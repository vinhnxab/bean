p = 'crates/bean/src/cli.rs'
s = open(p).read()
idx = s.index('pub command: McpTokenCommand,\n')
# 检查后面是否已经有 }
if s[idx + len(marker):idx + len(marker) + 1] != '}':
    s = s[:idx + len(marker)] + '}\n' + s[idx + len(marker):]
    open(p, 'w').write(s)
    print('Fixed: added } after pub command: McpTokenCommand,')
else:
    print('Already correct')
