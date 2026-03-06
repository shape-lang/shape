function mandelbrot(size) {
    let count = 0;
    for (let y = 0; y < size; y++) {
        for (let x = 0; x < size; x++) {
            const cr = 2.0 * x / size - 1.5;
            const ci = 2.0 * y / size - 1.0;
            let zr = 0, zi = 0, i = 0;
            while (i < 50) {
                const tr = zr * zr - zi * zi + cr;
                zi = 2.0 * zr * zi + ci;
                zr = tr;
                if (zr * zr + zi * zi > 4.0) break;
                i++;
            }
            if (i === 50) count++;
        }
    }
    return count;
}
console.log(mandelbrot(4000));
