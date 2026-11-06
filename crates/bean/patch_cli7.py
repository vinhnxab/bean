p = 'crates/bean/src/cli.rs'
s = open(p).read()
marker = 'pub command: McpTokenCommand,\n}'
idx = s.index(marker)
print(f'Found at {idx}: {repr(s[idx:idx+40])}')
# s[idx+len(marker):] 应该是 '}\n}\n\n/// Quản lý...'
# 我们要去掉第二个 '}\n'
if s[idx+len(marker):idx+len(marker)+2] == '}\n':
    s = s[:idx+len(marker)+2] + s[idx+len(marker)+4:]
    open(p, 'w').write(s)
    print('Fixed!')
    print(repr(s[idx:idx+40]))
else:
    print(f'Unexpected content after marker: {repr(s[idx+len(marker):idx+len(marker)+4])}')
