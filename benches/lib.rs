#![feature(test)]
extern crate base32768;
extern crate pipebuf;
extern crate rand;
extern crate test;

use base32768::optimized::DecoderError;
use pipebuf::{PBufRd, PBufWr, PipeBuf};
use rand::{Rng, SeedableRng};
use std::io::{ErrorKind, Read, Write};
use test::{black_box, Bencher};

// #[bench]
// fn bench_base32768_encode(b: &mut Bencher) {
//     let mut byte_vec = vec![0u8; 3_000_000];
//     let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
//     rng.fill(&mut byte_vec[..]);
//     b.iter(|| black_box(base32768::encode(black_box(&mut byte_vec).as_slice())))
// }

// #[bench]
// fn bench_base32768_decode(b: &mut Bencher) {
//     let mut byte_vec = vec![0u8; 3_000_000];
//     let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
//     rng.fill(&mut byte_vec[..]);
//     let mut encoded = base32768::encode(byte_vec.as_slice()).unwrap();
//     b.iter(|| {
//         byte_vec.clear();
//         black_box(base32768::decode(
//             black_box(encoded.as_mut_str()),
//             black_box(&mut byte_vec),
//         ))
//     })
// }

#[bench]
fn bench_jasper_encode(b: &mut Bencher) {
    let mut byte_vec = vec![0u8; 3_000_000];
    let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
    rng.fill(&mut byte_vec[..]);
    b.iter(|| {
        black_box(base32768::alternative::encode(
            black_box(&mut byte_vec).as_slice(),
        ))
    })
}

#[bench]
fn bench_jasper_decode(b: &mut Bencher) {
    let mut byte_vec = vec![0u8; 3_000_000];
    let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
    rng.fill(&mut byte_vec[..]);
    let mut encoded = base32768::alternative::encode(byte_vec.as_slice());
    byte_vec.clear();
    b.iter(|| black_box(base32768::alternative::decode(black_box(&mut *encoded))))
}

#[bench]
fn bench_optimized_write_encode(b: &mut Bencher) {
    let mut byte_vec = vec![0u8; 3_000_000];
    let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
    rng.fill(&mut byte_vec[..]);
    let mut start = 0;
    let slices: Vec<&[u8]> = std::iter::from_fn(|| {
        (start < byte_vec.len()).then(|| {
            let len = rng.gen_range(1..20);
            let next_start = byte_vec.len().min(start + len);
            let res = &byte_vec[start..next_start];
            start = next_start;
            res
        })
    })
    .collect();
    let mut output: Vec<u16> = Vec::new();
    b.iter(|| {
        let mut writer = black_box(base32768::optimized::WriteEncoder::new_by_ref(black_box(
            &mut output,
        )));
        for slice in slices.iter().copied() {
            writer.write_all(slice).unwrap();
        }
        writer.finish();
        black_box(String::from_utf16(output.as_slice()).unwrap());
        output.clear();
    })
}

#[bench]
fn bench_optimized_read_decode(b: &mut Bencher) {
    let mut byte_vec = vec![0u8; 3_000_000];
    let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
    rng.fill(&mut byte_vec[..]);
    let mut writer = base32768::optimized::WriteEncoder::new(Vec::new());
    writer.write_all(&byte_vec).unwrap();
    let mut encoded = writer.finish();
    b.iter(|| {
        black_box(&mut byte_vec).clear();
        base32768::optimized::ReadDecoder::<_, 960>::new(black_box(&mut encoded).iter().copied())
            .read_to_end(black_box(&mut byte_vec))
            .unwrap();
    })
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
        (self.source)(self.utf32768.wr());
        let mut activity = true;
        while activity {
            activity =
                base32768::pipebuf::decode_utf32768_to_u15(self.utf32768.rd(), self.u15s.wr())?
                    | base32768::pipebuf::decode_u15_to_bytes(self.u15s.rd(), self.bytes.wr())?;
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

    fn free_write_space(&mut self, buf_len: usize) -> usize {
        match self.bytes.wr().free_space() {
            None => buf_len,
            Some(len) => len.min(buf_len),
        }
    }

    fn close(mut self) {
        self.bytes.wr().close();
        self.process();
    }
}

impl<'a, F: for<'b> FnMut(PBufRd<'b, u16>) -> bool> Write for PipeBufEncoder<'a, F> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut written = self.free_write_space(buf.len());
        if written == 0 {
            self.process();
            written = self.free_write_space(buf.len());
            if written == 0 {
                return Ok(0);
            }
        }
        return self.bytes.write(&buf[..written]);
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.bytes.set_push(true);
        self.process();
        Ok(())
    }
}

#[bench]
fn bench_pipebuf_read_decode_no_cap(b: &mut Bencher) {
    let mut byte_vec = vec![0u8; 3_000_000];
    let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
    rng.fill(&mut byte_vec[..]);
    let encoded = {
        let mut encoded = Vec::new();
        let mut buf1 = PipeBuf::new();
        let mut buf2 = PipeBuf::new();
        let mut writer = PipeBufEncoder::new(
            |mut rd| {
                let before = rd.tripwire();
                encoded.extend_from_slice(rd.data());
                rd.consume_push();
                rd.consume_eof();
                rd.consume(rd.len());
                rd.is_tripped(before)
            },
            &mut buf1,
            &mut buf2,
        );
        writer.write_all(byte_vec.as_slice()).unwrap();
        writer.close();
        encoded
    };

    let mut buf1 = PipeBuf::new();
    let mut buf2 = PipeBuf::new();
    let mut buf3 = PipeBuf::new();
    b.iter(|| {
        black_box(&mut byte_vec).clear();
        PipeBufDecoder::new(
            |mut wr| {
                if wr.is_eof() {
                    false
                } else {
                    wr.space(encoded.len()).copy_from_slice(encoded.as_slice());
                    wr.commit(encoded.len());
                    wr.close();
                    true
                }
            },
            &mut buf1,
            &mut buf2,
            &mut buf3,
        )
        .read_to_end(black_box(&mut byte_vec))
        .unwrap();
        buf1.reset();
        buf2.reset();
        buf3.reset();
    })
}

#[bench]
fn bench_pipebuf_read_decode_with_cap(b: &mut Bencher) {
    let mut byte_vec = vec![0u8; 3_000_000];
    let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
    rng.fill(&mut byte_vec[..]);
    let encoded = {
        let mut encoded = Vec::new();
        let mut buf1 = PipeBuf::new();
        let mut buf2 = PipeBuf::new();
        let mut writer = PipeBufEncoder::new(
            |mut rd| {
                let before = rd.tripwire();
                encoded.extend_from_slice(rd.data());
                rd.consume_push();
                rd.consume_eof();
                rd.consume(rd.len());
                rd.is_tripped(before)
            },
            &mut buf1,
            &mut buf2,
        );
        writer.write_all(byte_vec.as_slice()).unwrap();
        writer.close();
        encoded
    };

    let mut buf1 = PipeBuf::with_fixed_capacity(512);
    let mut buf2 = PipeBuf::with_fixed_capacity(512);
    let mut buf3 = PipeBuf::with_fixed_capacity(960);
    b.iter(|| {
        black_box(&mut byte_vec).clear();
        let mut encoded_slice = encoded.as_slice();
        PipeBufDecoder::new(
            |mut wr| {
                if wr.is_eof() {
                    false
                } else {
                    let available = wr.free_space().unwrap().min(encoded_slice.len());
                    wr.space(available)
                        .copy_from_slice(&encoded_slice[..available]);
                    wr.commit(available);
                    encoded_slice = &encoded_slice[available..];
                    if encoded_slice.len() == 0 {
                        wr.close();
                    }
                    available > 0
                }
            },
            &mut buf1,
            &mut buf2,
            &mut buf3,
        )
        .read_to_end(black_box(&mut byte_vec))
        .unwrap();
        buf1.reset();
        buf2.reset();
        buf3.reset();
    })
}

#[bench]
fn bench_pipebuf_write_encode_no_cap(b: &mut Bencher) {
    let mut byte_vec = vec![0u8; 3_000_000];
    let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
    rng.fill(&mut byte_vec[..]);
    let mut start = 0;
    let slices: Vec<&[u8]> = std::iter::from_fn(|| {
        (start < byte_vec.len()).then(|| {
            let len = rng.gen_range(1..20);
            let next_start = byte_vec.len().min(start + len);
            let res = &byte_vec[start..next_start];
            start = next_start;
            res
        })
    })
    .collect();
    let mut buf1 = PipeBuf::new();
    let mut buf2 = PipeBuf::new();
    let mut s = String::with_capacity(3_000_000);
    b.iter(|| {
        let mut writer = PipeBufEncoder::new(
            |mut rd| {
                if rd.consume_push() || rd.consume_eof() {
                    s += String::from_utf16(rd.data()).unwrap().as_str();
                    rd.consume(rd.len());
                    true
                } else {
                    false
                }
            },
            &mut buf1,
            &mut buf2,
        );
        for slice in slices.iter().copied() {
            writer.write_all(slice).unwrap();
        }
        writer.close();
        buf1.reset();
        buf2.reset();
        s.clear();
    })
}

#[bench]
fn bench_pipebuf_write_encode_with_cap(b: &mut Bencher) {
    let mut byte_vec = vec![0u8; 3_000_000];
    let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
    rng.fill(&mut byte_vec[..]);
    let mut start = 0;
    let slices: Vec<&[u8]> = std::iter::from_fn(|| {
        (start < byte_vec.len()).then(|| {
            let len = rng.gen_range(1..20);
            let next_start = byte_vec.len().min(start + len);
            let res = &byte_vec[start..next_start];
            start = next_start;
            res
        })
    })
    .collect();
    let mut buf1 = PipeBuf::with_fixed_capacity(960);
    let mut buf2 = PipeBuf::with_fixed_capacity(512);
    let mut s = String::with_capacity(3_000_000);
    b.iter(|| {
        let mut writer = PipeBufEncoder::new(
            |mut rd| {
                if rd.consume_push() || rd.consume_eof() || rd.len() >= 480 {
                    s += String::from_utf16(rd.data()).unwrap().as_str();
                    rd.consume(rd.len());
                    true
                } else {
                    false
                }
            },
            &mut buf1,
            &mut buf2,
        );
        let mut byte_vec_slice = byte_vec.as_slice();
        for slice in slices.iter().copied() {
            writer.write_all(slice).unwrap();
        }
        writer.close();
        buf1.reset();
        buf2.reset();
        s.clear();
    })
}

#[bench]
fn bench_pipebuf_write_encode_half_cap(b: &mut Bencher) {
    let mut byte_vec = vec![0u8; 3_000_000];
    let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
    rng.fill(&mut byte_vec[..]);
    let mut start = 0;
    let slices: Vec<&[u8]> = std::iter::from_fn(|| {
        (start < byte_vec.len()).then(|| {
            let len = rng.gen_range(1..20);
            let next_start = byte_vec.len().min(start + len);
            let res = &byte_vec[start..next_start];
            start = next_start;
            res
        })
    })
    .collect();
    let mut buf1 = PipeBuf::with_fixed_capacity(1024);
    let mut buf2 = PipeBuf::new();
    let mut s = String::with_capacity(3_000_000);
    b.iter(|| {
        let mut writer = PipeBufEncoder::new(
            |mut rd| {
                if rd.consume_push() || rd.consume_eof() {
                    s += String::from_utf16(rd.data()).unwrap().as_str();
                    rd.consume(rd.len());
                    true
                } else {
                    false
                }
            },
            &mut buf1,
            &mut buf2,
        );
        let mut byte_vec_slice = byte_vec.as_slice();
        for slice in slices.iter().copied() {
            writer.write_all(slice).unwrap();
        }
        writer.close();
        buf1.reset();
        buf2.reset();
        s.clear();
    })
}
