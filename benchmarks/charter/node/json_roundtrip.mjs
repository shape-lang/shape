// Charter workload: JSON — parse, navigate, re-serialize (Node reference).
function document(i) {
    return `{"id":${i},"name":"row-${i}","tags":[1,2,3],"nested":{"score":${i * 2},"ok":true}}`;
}

function roundtrip(rounds) {
    let total = 0;
    for (let i = 0; i < rounds; i++) {
        const text = document(i);
        const value = JSON.parse(text);
        const out = JSON.stringify(value);
        total = total + out.length;
    }
    return total;
}

console.log(roundtrip(60000));
