p = 'crates/bean/src/cli.rs'
s = open(p).read()
lines = s.split('\n')
# Line 119 is index 118 (0-based)
if '    }}}' in lines[118]:
    lines[118] = lines[118].replace('    }}}', '    }', 1)
    s = '\n'.join(lines)
    open(p, 'w').write(s)
    print('Fixed line 119')
    print(repr(lines[118]))
else:
    print(f'Line 118 content: {repr(lines[118])}')
