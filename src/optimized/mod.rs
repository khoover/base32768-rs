use std::mem::MaybeUninit;
use std::ops::Range;
use std::ptr::addr_of_mut;
use std::sync::OnceLock;

pub(crate) const CODE_LEN: usize = 15;
pub(crate) const SMALL_LEN: usize = 7;
pub(crate) const BYTE_SIZE: usize = 8;

pub struct Tables {
    pub long_encode: [u16; 1 << CODE_LEN],
    pub short_encode: [u16; 1 << SMALL_LEN],
    pub decode: [u16; 42183],
}

fn make_tables() -> Box<Tables> {
    let mut tables: Box<MaybeUninit<Tables>> = Box::new(MaybeUninit::uninit());
    let tables_ptr = tables.as_mut_ptr();
    unsafe {
        addr_of_mut!((*tables_ptr).decode).write_bytes(0xFF, 1);
    }
    let decode_ptr = unsafe { addr_of_mut!((*tables_ptr).decode) as *mut u16 };

    let long_encode_ptr = unsafe { addr_of_mut!((*tables_ptr).long_encode) as *mut u16 };
    let mut ranges: [Range<u16>; 49] = [
        19904..40892,
        13312..19894,
        40960..42125,
        5121..5741,
        9451..9885,
        10224..10627,
        9003..9140,
        11392..11499,
        10765..10868,
        10871..10972,
        592..688,
        4352..4442,
        6176..6264,
        5024..5109,
        11936..12019,
        5792..5867,
        4608..4681,
        1657..1728,
        4888..4955,
        10649..10712,
        8942..9001,
        4824..4881,
        1162..1217,
        4547..4602,
        6624..6679,
        10973..11028,
        42128..42183,
        11568..11622,
        6016..6068,
        8656..8708,
        3585..3633,
        8880..8928,
        11264..11311,
        11312..11359,
        4470..4515,
        7424..7468,
        4304..4347,
        6528..6570,
        4704..4745,
        6272..6313,
        6470..6510,
        12549..12589,
        9216..9255,
        1329..1367,
        1377..1415,
        1920..1958,
        4256..4294,
        11520..11558,
        2308..2345,
    ];
    ranges
        .iter_mut()
        .flatten()
        .take(1 << CODE_LEN)
        .enumerate()
        .for_each(|(idx, code)| unsafe {
            debug_assert!(code < 42183);
            long_encode_ptr.add(idx).write(code);
            decode_ptr.offset(code as isize).write(idx as u16);
        });

    let short_encode_ptr = unsafe { addr_of_mut!((*tables_ptr).short_encode) as *mut u16 };
    let mut ranges: [Range<u16>; 4] = [9143..9180, 10025..10060, 4096..4130, 7545..7579];
    ranges
        .iter_mut()
        .flatten()
        .take(1 << SMALL_LEN)
        .enumerate()
        .for_each(|(idx, code)| unsafe {
            debug_assert!(code < 42183);
            short_encode_ptr.add(idx).write(code);
            decode_ptr.offset(code as isize).write(idx as u16 | 0x8000);
        });

    // The same thing as what `Box<MaybeUnint<T>>::assume_init`` does, without the nightly
    unsafe { Box::from_raw(Box::into_raw(tables) as *mut Tables) }
}

static LOOKUPS: OnceLock<Box<Tables>> = OnceLock::new();

pub(crate) fn get_lookups() -> &'static Tables {
    LOOKUPS.get_or_init(make_tables)
}

mod decoder_impl;
mod encoder_impl;

pub use self::decoder_impl::{DecoderError, ReadDecoder};
pub use self::encoder_impl::{BufferedJsString, ByRef, WriteEncoder};
