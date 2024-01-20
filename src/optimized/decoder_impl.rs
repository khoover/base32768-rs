use std::{
    convert::TryFrom,
    convert::TryInto,
    error::Error,
    fmt::Display,
    io::{BufRead, Read, Write},
    mem::MaybeUninit,
};

use super::{get_lookups, BYTE_SIZE, CODE_LEN, SMALL_LEN};

extern crate arrayvec;

use self::arrayvec::ArrayVec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecoderError {
    InvalidCodePoint(u16),
    UnexpectedEndOfStreamMarker,
    InvalidPadding(u8),
}

impl Display for DecoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCodePoint(x) => f.write_fmt(format_args!(
                "Non-base32768 code point encountered: {:#x} ({})",
                *x,
                String::from_utf16_lossy(&[*x])
            )),
            Self::UnexpectedEndOfStreamMarker => f.write_str("Encountered end-of-stream code point mid-stream"),
            Self::InvalidPadding(x) => {
                f.write_fmt(format_args!("Invalid padding used for end-of-stream: expected least-significant bits of {:b} to be all-1", x))
            }
        }
    }
}

impl Error for DecoderError {}

impl From<DecoderError> for std::io::Error {
    fn from(value: DecoderError) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, value)
    }
}

#[derive(Debug, Clone)]
pub struct Buffer<const N: usize> {
    buf: [MaybeUninit<u8>; N],
    filled: usize,
    consumed: usize,
}

impl<const N: usize> Buffer<N> {
    pub const fn new() -> Self {
        Self {
            buf: [MaybeUninit::uninit(); N],
            filled: 0,
            consumed: 0,
        }
    }

    pub fn is_all_consumed(&self) -> bool {
        self.consumed >= self.filled
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.buf[self.consumed..self.filled].as_ptr() as *const u8,
                self.filled - self.consumed,
            )
        }
    }

    pub fn clear_and_get_mut(&mut self) -> &mut [MaybeUninit<u8>; N] {
        self.filled = 0;
        self.consumed = 0;
        &mut self.buf
    }

    pub unsafe fn set_filled(&mut self, amt: usize) {
        self.filled = usize::min(N, amt);
    }

    pub fn consume(&mut self, amt: usize) {
        self.consumed = usize::min(self.consumed + amt, self.filled);
    }

    pub fn consume_with(&mut self, amt: usize, f: impl FnOnce(&[u8])) -> bool {
        if let Some(claimed) = self.as_slice().get(..amt) {
            f(claimed);
            self.consumed += amt;
            true
        } else {
            false
        }
    }
}

const fn byte_count_to_u15_count(count: usize) -> usize {
    count / 15 * 8
}

#[derive(Debug)]
pub struct ReadDecoder<I: ?Sized, const N: usize> {
    buf: Box<Buffer<N>>,
    closed: bool,
    iter: I,
}

impl<I: Iterator<Item = u16>, const N: usize> ReadDecoder<I, N> {
    pub fn new<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = u16, IntoIter = I>,
    {
        assert!(N % 15 == 0 && N > 0);
        Self {
            iter: iter.into_iter(),
            closed: false,
            buf: Box::new(Buffer::new()),
        }
    }
}

impl<I: ?Sized + Iterator<Item = u16>, const N: usize> ReadDecoder<I, N> {
    // We don't want this to get inlined into the read methods
    // #[inline(never)]
    // #[cold]
    fn refill_buf(&mut self) -> std::io::Result<()> {
        let out_buf = self.buf.clear_and_get_mut();
        if self.closed {
            return Ok(());
        }
        let u15_count = byte_count_to_u15_count(N);
        let decoder = &get_lookups().decode;
        let decoded: ArrayVec<u16, N> = {
            let iter = &mut self.iter;
            let closed = &mut self.closed;
            iter.take(u15_count)
                .map(|encoded| {
                    decoder
                        .get(encoded as usize)
                        .copied()
                        .filter(|&x| x != 0xFFFF)
                        .ok_or(DecoderError::InvalidCodePoint(encoded))
                })
                .collect::<Result<_, _>>()
                .map_err(|e: DecoderError| {
                    *closed = true;
                    e
                })?
        };

        if decoded.is_empty() {
            self.closed = true;
            return Ok(());
        }

        if decoded[..decoded.len() - 1]
            .iter()
            .any(|&x| x & 0x8000 != 0)
        {
            self.closed = true;
            return Err(DecoderError::UnexpectedEndOfStreamMarker.into());
        }

        let mut chunks = out_buf
            .chunks_exact_mut(15)
            .map(|chunk| <&mut [MaybeUninit<u8>; 15]>::try_from(chunk).unwrap());
        let mut decoded_chunks = decoded.chunks_exact(8);
        let filled_amount = if decoded.len() == u15_count && (decoded.last().unwrap() & 0x8000 == 0)
        {
            chunks
                .zip(decoded_chunks.map(|chunk| chunk.try_into().unwrap()))
                .for_each(|(out_chunk, in_chunk)| decode_full_chunk(out_chunk, in_chunk));
            N
        } else {
            self.closed = true;
            let (full_chunks, remainder) = if decoded.len() == u15_count {
                (
                    decoded_chunks.len() - 1,
                    decoded_chunks.next_back().unwrap(),
                )
            } else {
                (decoded_chunks.len(), decoded_chunks.remainder())
            };
            chunks
                .by_ref()
                .take(full_chunks)
                .zip(decoded_chunks.by_ref())
                .for_each(|(out_chunk, in_chunk)| {
                    decode_full_chunk(out_chunk, in_chunk.try_into().unwrap())
                });
            let partial_amt = decode_partial_final_chunk(chunks.next().unwrap(), remainder)?;
            partial_amt + 15 * full_chunks
        };
        unsafe {
            self.buf.set_filled(filled_amount);
        }
        Ok(())
    }
}

#[inline]
fn decode_full_chunk(out_chunk: &mut [MaybeUninit<u8>; 15], in_chunk: &[u16; 8]) {
    IntoIterator::into_iter(
        in_chunk
            .iter()
            .copied()
            .enumerate()
            .map(|(idx, word)| (word as u128) << (CODE_LEN * idx))
            .reduce(core::ops::BitOr::bitor)
            .unwrap()
            .to_le_bytes(),
    )
    .zip(out_chunk)
    .for_each(|(byte, place)| {
        place.write(byte);
    });
}

fn decode_partial_final_chunk(
    out_chunk: &mut [MaybeUninit<u8>; 15],
    in_chunk: &[u16],
) -> Result<usize, DecoderError> {
    let Some((combined, used_bits)) = in_chunk
        .iter()
        .copied()
        .enumerate()
        .map(|(idx, word)| {
            if word & 0x8000 != 0 {
                (((word & 0x7F) as u128) << (CODE_LEN * idx), SMALL_LEN)
            } else {
                ((word as u128) << (CODE_LEN * idx), CODE_LEN)
            }
        })
        .reduce(|(acc, bits), (shifted_word, new_bits)| (acc | shifted_word, bits + new_bits))
    else {
        return Ok(0);
    };
    let bytes = combined.to_le_bytes();
    let full_bytes = used_bits / BYTE_SIZE;
    let padding_bits = used_bits % BYTE_SIZE;
    for i in 0..full_bytes {
        out_chunk[i].write(bytes[i]);
    }
    if bytes[full_bytes].trailing_ones() as usize != padding_bits {
        Err(DecoderError::InvalidPadding(combined as u8))
    } else {
        Ok(full_bytes)
    }
}

impl<I: ?Sized + Iterator<Item = u16>, const N: usize> BufRead for ReadDecoder<I, N> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.buf.is_all_consumed() {
            self.refill_buf()?;
        }
        Ok(self.buf.as_slice())
    }

    fn consume(&mut self, amt: usize) {
        self.buf.consume(amt);
    }
}

impl<I: ?Sized + Iterator<Item = u16>, const N: usize> Read for ReadDecoder<I, N> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let bytes = self.fill_buf()?;
        let to_copy = usize::min(bytes.len(), buf.len());
        buf[..to_copy].copy_from_slice(&bytes[..to_copy]);
        self.consume(to_copy);
        Ok(to_copy)
    }

    fn read_exact(&mut self, mut buf: &mut [u8]) -> std::io::Result<()> {
        if self
            .buf
            .consume_with(buf.len(), |bytes| buf.copy_from_slice(bytes))
        {
            return Ok(());
        }

        while !buf.is_empty() {
            let bytes = self.fill_buf()?;
            if bytes.is_empty() {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            let written = buf.write(bytes)?;
            self.consume(written);
        }

        Ok(())
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> std::io::Result<usize> {
        let mut read = 0;
        let existing = self.buf.as_slice();
        read += existing.len();
        buf.extend_from_slice(existing);
        self.buf.consume(existing.len());

        let (low, high) = self.iter.size_hint();
        buf.reserve((high.unwrap_or(low) * 15) / 8 + 1);
        self.refill_buf()?;
        while !self.buf.is_all_consumed() {
            let slice = self.buf.as_slice();
            buf.extend_from_slice(slice);
            read += slice.len();
            self.refill_buf()?;
        }
        Ok(read)
    }
}
