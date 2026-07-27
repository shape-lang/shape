// Charter workload: strings — build, split, case-fold, filter, re-join
// (Node reference).
function makeRecord(i) {
    return `id${i},name-${i},${i * 7},tag${i % 13}`;
}

function transform(rounds, perRound) {
    let total = 0;
    for (let r = 0; r < rounds; r++) {
        for (let i = 0; i < perRound; i++) {
            const rec = makeRecord(i);
            const fields = rec.split(",");
            const upper = fields[1].toUpperCase();
            const rebuilt = fields[0] + "|" + upper + "|" + fields[3];
            total = total + rebuilt.length;
        }
    }
    return total;
}

console.log(transform(192, 2000));
