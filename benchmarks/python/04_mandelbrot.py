def mandelbrot(size):
    count = 0
    for y in range(size):
        for x in range(size):
            cr = 2.0 * x / size - 1.5
            ci = 2.0 * y / size - 1.0
            zr = zi = 0.0
            i = 0
            while i < 50:
                tr = zr * zr - zi * zi + cr
                zi = 2.0 * zr * zi + ci
                zr = tr
                if zr * zr + zi * zi > 4.0:
                    break
                i += 1
            if i == 50:
                count += 1
    return count
print(mandelbrot(4000))
