p = 'crates/bean/src/setup.rs'
s = open(p).read()

old = 'profile.provider,'
new = 'profile.provider.as_str(),'
assert s.count(old) == 2, ('count1', s.count(old))
s = s.replace(old, new, 2)

old = 'config.llm.model = model;'
new = 'config.llm.model = model.clone();'
assert s.count(old) == 2, ('count2', s.count(old))
s = s.replace(old, new, 2)

old = 'let active = config.llm.active_profile_name();'
new = 'let active = config.llm.active_profile_name().to_string();'
assert s.count(old) == 1, ('count3', s.count(old))
s = s.replace(old, new, 1)

old = '"không cần key"'
new = '"không cần key".to_string()'
assert s.count(old) == 1, ('count4', s.count(old))
s = s.replace(old, new, 1)

open(p, 'w').write(s)
print('done')
