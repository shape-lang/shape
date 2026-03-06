def collatz_len(n):
    count = 0
    x = n
    while x != 1:
        if x % 2 == 0:
            x = x // 2
        else:
            x = 3 * x + 1
        count += 1
    return count

def longest_collatz(limit):
    best = 0
    for n in range(2, limit):
        l = collatz_len(n)
        if l > best:
            best = l
    return best
print(longest_collatz(1000000))
