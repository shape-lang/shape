class Particle:
    __slots__ = ('x', 'y', 'mass')
    def __init__(self, x, y, mass):
        self.x = x
        self.y = y
        self.mass = mass

def array_of_objects(n):
    particles = []
    for i in range(n):
        particles.append(Particle(i * 1.0, i * 2.0, i * 0.5))
    total_mass = 0.0
    for i in range(n):
        total_mass += particles[i].mass
    return total_mass
print(array_of_objects(100000))
