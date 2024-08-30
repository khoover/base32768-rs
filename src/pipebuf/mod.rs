use crate::optimized::{get_lookups, DecoderError, BYTE_SIZE, CODE_LEN, SMALL_LEN};
use pipebuf::{tripwire, PBufRd, PBufWr};

fn encode_full_block(src: &[u8; CODE_LEN], dst: &mut [u16; BYTE_SIZE]) {
    let table = &get_lookups().long_encode;
    let block = u128::from_le_bytes([
        src[0], src[1], src[2], src[3], src[4], src[5], src[6], src[7], src[8], src[9], src[10],
        src[11], src[12], src[13], src[14], 0,
    ]);
    dst.iter_mut()
        .enumerate()
        .map(|(idx, place)| (idx * CODE_LEN, place))
        .for_each(|(shift, place)| {
            *place = table[(block >> shift) as usize & 0x7FFF];
        });
}

fn encode_partial_block(src: &[u8], dst: &mut [u16; BYTE_SIZE]) -> usize {
    let mut idx = 0;
    let tables = get_lookups();
    let mut acc: u16 = 0;
    let mut used_bits = 0_u8;

    for byte in src.iter().copied() {
        acc |= (byte as u16) << used_bits;
        used_bits += BYTE_SIZE as u8;
        if used_bits >= CODE_LEN as u8 {
            dst[idx] = tables.long_encode[(acc & 0x7FFF) as usize];
            idx += 1;
            used_bits -= CODE_LEN as u8;
            acc = (byte.rotate_left(used_bits as u32) & !(0xFF << used_bits)) as u16;
        }
    }

    acc |= 0xFFFF << used_bits;
    match used_bits as usize {
        1..=SMALL_LEN => {
            dst[idx] = tables.short_encode[(acc & 0x7F) as usize];
            idx + 1
        }
        CODE_LEN => unreachable!(),
        0 => idx,
        _ => {
            dst[idx] = tables.long_encode[(acc & 0x7FFF) as usize];
            idx + 1
        }
    }
}

pub fn encode_bytes_to_utf32768(mut bytes: PBufRd<'_, u8>, mut utf32768: PBufWr<'_, u16>) -> bool {
    let before = tripwire!(bytes, utf32768);

    if bytes.consume_push() {
        utf32768.push();
    }

    if bytes.is_aborted() && bytes.consume_eof() {
        utf32768.abort();
    } else {
        let backpressure = loop {
            if bytes.len() < CODE_LEN {
                break false;
            }
            let Some(space) = utf32768.try_space(BYTE_SIZE) else {
                break true;
            };
            encode_full_block(
                &bytes.data()[..CODE_LEN].try_into().unwrap(),
                space.try_into().unwrap(),
            );
            bytes.consume(CODE_LEN);
            utf32768.commit(BYTE_SIZE);
        };
        if !backpressure && bytes.has_pending_eof() {
            if let Some(space) = utf32768.try_space(BYTE_SIZE) {
                let committed = encode_partial_block(bytes.data(), space.try_into().unwrap());
                bytes.consume(bytes.len());
                utf32768.commit(committed);
                bytes.consume_eof();
                utf32768.close();
            }
        }
    }

    let after = tripwire!(bytes, utf32768);
    before != after
}

fn decode_utf32768_stream(utf32768: &[u16], u15s: &mut [u16]) -> Option<DecoderError> {
    let table = &get_lookups().decode;
    let mut err: Option<DecoderError> = None;
    utf32768
        .iter()
        .copied()
        .map(|encoded| {
            table
                .get(encoded as usize)
                .copied()
                .filter(|&x| x != 0xFFFF)
                .unwrap_or_else(|| {
                    err = Some(DecoderError::InvalidCodePoint(encoded));
                    0xFFFF
                })
        })
        .zip(u15s.iter_mut())
        .for_each(|(val, place)| {
            *place = val;
        });
    err
}

pub fn decode_utf32768_to_u15(
    mut utf32768: PBufRd<'_, u16>,
    mut u15s: PBufWr<'_, u16>,
) -> Result<bool, DecoderError> {
    let before = tripwire!(utf32768, u15s);

    if utf32768.is_aborted() && utf32768.consume_eof() {
        u15s.abort();
    } else {
        let data = utf32768.data();
        let len = free_space_sizing(u15s.free_space(), data.len());
        if len > 0 {
            let space = u15s.space(len);

            if let Some(e) = decode_utf32768_stream(data, space) {
                u15s.abort();
                return Err(e);
            }
            utf32768.consume(len);
            u15s.commit(len);
        }

        if utf32768.consume_push() {
            u15s.push();
        }

        if utf32768.is_empty() && utf32768.consume_eof() {
            u15s.close();
        }
    }

    let after = tripwire!(utf32768, u15s);
    Ok(before != after)
}

fn decode_full_block(src: &[u16; BYTE_SIZE], dst: &mut [u8; CODE_LEN]) -> Option<DecoderError> {
    let mut err = None;
    IntoIterator::into_iter(
        src.iter()
            .copied()
            .inspect(|&x| {
                if x & 0x8000 != 0 {
                    err = Some(DecoderError::UnexpectedEndOfStreamMarker)
                }
            })
            .enumerate()
            .map(|(idx, word)| (word as u128) << (CODE_LEN * idx))
            .reduce(core::ops::BitOr::bitor)
            .unwrap()
            .to_le_bytes(),
    )
    .zip(dst.iter_mut())
    .for_each(|(val, place)| {
        *place = val;
    });
    err
}

fn decode_partial_final_chunk(
    src: &[u16],
    dst: &mut [u8; CODE_LEN],
) -> Result<usize, DecoderError> {
    let (combined, used_bits) = src
        .iter()
        .copied()
        .enumerate()
        .map(|(idx, word)| {
            if word & 0x8000 != 0 && idx != src.len() - 1 {
                Err(DecoderError::UnexpectedEndOfStreamMarker)
            } else {
                Ok((idx, word))
            }
        })
        .map(|pair_or_err| {
            pair_or_err.map(|(idx, word)| {
                if word & 0x8000 != 0 {
                    (((word & 0x7F) as u128) << (CODE_LEN * idx), SMALL_LEN)
                } else {
                    ((word as u128) << (CODE_LEN * idx), CODE_LEN)
                }
            })
        })
        .try_fold((0, 0), |(acc, bits), val_or_err| {
            val_or_err.map(|(shifted_word, new_bits)| (acc | shifted_word, bits + new_bits))
        })?;

    if used_bits == 0 {
        return Ok(0);
    }

    let bytes = combined.to_le_bytes();
    let full_bytes = used_bits / BYTE_SIZE;
    let padding_bits = used_bits % BYTE_SIZE;
    for i in 0..full_bytes {
        dst[i] = bytes[i];
    }

    if bytes[full_bytes].trailing_ones() as usize != padding_bits {
        Err(DecoderError::InvalidPadding(combined as u8))
    } else {
        Ok(full_bytes)
    }
}

pub fn decode_u15_to_bytes(
    mut u15s: PBufRd<'_, u16>,
    mut bytes: PBufWr<'_, u8>,
) -> Result<bool, DecoderError> {
    let before = tripwire!(u15s, bytes);

    if u15s.is_aborted() && u15s.consume_eof() {
        bytes.abort();
    } else {
        let data = u15s.data();
        let is_final =
            data.last().is_some_and(|&last| last & 0x8000 != 0) || u15s.has_pending_eof();
        let threshold = if is_final { BYTE_SIZE } else { BYTE_SIZE - 1 };
        let mut backpressure = false;

        while u15s.len() > threshold {
            let chunk: &[u16; BYTE_SIZE] = u15s.data()[..BYTE_SIZE].try_into().unwrap();
            let Some(space) = bytes.try_space(CODE_LEN) else {
                backpressure = true;
                break;
            };
            if let Some(err) = decode_full_block(chunk, space.try_into().unwrap()) {
                bytes.abort();
                return Err(err);
            }
            bytes.commit(CODE_LEN);
            u15s.consume(BYTE_SIZE);
        }

        if !backpressure && is_final {
            if bytes.is_eof() || u15s.len() > BYTE_SIZE {
                return Err(DecoderError::UnexpectedEndOfStreamMarker);
            }
            if let Some(space) = bytes.try_space(CODE_LEN) {
                let committed = decode_partial_final_chunk(u15s.data(), space.try_into().unwrap())?;
                u15s.consume(u15s.len());
                u15s.consume_eof();
                bytes.commit(committed);
                bytes.close();
            }
        } else if u15s.consume_push() {
            bytes.push();
        }
    }

    let after = tripwire!(u15s, bytes);
    Ok(before != after)
}

fn encode_full_block_utf8(src: &[u8; CODE_LEN], dst: &mut PBufWr<'_, u8>) {
    let tables = get_lookups();
    let block = u128::from_le_bytes([
        src[0], src[1], src[2], src[3], src[4], src[5], src[6], src[7], src[8], src[9], src[10],
        src[11], src[12], src[13], src[14], 0,
    ]);
    (0..8)
        .map(|idx| {
            let u15 = (block >> (idx * CODE_LEN)) as usize & 0x7FFF;
            let encoding = tables.long_encode[u15] as u32;
            char::from_u32(encoding).unwrap()
        })
        .for_each(|c| {
            let written = c.encode_utf8(dst.space(3)).len();
            dst.commit(written);
        });
}

fn encode_partial_block_utf8(src: &[u8], dst: &mut PBufWr<'_, u8>) {
    let tables = get_lookups();
    let mut acc: u16 = 0;
    let mut used_bits = 0_u8;

    for byte in src.iter().copied() {
        acc |= (byte as u16) << used_bits;
        used_bits += BYTE_SIZE as u8;
        if used_bits >= CODE_LEN as u8 {
            let written = tables.long_encode_char[(acc & 0x7FFF) as usize]
                .encode_utf8(dst.space(3))
                .len();
            dst.commit(written);
            used_bits -= CODE_LEN as u8;
            acc = (byte.rotate_left(used_bits as u32) & !(0xFF << used_bits)) as u16;
        }
    }

    acc |= 0xFFFF << used_bits;
    let encoded = match used_bits as usize {
        1..=SMALL_LEN => tables.short_encode_char[(acc & 0x7F) as usize],
        CODE_LEN => unreachable!(),
        0 => {
            return;
        }
        _ => tables.long_encode_char[(acc & 0x7FFF) as usize],
    };
    let written = encoded.encode_utf8(dst.space(3)).len();
    dst.commit(written);
}

pub fn encode_bytes_to_base32768_utf8(
    mut bytes: PBufRd<'_, u8>,
    mut base32768: PBufWr<'_, u8>,
) -> bool {
    let before = tripwire!(bytes, base32768);

    if bytes.consume_push() {
        base32768.push();
    }

    if bytes.is_aborted() && bytes.consume_eof() {
        base32768.abort();
    } else {
        let mut backpressure = false;
        while bytes.len() >= CODE_LEN {
            if base32768
                .free_space()
                .is_some_and(|free_space| free_space < 24)
            {
                backpressure = true;
                break;
            }
            encode_full_block_utf8(
                &bytes.data()[..CODE_LEN].try_into().unwrap(),
                &mut base32768,
            );
            bytes.consume(CODE_LEN);
        }
        if !backpressure
            && bytes.has_pending_eof()
            && !base32768
                .free_space()
                .is_some_and(|free_space| free_space < 24)
        {
            encode_partial_block_utf8(bytes.data(), &mut base32768);
            bytes.consume(bytes.len());
            bytes.consume_eof();
            base32768.close();
        }
    }

    let after = tripwire!(bytes, base32768);
    before != after
}

fn free_space_sizing(free_space: Option<usize>, target_space: usize) -> usize {
    match free_space {
        None => target_space,
        Some(x) => x.min(target_space),
    }
}
