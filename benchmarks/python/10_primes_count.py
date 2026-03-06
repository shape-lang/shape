def is_prime(n):
    if n < 2: return False
    if n < 4: return True
    if n % 2 == 0: return False
    d = 3
    while d * d <= n:
        if n % d == 0: return False
        d += 2
    return True

def count_primes(limit):
    count = 0
    for n in range(2, limit):
        if is_prime(n):
            count += 1
    return count
print(count_primes(10000000))
