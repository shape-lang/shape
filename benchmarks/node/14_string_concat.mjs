function string_concat(n) {
    let s = "";
    for (let i = 0; i < n; i++) {
        s += "x";
    }
    return s.length;
}
console.log(string_concat(100000));
