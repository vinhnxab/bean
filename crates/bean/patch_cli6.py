p = 'crates/bean/src/cli.rs'
s = open(p).read()
marker = 'pub command: McpTokenCommand,\n'
idx = s.index(marker)
s = s[:idx + len(marker)] + '}\n' + s[idx + len(marker):]
open(p, 'w').write(s)
print('done')
