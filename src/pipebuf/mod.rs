use crate::optimized::{get_lookups, DecoderError, BYTE_SIZE, CODE_LEN, SMALL_LEN};
use pipebuf::{tripwire, PBufRd, PBufWr};

fn encode_full_block(src: &[u8; CODE_LEN], dst: &mut [u16; BYTE_SIZE]) {
    let tables = get_lookups();
    let block = u128::from_le_bytes([
        src[0], src[1], src[2], src[3], src[4], src[5], src[6], src[7], src[8], src[9], src[10],
        src[11], src[12], src[13], src[14], 0,
    ]);
    dst.iter_mut()
        .enumerate()
        .map(|(idx, place)| (idx * CODE_LEN, place))
        .for_each(|(shift, place)| {
            *place = tables.long_encode[(block >> shift) as usize & 0x7FFF];
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
        let mut backpressure = false;
        while bytes.len() >= CODE_LEN {
            let Some(space) = utf32768.maybe_space(BYTE_SIZE) else {
                backpressure = true;
                break;
            };
            encode_full_block(
                &bytes.data()[..CODE_LEN].try_into().unwrap(),
                space.try_into().unwrap(),
            );
            bytes.consume(CODE_LEN);
            utf32768.commit(BYTE_SIZE);
        }
        if !backpressure && bytes.has_pending_eof() {
            if let Some(space) = utf32768.maybe_space(BYTE_SIZE) {
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

pub fn decode_utf32768_to_u15(
    mut utf32768: PBufRd<'_, u16>,
    mut u15s: PBufWr<'_, u16>,
) -> Result<bool, DecoderError> {
    let before = tripwire!(utf32768, u15s);

    if utf32768.consume_push() {
        u15s.push();
    }

    if utf32768.is_aborted() && utf32768.consume_eof() {
        u15s.abort();
    } else {
        let data = utf32768.data();
        let len = u15s.free_space().unwrap_or(usize::MAX).min(data.len());
        if len > 0 {
            let space = u15s.space(len);
            let tables = get_lookups();

            data.iter()
                .copied()
                .map(|encoded| {
                    tables
                        .decode
                        .get(encoded as usize)
                        .copied()
                        .filter(|&x| x != 0xFFFF)
                        .ok_or(DecoderError::InvalidCodePoint(encoded))
                })
                .zip(space.iter_mut())
                .try_for_each(|(val_or_err, place)| {
                    *place = val_or_err?;
                    Ok(())
                })?;

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

fn decode_full_block(src: &[u16; BYTE_SIZE], dst: &mut [u8; CODE_LEN]) {
    IntoIterator::into_iter(
        src.iter()
            .copied()
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
            if chunk.iter().copied().any(|x| x & 0x8000 != 0) {
                return Err(DecoderError::UnexpectedEndOfStreamMarker);
            }
            let Some(space) = bytes.maybe_space(CODE_LEN) else {
                backpressure = true;
                break;
            };
            decode_full_block(chunk, space.try_into().unwrap());
            bytes.commit(CODE_LEN);
            u15s.consume(BYTE_SIZE);
        }

        if !backpressure && is_final {
            if bytes.is_eof() {
                return Err(DecoderError::UnexpectedEndOfStreamMarker);
            }
            if let Some(space) = bytes.maybe_space(CODE_LEN) {
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
