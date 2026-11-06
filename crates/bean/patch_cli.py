p = 'crates/bean/src/cli.rs'
s = open(p).read()
old = 'Mcp(McpArgs),'
assert s.count(old) == 1, ('count', s.count(old))
idx = s.index(old)
s = s[:idx + len(old)] + '\n    Secret(SecretArgs),' + s[idx + len(old):]
open(p, 'w').write(s)
print('done')
