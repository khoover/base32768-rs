require("./EncoderDecoderTogether.src")
fromCharCode = String.fromCharCode.bind(String)
function arr_to_str(uint16arr) {
    let s = "";
    for (let i = 0; i < uint16arr.length; i += 256) {
        s += fromCharCode.apply(null, uint16arr.subarray(i, i + 256));
    }
    return s;
}

function str_to_arr(s, arr) {
    const to_copy = s.length < arr.length ? s.length : arr.length;
    for (let i = 0; i < to_copy; ++i) {
        arr[i] = s.charCodeAt(i);
    }
    return to_copy;
}
global.arr_to_str = arr_to_str;
global.str_to_arr = str_to_arr;
const start = Date.now();
const { test_codecs } = require("./pkg/js_bench");
console.log(`Took ${Date.now() - start} ms to load and init bench wasm`);
const crypto = require("crypto");

const arr = new Uint8Array(1000000);
crypto.randomFillSync(arr);
test_codecs(arr);