def mat_mul(n):
    a = [float(i) for i in range(n * n)]
    b = [float(n * n - i) for i in range(n * n)]
    c = [0.0] * (n * n)
    for i in range(n):
        for j in range(n):
            s = 0.0
            for k in range(n):
                s += a[i * n + k] * b[k * n + j]
            c[i * n + j] = s
    return c[0]
print(mat_mul(800))
