extern crate base32768;
extern crate glob;
extern crate rand;

use std::fs;
use std::io::{Read, Write};
use std::path::Path;

#[test]
fn test_encode_hello() {
    let res = base32768::encode(&[72u8, 101u8, 108u8, 108u8, 111u8]);
    assert_eq!(res.unwrap(), "䩲腻㐿");
}

#[test]
fn test_decode_hello() {
    let hello = [72u8, 101u8, 108u8, 108u8, 111u8];
    let mut decoded = Vec::<u8>::new();
    base32768::decode("䩲腻㐿", &mut decoded).unwrap();

    assert_eq!(decoded.as_slice(), hello);
}

#[test]
fn run_encode_decode_test_suite() {
    let src_dir = Path::new(file!()).parent().unwrap().to_str().unwrap();
    for entry in
        glob::glob(&format!("{}/test/**/*.bin", src_dir)).expect("Failed to glob test directory")
    {
        if let Ok(path) = entry {
            let bin_vec = fs::read(&path).unwrap();
            if path.ends_with("every-pair-of-bytes.bin") {
                println!("{}", bin_vec.len());
            }
            let test_string = fs::read_to_string(path.with_extension("txt")).unwrap();

            let res = base32768::encode(&bin_vec);
            if let Err(e) = res {
                panic!(
                    "Got error {} trying to encode from file {}",
                    e,
                    path.display()
                );
            }
            let out = res.unwrap();
            assert_eq!(out, test_string);

            let mut decoded = Vec::<u8>::new();
            let res = base32768::decode(&out, &mut decoded);
            if let Err(e) = res {
                panic!(
                    "Got error {} trying to decode from file {}",
                    e,
                    path.display()
                );
            }
            assert_eq!(decoded.as_slice(), bin_vec.as_slice());

            let out = {
                let mut writer = base32768::optimized::WriteEncoder::new(Vec::new());
                writer.write_all(&bin_vec).unwrap();
                writer.finish()
            };
            assert!(out
                .iter()
                .copied()
                .all(|codepoint| base32768::optimized::get_lookups()
                    .decode
                    .get(codepoint as usize)
                    .filter(|x| **x != 0xFFFF)
                    .is_some()));
            let mut read_back = Vec::new();
            base32768::optimized::ReadDecoder::<_, 60>::new(out.iter().copied())
                .read_to_end(&mut read_back)
                .unwrap();
            assert_eq!(read_back.as_slice(), bin_vec.as_slice());
        }
    }
}
