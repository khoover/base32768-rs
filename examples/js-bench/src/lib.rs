use base32768::{self, optimized::DecoderError};
use js_sys::{Date, JsString, Uint8Array};
use pipebuf::{PBufRd, PBufWr, PipeBuf};
use rand::{Rng, SeedableRng};
use std::{
    hint::black_box,
    io::{ErrorKind, Read, Write},
};
use wasm_bindgen::prelude::*;
use web_sys::console;

// #[cfg(target_family = "wasm")]
// #[global_allocator]
// static ALLOCATOR: talc::Talck<talc::locking::AssumeUnlockable, talc::WasmHandler> =
//     talc::Talc::new(unsafe { talc::WasmHandler::new() }).lock();

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
pub fn test_codecs(random_bytes: &Uint8Array) {
    let local_bytes = random_bytes.to_vec();
    let jasper_encode_time = bench_jasper_encode(&local_bytes);
    let jasper_decode_time = bench_jasper_decode(&local_bytes);
    let optimized_encode_time = bench_optimized_write_encode(&local_bytes);
    let optimized_decode_time = bench_optimized_read_decode(&local_bytes);
    let pipebuf_encode_time = bench_pipebuf_write_encode(&local_bytes);
    let pipebuf_into_encode_time = bench_pipebuf_write_encode_using_into(&local_bytes);
    let pipebuf_utf8_into_encode_time = bench_pipebuf_write_encode_using_utf8(&local_bytes);
    let pipebuf_decode_time = bench_pipebuf_read_decode(&local_bytes);
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
    console::log_1(&JsValue::from_str(&format!(
        "Pipebuf encode: {:.3}ms/iter",
        pipebuf_encode_time
    )));
    console::log_1(&JsValue::from_str(&format!(
        "Pipebuf encode using JsString::from: {:.3}ms/iter",
        pipebuf_into_encode_time
    )));
    console::log_1(&JsValue::from_str(&format!(
        "Pipebuf encode using UTF8 and JsString::from: {:.3}ms/iter",
        pipebuf_utf8_into_encode_time
    )));
    console::log_1(&JsValue::from_str(&format!(
        "Pipebuf decode: {:.3}ms/iter",
        pipebuf_decode_time
    )));
}

#[inline(never)]
fn bench_jasper_encode(bytes: &[u8]) -> f64 {
    let start = Date::now();
    for _ in 0..100 {
        let char_codes = black_box(base32768::alternative::encode(black_box(bytes)));
        black_box(arr_to_str(&char_codes));
    }
    (Date::now() - start) / 100.0
}

#[inline(never)]
fn bench_jasper_decode(bytes: &[u8]) -> f64 {
    let encoded = base32768::alternative::encode(bytes)
        .chunks(64)
        .map(JsString::from_char_code)
        .reduce(|a, b| a.concat(&b))
        .unwrap();
    let mut local_str: String = encoded.clone().into();
    black_box(&mut local_str);
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

#[inline(never)]
fn bench_optimized_write_encode(bytes: &[u8]) -> f64 {
    let start = Date::now();
    let mut output = Vec::new();
    let slices: Vec<&[u8]> = {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
        let mut start = 0;
        std::iter::from_fn(|| {
            (start < bytes.len()).then(|| {
                let len = rng.gen_range(1..20);
                let next_start = bytes.len().min(start + len);
                let res = &bytes[start..next_start];
                start = next_start;
                res
            })
        })
        .collect()
    };
    for _ in 0..100 {
        let mut writer = black_box(base32768::optimized::WriteEncoder::new_by_ref(&mut output));
        for slice in slices.iter().copied() {
            writer.write_all(slice).unwrap();
        }
        writer.finish();
        black_box(arr_to_str(&output));
        output.clear();
    }
    (Date::now() - start) / 100.0
}

#[inline(never)]
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

struct PipeBufEncoder<'a, F> {
    bytes: &'a mut PipeBuf<u8>,
    utf32768: &'a mut PipeBuf<u16>,
    sink: F,
}

impl<'a, F: for<'b> FnMut(PBufRd<'b, u16>) -> bool> PipeBufEncoder<'a, F> {
    fn new(sink: F, bytes: &'a mut PipeBuf<u8>, utf32768: &'a mut PipeBuf<u16>) -> Self {
        Self {
            sink,
            bytes,
            utf32768,
        }
    }

    fn process(&mut self) {
        let mut activity = true;
        while activity && !(self.bytes.is_done() && self.utf32768.is_done()) {
            activity =
                base32768::pipebuf::encode_bytes_to_utf32768(self.bytes.rd(), self.utf32768.wr())
                    | (self.sink)(self.utf32768.rd());
        }
    }

    fn close(mut self) {
        self.bytes.wr().close();
        self.process();
    }
}

impl<'a, F: for<'b> FnMut(PBufRd<'b, u16>) -> bool> Write for PipeBufEncoder<'a, F> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut written = match self.bytes.wr().free_space() {
            None => buf.len(),
            Some(len) => len.min(buf.len()),
        };
        if written == 0 {
            self.process();
            written = match self.bytes.wr().free_space() {
                None => buf.len(),
                Some(len) => len.min(buf.len()),
            };
        }
        let written = self.bytes.write(&buf[..written])?;
        return Ok(written);
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.bytes.set_push(true);
        self.process();
        Ok(())
    }
}

#[inline(never)]
fn bench_pipebuf_write_encode(bytes: &[u8]) -> f64 {
    // let mut bytes_buf = PipeBuf::new();
    let mut utf32768_buf = PipeBuf::new();
    // const BUF_CAPACITY: usize = 2048;
    let mut bytes_buf: PipeBuf<u8> = PipeBuf::with_fixed_capacity(960);
    //let mut utf32768_buf: PipeBuf<u16> = PipeBuf::with_fixed_capacity(512);
    let slices: Vec<&[u8]> = {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
        let mut start = 0;
        std::iter::from_fn(|| {
            (start < bytes.len()).then(|| {
                let len = rng.gen_range(1..20);
                let next_start = bytes.len().min(start + len);
                let res = &bytes[start..next_start];
                start = next_start;
                res
            })
        })
        .collect()
    };
    let mut output = JsString::from("");
    let start = Date::now();
    for _ in 0..100 {
        let mut writer = black_box(PipeBufEncoder::new(
            |mut rd| {
                if rd.consume_push() || rd.has_pending_eof() {
                    //if rd.consume_push() || rd.has_pending_eof() || rd.len() >= 480 {
                    output = black_box(arr_to_str(rd.data()));
                    rd.consume(rd.len());
                    rd.consume_eof();
                    true
                } else {
                    false
                }
            },
            &mut bytes_buf,
            &mut utf32768_buf,
        ));
        for slice in slices.iter().copied() {
            writer.write_all(slice).unwrap();
        }
        writer.close();
        output = JsString::from("");
        bytes_buf.reset();
        utf32768_buf.reset();
    }
    (Date::now() - start) / 100.0
}

#[inline(never)]
fn bench_pipebuf_write_encode_using_into(bytes: &[u8]) -> f64 {
    // let mut bytes_buf = PipeBuf::new();
    // let mut utf32768_buf = PipeBuf::new();
    const BUF_CAPACITY: usize = 2048;
    let mut bytes_buf: PipeBuf<u8> = PipeBuf::with_fixed_capacity(BUF_CAPACITY);
    let mut utf32768_buf: PipeBuf<u16> = PipeBuf::with_fixed_capacity(BUF_CAPACITY);
    let mut output = JsString::from("");
    let mut buffer_str = String::with_capacity(BUF_CAPACITY * 3);
    let start = Date::now();
    for _ in 0..100 {
        let mut writer = black_box(PipeBufEncoder::new(
            |mut rd| {
                // if rd.consume_push() || rd.has_pending_eof() {
                if rd.consume_push() || rd.has_pending_eof() || rd.len() >= BUF_CAPACITY / 2 {
                    buffer_str.clear();
                    buffer_str.extend(
                        std::char::decode_utf16(rd.data().iter().copied()).map(Result::unwrap),
                    );
                    output = output.concat(JsString::from(buffer_str.as_str()).as_ref());
                    rd.consume(rd.len());
                    rd.consume_eof();
                    true
                } else {
                    false
                }
            },
            &mut bytes_buf,
            &mut utf32768_buf,
        ));
        writer.write_all(black_box(bytes)).unwrap();
        writer.close();
        output = JsString::from("");
        bytes_buf.reset();
        utf32768_buf.reset();
    }
    (Date::now() - start) / 100.0
}

struct PipeBufUtf8Encoder<'a, F> {
    bytes: &'a mut PipeBuf<u8>,
    utf32768: &'a mut PipeBuf<u8>,
    sink: F,
}

impl<'a, F: for<'b> FnMut(PBufRd<'b, u8>) -> bool> PipeBufUtf8Encoder<'a, F> {
    fn new(sink: F, bytes: &'a mut PipeBuf<u8>, utf32768: &'a mut PipeBuf<u8>) -> Self {
        Self {
            sink,
            bytes,
            utf32768,
        }
    }

    fn process(&mut self) {
        let mut activity = true;
        while activity && !(self.bytes.is_done() && self.utf32768.is_done()) {
            activity = base32768::pipebuf::encode_bytes_to_base32768_utf8(
                self.bytes.rd(),
                self.utf32768.wr(),
            ) | (self.sink)(self.utf32768.rd());
        }
    }

    fn close(mut self) {
        self.bytes.wr().close();
        self.process();
    }
}

impl<'a, F: for<'b> FnMut(PBufRd<'b, u8>) -> bool> Write for PipeBufUtf8Encoder<'a, F> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        loop {
            let written = match self.bytes.wr().free_space() {
                None => buf.len(),
                Some(len) => len.min(buf.len()),
            };
            if written == 0 {
                self.process();
                continue;
            }
            let written = self.bytes.write(&buf[..written])?;
            return Ok(written);
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.bytes.set_push(true);
        self.process();
        Ok(())
    }
}

#[inline(never)]
fn bench_pipebuf_write_encode_using_utf8(bytes: &[u8]) -> f64 {
    // let mut bytes_buf = PipeBuf::new();
    // let mut utf32768_buf = PipeBuf::new();
    const BUF_CAPACITY: usize = 2048;
    let mut bytes_buf = PipeBuf::with_fixed_capacity(BUF_CAPACITY);
    let mut utf32768_buf = PipeBuf::with_fixed_capacity(BUF_CAPACITY);
    let mut output = JsString::from("");
    let start = Date::now();
    for _ in 0..100 {
        let mut writer = black_box(PipeBufUtf8Encoder::new(
            |mut rd| {
                // if rd.consume_push() || rd.has_pending_eof() {
                if rd.consume_push() || rd.has_pending_eof() || rd.len() >= (3 * BUF_CAPACITY) / 4 {
                    let s = unsafe { std::str::from_utf8_unchecked(rd.data()) };
                    output = output.concat(JsString::from(s).as_ref());
                    rd.consume(rd.len());
                    rd.consume_eof();
                    true
                } else {
                    false
                }
            },
            &mut bytes_buf,
            &mut utf32768_buf,
        ));
        writer.write_all(black_box(bytes)).unwrap();
        writer.close();
        output = JsString::from("");
        bytes_buf.reset();
        utf32768_buf.reset();
    }
    (Date::now() - start) / 100.0
}

struct PipeBufDecoder<'a, F> {
    source: F,
    utf32768: &'a mut PipeBuf<u16>,
    u15s: &'a mut PipeBuf<u16>,
    bytes: &'a mut PipeBuf<u8>,
}

impl<'a, F: for<'b> FnMut(PBufWr<'b, u16>) -> bool> PipeBufDecoder<'a, F> {
    fn new(
        source: F,
        utf32768: &'a mut PipeBuf<u16>,
        u15s: &'a mut PipeBuf<u16>,
        bytes: &'a mut PipeBuf<u8>,
    ) -> Self {
        Self {
            source,
            utf32768,
            u15s,
            bytes,
        }
    }

    fn process(&mut self) -> Result<(), DecoderError> {
        if (self.source)(self.utf32768.wr()) {
            let mut activity = true;
            while activity {
                activity =
                    base32768::pipebuf::decode_utf32768_to_u15(self.utf32768.rd(), self.u15s.wr())?
                        | base32768::pipebuf::decode_u15_to_bytes(self.u15s.rd(), self.bytes.wr())?;
            }
        }
        Ok(())
    }
}

impl<'a, F: for<'b> FnMut(PBufWr<'b, u16>) -> bool> Read for PipeBufDecoder<'a, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            match self.bytes.read(buf) {
                Err(e) if e.kind() == ErrorKind::WouldBlock => self.process()?,
                r => return r,
            }
        }
    }
}

#[inline(never)]
fn bench_pipebuf_read_decode(bytes: &[u8]) -> f64 {
    let mut utf32768_buf = PipeBuf::new();
    let mut u15s_buf = PipeBuf::new();
    let mut bytes_buf = PipeBuf::new();
    let encoded = {
        let mut encoded = JsString::from("");
        let mut writer = PipeBufEncoder::new(
            |mut rd| {
                if rd.consume_push() || rd.has_pending_eof() {
                    encoded = encoded.concat(arr_to_str(rd.data()).as_ref());
                    rd.consume(rd.len());
                    rd.consume_eof();
                    true
                } else {
                    false
                }
            },
            &mut bytes_buf,
            &mut utf32768_buf,
        );
        writer.write_all(bytes).unwrap();
        writer.close();
        bytes_buf.reset();
        utf32768_buf.reset();
        encoded
    };
    let mut output = Vec::new();

    let start = Date::now();
    for _ in 0..100 {
        let mut start = 0;
        let mut reader = PipeBufDecoder::new(
            |mut wr| {
                if wr.is_eof() {
                    false
                } else if start >= encoded.length() {
                    wr.close();
                    true
                } else {
                    let length = encoded.length();
                    let chunk_size = length;
                    let written = str_to_arr(
                        &encoded.substring(start, start + chunk_size),
                        wr.space(
                            wr.free_space()
                                .unwrap_or(usize::MAX)
                                .min(chunk_size as usize),
                        ),
                    );
                    wr.commit(written as usize);
                    start += written;
                    if written == length {
                        wr.close();
                    }
                    written > 0
                }
            },
            &mut utf32768_buf,
            &mut u15s_buf,
            &mut bytes_buf,
        );
        reader.read_to_end(&mut output).unwrap();
        utf32768_buf.reset();
        u15s_buf.reset();
        bytes_buf.reset();
        output.clear();
    }
    (Date::now() - start) / 100.0
}
