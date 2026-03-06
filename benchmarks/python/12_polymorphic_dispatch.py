class Linear:
    __slots__ = ('slope', 'intercept')
    def __init__(self, slope, intercept):
        self.slope = slope
        self.intercept = intercept

class Quadratic:
    __slots__ = ('a', 'b', 'c')
    def __init__(self, a, b, c):
        self.a = a
        self.b = b
        self.c = c

class Cubic:
    __slots__ = ('a', 'b', 'c', 'd')
    def __init__(self, a, b, c, d):
        self.a = a
        self.b = b
        self.c = c
        self.d = d

def polymorphic_dispatch(n):
    lin = Linear(2.0, 1.0)
    quad = Quadratic(1.0, 2.0, 3.0)
    cub = Cubic(0.5, 1.0, 1.5, 2.0)
    s = 0.0
    i = 0
    while i < n:
        x = (i % 100) * 1.0
        r = i % 3
        if r == 0:
            s += lin.slope * x + lin.intercept
        elif r == 1:
            s += quad.a * x * x + quad.b * x + quad.c
        else:
            s += cub.a * x * x * x + cub.b * x * x + cub.c * x + cub.d
        i += 1
    return s
print(polymorphic_dispatch(5000000))
