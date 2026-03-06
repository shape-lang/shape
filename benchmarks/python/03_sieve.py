def sieve(n):
    flags = [True] * (n + 1)
    p = 2
    while p * p <= n:
        if flags[p]:
            j = p * p
            while j <= n:
                flags[j] = False
                j += p
        p += 1
    return sum(1 for i in range(2, n + 1) if flags[i])
print(sieve(10000000))
