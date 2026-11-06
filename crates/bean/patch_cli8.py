p = 'crates/bean/src/cli.rs'
s = open(p).read()

# 找到 'pub command: McpTokenCommand,\n}' 并确保后面只有一个 '}\n'
old = 'pub command: McpTokenCommand,\n}\n}'
new = 'pub command: McpTokenCommand,\n}'

count = s.count(old)
print(f'找到 {count} 处 "pub command: McpTokenCommand,\n}\\n}}"')

if count > 0:
    s = s.replace(old, new, 1)
    open(p, 'w').write(s)
    print('已修复 McpTokenArgs 结构体闭合')

# 再检查是否还有问题
print('\\n检查修复后的结构:')
for i, line in enumerate(s.split('\\n')[:120], 1):
    if 114 <= i <= 125:
        print(f'{i}: {repr(line)}')
