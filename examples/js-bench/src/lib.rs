use base32768;
use js_sys::{Date, JsString, Uint8Array};
use std::{
    hint::black_box,
    io::{Read, Write},
};
use wasm_bindgen::prelude::*;
use web_sys::console;

#[cfg(target_family = "wasm")]
#[global_allocator]
static ALLOCATOR: talc::Talck<talc::locking::AssumeUnlockable, talc::ClaimOnOom> = {
    static mut MEMORY: [std::mem::MaybeUninit<u8>; 128 * 1024 * 1024] =
        [std::mem::MaybeUninit::uninit(); 128 * 1024 * 1024];
    let span = talc::Span::from_base_size(unsafe { MEMORY.as_ptr() as *mut _ }, 128 * 1024 * 1024);
    talc::Talc::new(unsafe { talc::ClaimOnOom::new(span) }).lock()
};

#[wasm_bindgen]
extern "C" {
    fn arr_to_str(arr: &[u16]) -> JsString;
    fn str_to_arr(s: &JsString, arr: &mut [u16]) -> u32;
}

#[wasm_bindgen]
pub fn test_codecs(random_bytes: Uint8Array) {
    let local_bytes = random_bytes.to_vec();
    let jasper_encode_time = bench_jasper_encode(&local_bytes);
    let jasper_decode_time = bench_jasper_decode(&local_bytes);
    let optimized_encode_time = bench_optimized_write_encode(&local_bytes);
    let optimized_decode_time = bench_optimized_read_decode(&local_bytes);
    console::log_1(&JsValue::from_str("Runtimes:"));
    console::log_1(&JsValue::from_str(&format!(
        "Jasper encode:    {:.3}ms/iter",
        jasper_encode_time
    )));
    console::log_1(&JsValue::from_str(&format!(
        "Jasper decode:    {:.3}ms/iter",
        jasper_decode_time
    )));
    console::log_1(&JsValue::from_str(&format!(
        "Optimized encode: {:.3}ms/iter",
        optimized_encode_time
    )));
    console::log_1(&JsValue::from_str(&format!(
        "Optimized decode: {:.3}ms/iter",
        optimized_decode_time
    )));
}

fn bench_jasper_encode(bytes: &[u8]) -> f64 {
    let start = Date::now();
    for _ in 0..100 {
        let char_codes = black_box(base32768::alternative::encode(black_box(bytes)));
        black_box(arr_to_str(&char_codes));
    }
    (Date::now() - start) / 100.0
}

fn bench_jasper_decode(bytes: &[u8]) -> f64 {
    let encoded = base32768::alternative::encode(bytes)
        .chunks(64)
        .map(JsString::from_char_code)
        .reduce(|a, b| a.concat(&b))
        .unwrap();
    let local_str: String = encoded.clone().into();
    let mut local_encoded: Vec<u16> = local_str.encode_utf16().collect();
    let start = Date::now();
    for _ in 0..100 {
        local_encoded.reserve(encoded.length() as usize);
        unsafe {
            local_encoded.set_len(encoded.length() as usize);
        }
        assert_eq!(encoded.length(), str_to_arr(&encoded, &mut local_encoded));
        black_box(base32768::alternative::decode(&black_box(std::mem::take(
            &mut local_encoded,
        ))));
    }
    (Date::now() - start) / 100.0
}

fn bench_optimized_write_encode(bytes: &[u8]) -> f64 {
    let start = Date::now();
    let mut output = Vec::new();
    for _ in 0..100 {
        let mut writer = black_box(base32768::optimized::WriteEncoder::new_by_ref(&mut output));
        writer.write_all(black_box(bytes)).unwrap();
        writer.finish();
        black_box(arr_to_str(&output));
        output.clear();
    }
    (Date::now() - start) / 100.0
}

fn bench_optimized_read_decode(bytes: &[u8]) -> f64 {
    let mut writer = base32768::optimized::WriteEncoder::new(
        base32768::optimized::BufferedJsString::<1024>::new(),
    );
    writer.write_all(bytes).unwrap();
    let encoded = writer.finish().finish();
    let mut local_encoded = Vec::with_capacity(encoded.length() as usize);
    let mut decoded = Vec::with_capacity(bytes.len());
    let start = Date::now();
    for _ in 0..100 {
        decoded.clear();
        local_encoded.reserve(encoded.length() as usize);
        unsafe {
            local_encoded.set_len(encoded.length() as usize);
        }
        assert_eq!(encoded.length(), str_to_arr(&encoded, &mut local_encoded));
        base32768::optimized::ReadDecoder::<_, 1920>::new(black_box(std::mem::take(
            &mut local_encoded,
        )))
        .read_to_end(black_box(&mut decoded))
        .unwrap();
    }
    (Date::now() - start) / 100.0
}
