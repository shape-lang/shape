class Point5D:
    __slots__ = ('x', 'y', 'z', 'w', 'v')
    def __init__(self, x, y, z, w, v):
        self.x = x
        self.y = y
        self.z = z
        self.w = w
        self.v = v

def object_property_loop(n):
    p = Point5D(1.0, 2.0, 3.0, 4.0, 5.0)
    s = 0.0
    i = 0
    while i < n:
        s += p.x + p.y + p.z + p.w + p.v
        i += 1
    return s
print(object_property_loop(10000000))
