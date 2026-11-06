p = 'crates/bean/src/setup.rs'
s = open(p).read()

# 1. snapshot.as_ref() -> Some(&snapshot)
old = 'bean_settings::env_first_lookup(snapshot.as_ref())'
new = 'bean_settings::env_first_lookup(Some(&snapshot))'
assert s.count(old) == 1, ('count1', s.count(old))
s = s.replace(old, new, 1)

# 2. profile.provider, -> profile.provider.as_str(), (tất cả lần xuất hiện)
old = 'profile.provider,'
new = 'profile.provider.as_str(),'
assert s.count(old) == 2, ('count2', s.count(old))
s = s.replace(old, new, 2)

# 3. config.llm.model = model; -> clone (tránh borrow/move conflict)
old = 'config.llm.model = model;'
assert s.count(old) == 2, ('count3', s.count(old))
s = s.replace(old, 'config.llm.model = model.clone();', 2)

# 4. let active = config.llm.active_profile_name(); -> to_string()
old = 'let active = config.llm.active_profile_name();'
assert s.count(old) == 1, ('count4', s.count(old))
s = s.replace(old, 'let active = config.llm.active_profile_name().to_string();', 1)

open(p, 'w').write(s)
print('done')
