def sum_to(n):
    s = 0
    i = 0
    while i < n:
        s += i
        i += 1
    return s
print(sum_to(1000000000))
