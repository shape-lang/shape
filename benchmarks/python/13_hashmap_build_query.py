def hashmap_build_query(n):
    m = {}
    for i in range(n):
        m[i] = i * 2.5
    s = 0.0
    for i in range(n):
        v = m.get(i)
        if v is not None:
            s += v
    return s
print(hashmap_build_query(100000))
