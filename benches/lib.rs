#![feature(test)]
extern crate base32768;
extern crate rand;
extern crate test;

use std::io::Read;
use std::io::Write;

use test::black_box;
use test::Bencher;

use rand::{Rng, SeedableRng};

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

// #[bench]
// fn bench_jasper_encode(b: &mut Bencher) {
//     let mut byte_vec = vec![0u8; 3_000_000];
//     let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
//     rng.fill(&mut byte_vec[..]);
//     b.iter(|| {
//         black_box(base32768::alternative::encode(
//             black_box(&mut byte_vec).as_slice(),
//         ))
//     })
// }

// #[bench]
// fn bench_jasper_decode(b: &mut Bencher) {
//     let mut byte_vec = vec![0u8; 3_000_000];
//     let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
//     rng.fill(&mut byte_vec[..]);
//     let mut encoded = base32768::alternative::encode(byte_vec.as_slice());
//     byte_vec.clear();
//     b.iter(|| black_box(base32768::alternative::decode(black_box(&mut *encoded))))
// }

#[bench]
fn bench_optimized_write_encode(b: &mut Bencher) {
    let mut byte_vec = vec![0u8; 1000_000];
    let mut rng = rand::rngs::SmallRng::seed_from_u64(42023241994);
    rng.fill(&mut byte_vec[..]);
    let mut output: Vec<u16> = Vec::new();
    b.iter(|| {
        let mut writer = black_box(base32768::optimized::WriteEncoder::new_by_ref(black_box(
            &mut output,
        )));
        writer
            .write_all(black_box(&mut byte_vec).as_slice())
            .unwrap();
        writer.finish();
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
