def spectral(n):
    u = [1.0] * n
    v = [0.0] * n
    for _ in range(10):
        for i in range(n):
            s = 0.0
            for j in range(n):
                s += u[j] / ((i + j) * (i + j + 1) / 2 + i + 1)
            v[i] = s
        for i in range(n):
            s = 0.0
            for j in range(n):
                s += v[j] / ((i + j) * (i + j + 1) / 2 + i + 1)
            u[i] = s
    print(u[0])
spectral(5000)
