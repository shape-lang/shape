// Charter workload: numeric kernel — Mandelbrot escape-time (Node reference).
function mandelbrot(width, height, maxIter) {
    let total = 0;
    for (let py = 0; py < height; py++) {
        const y0 = (py / height) * 2.0 - 1.0;
        for (let px = 0; px < width; px++) {
            const x0 = (px / width) * 3.0 - 2.0;
            let x = 0.0, y = 0.0, it = 0;
            while (it < maxIter) {
                const x2 = x * x;
                const y2 = y * y;
                if (x2 + y2 > 4.0) break;
                y = 2.0 * x * y + y0;
                x = x2 - y2 + x0;
                it = it + 1;
            }
            total = total + it;
        }
    }
    return total;
}

console.log(mandelbrot(420, 420, 200));
