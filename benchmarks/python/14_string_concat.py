def string_concat(n):
    parts = []
    for i in range(n):
        parts.append("x")
    s = "".join(parts)
    return len(s)
print(string_concat(100000))
