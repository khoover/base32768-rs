use std::convert::TryInto;
use std::hint::unreachable_unchecked;
use std::io::Result as IoResult;
use std::io::Write;
use std::mem::ManuallyDrop;
use std::str::FromStr;

extern crate arrayvec;
use self::arrayvec::ArrayVec;

extern crate js_sys;
use self::js_sys::{Array, JsString};

use super::{get_lookups, BYTE_SIZE, CODE_LEN, SMALL_LEN};

pub struct WriteEncoder<T: ?Sized + Extend<u16>> {
    buf: ArrayVec<u8, CODE_LEN>,
    sink: T,
}

pub struct ByRef<'a, T>(&'a mut T);

impl<'a, E: Extend<u16>> Extend<u16> for ByRef<'a, E> {
    fn extend<T: IntoIterator<Item = u16>>(&mut self, iter: T) {
        self.0.extend(iter)
    }
}

pub struct BufferedJsString<const N: usize> {
    buf: Box<ArrayVec<u16, N>>,
    js_str: JsString,
}

impl<const N: usize> Default for BufferedJsString<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> BufferedJsString<N> {
    pub fn new() -> Self {
        Self {
            buf: Box::new(ArrayVec::new()),
            js_str: JsString::from_str("").unwrap(),
        }
    }

    pub fn new_from(s: impl Into<JsString>) -> Self {
        Self {
            buf: Box::new(ArrayVec::new()),
            js_str: s.into(),
        }
    }

    pub fn flush(&mut self) {
        if let Some(other) = self
            .buf
            .take()
            .as_slice()
            .chunks(64)
            .map(JsString::from_char_code)
            .reduce(|a, b| a.concat(&b))
        {
            self.js_str = self.js_str.concat(&other);
        }
    }

    pub fn finish(mut self) -> JsString {
        self.flush();
        self.js_str
    }
}

impl<const N: usize> Extend<u16> for BufferedJsString<N> {
    fn extend<T: IntoIterator<Item = u16>>(&mut self, iter: T) {
        let mut iter = iter.into_iter();
        loop {
            self.buf
                .extend(iter.by_ref().take(self.buf.remaining_capacity()));
            if !self.buf.is_full() {
                break;
            }
            self.flush();
        }
    }
}

impl<T: Extend<u16>> WriteEncoder<T> {
    pub fn new(sink: T) -> Self {
        Self {
            buf: ArrayVec::new(),
            sink,
        }
    }

    pub fn new_by_ref(sink: &mut T) -> WriteEncoder<ByRef<'_, T>> {
        WriteEncoder {
            buf: ArrayVec::new(),
            sink: ByRef(sink),
        }
    }

    pub fn finish(mut self) -> T {
        self.flush_buf();
        let mut this = ManuallyDrop::new(self);
        // SAFETY: We're reading the sink out into a new memory location, dropping the buffer,
        // and forgetting the memory used by `self`.
        unsafe {
            let res = std::ptr::read(&this.sink);
            std::ptr::drop_in_place(&mut this.buf);
            res
        }
    }
}

impl<T: ?Sized + Extend<u16>> WriteEncoder<T> {
    fn flush_full_buf(&mut self) {
        debug_assert_eq!(self.buf.remaining_capacity(), 0);
        self.sink
            .extend(block_encode_iter(&self.buf.take().into_inner().unwrap()));
    }

    fn flush_buf(&mut self) {
        if self.buf.is_empty() {
        } else if self.buf.remaining_capacity() == 0 {
            self.flush_full_buf();
        } else {
            let mut output = [0_u16; 8];
            let mut idx = 0;
            let tables = get_lookups();
            let mut acc: u16 = 0;
            let mut used_bits = 0_u8;

            for byte in self.buf.take() {
                acc |= (byte as u16) << used_bits;
                used_bits += BYTE_SIZE as u8;
                if used_bits >= CODE_LEN as u8 {
                    output[idx] = tables.long_encode[(acc & 0x7FFF) as usize];
                    idx += 1;
                    used_bits -= CODE_LEN as u8;
                    acc = (byte.rotate_left(used_bits as u32) & !(0xFF << used_bits)) as u16;
                }
            }

            acc |= 0xFFFF << used_bits;
            match used_bits as usize {
                1..=SMALL_LEN => {
                    output[idx] = tables.short_encode[(acc & 0x7F) as usize];
                    idx += 1;
                }
                CODE_LEN => unreachable!(),
                0 => (),
                _ => {
                    output[idx] = tables.long_encode[(acc & 0x7FFF) as usize];
                    idx += 1;
                }
            }
            self.sink.extend(output[..idx].iter().copied());
        }
    }
}

impl<T: ?Sized + Extend<u16>> Drop for WriteEncoder<T> {
    fn drop(&mut self) {
        self.flush_buf();
    }
}

impl<T: ?Sized + Extend<u16>> Write for WriteEncoder<T> {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        if self.buf.is_full() {
            self.flush_full_buf();
            let to_copy = usize::min(CODE_LEN, buf.len());
            self.buf.try_extend_from_slice(&buf[..to_copy]).unwrap();
            Ok(to_copy)
        } else {
            let to_copy = usize::min(self.buf.remaining_capacity(), buf.len());
            self.buf.try_extend_from_slice(&buf[..to_copy]).unwrap();
            Ok(to_copy)
        }
    }

    fn flush(&mut self) -> IoResult<()> {
        if self.buf.is_full() {
            self.flush_full_buf();
        }
        Ok(())
    }

    fn write_all(&mut self, mut buf: &[u8]) -> IoResult<()> {
        if !self.buf.is_empty() || buf.len() < CODE_LEN {
            let copied = usize::min(self.buf.remaining_capacity(), buf.len());
            self.buf.try_extend_from_slice(&buf[..copied]).unwrap();
            if self.buf.is_full() {
                self.flush_full_buf();
            } else {
                return Ok(());
            }
            buf = buf.split_at(copied).1;
        }
        let mut chunks = buf.chunks_exact(CODE_LEN);
        chunks
            .by_ref()
            .map(|chunk| block_encode_iter(chunk.try_into().unwrap()))
            .for_each(|iter| self.sink.extend(iter));
        self.buf.try_extend_from_slice(chunks.remainder()).unwrap();
        Ok(())
    }
}

fn block_encode_iter(src: &[u8; 15]) -> impl Iterator<Item = u16> {
    let tables = get_lookups();
    let block = u128::from_le_bytes([
        src[0], src[1], src[2], src[3], src[4], src[5], src[6], src[7], src[8], src[9], src[10],
        src[11], src[12], src[13], src[14], 0,
    ]);
    (0..8)
        .map(|idx| idx * CODE_LEN)
        .map(move |shift| tables.long_encode[(block >> shift) as usize & 0x7FFF])
}
