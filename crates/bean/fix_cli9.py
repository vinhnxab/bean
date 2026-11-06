p = 'crates/bean/src/cli.rs'
s = open(p).read()
# 找到 'pub command: McpTokenCommand,\n}' 后面是 '}}\n}\n' 而不是 '}\n'
old = 'pub command: McpTokenCommand,\n}\n}\n}'
new = 'pub command: McpTokenCommand,\n}\n'
assert s.count(old) == 1, f'count={s.count(old)}'
s = s.replace(old, new, 1)
open(p, 'w').write(s)
print('Fixed!')
print(repr(s[idx:idx+60]) if (idx := s.index('pub command: McpTokenCommand,\n')) >= 0 else 'not found')
