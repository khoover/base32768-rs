pub fn encode(bytes: &[u8]) -> Box<[u16]> {
    let mut out = Vec::with_capacity((bytes.len() * 8 + 14) / 15);

    let mut chunks = bytes.chunks_exact(15);
    let mut buf = [0; 8];
    chunks
        .by_ref()
        .map(|chunk| {
            u128::from_le_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
                chunk[8], chunk[9], chunk[10], chunk[11], chunk[12], chunk[13], chunk[14], 0,
            ])
        })
        .for_each(|big_word| {
            buf.iter_mut().fold(big_word, |word, place| {
                *place = (word as u16) & 0x7FFF;
                word >> 15
            });
            out.extend_from_slice(&buf);
        });

    let mut num = 0u128;
    for (i, &b) in chunks.remainder().iter().enumerate() {
        num |= (b as u128) << (i * 8);
    }
    for _ in 0..out.capacity() - out.len() {
        out.push(num as u16 & 0x7FFF);
        num >>= 15
    }

    debug_assert_eq!(out.capacity(), out.len());

    let rem = (bytes.len() * 8) % 15;
    if rem > 0 && rem <= 7 {
        *out.last_mut().unwrap() |= 0x8000; // Set last bit to indicate that it only contains 7 of less bits
    }
    out.into_boxed_slice()
}

pub fn decode(chars: &[u16]) -> Box<[u8]> {
    if chars.is_empty() {
        return vec![].into_boxed_slice();
    }
    let last_contains_few = *chars.last().unwrap() >= 0x8000;

    let num_bytes =
        (chars.len() * 15 - 15 + if last_contains_few { 0 } else { 8 } as usize + 7) / 8;

    let mut out = Vec::with_capacity(num_bytes);

    let mut chunks = chars[..chars.len() - 1].chunks_exact(8);
    while let Some(byte_chunk) = chunks.next() {
        let mut num = 0u128;
        for (i, &b) in byte_chunk.iter().enumerate() {
            num |= (b as u128) << (i * 15);
        }
        for _ in 0..15 {
            out.push(num as u8);
            num >>= 8;
        }
    }

    let remainder = chunks.remainder();
    let mut num = (chars[chars.len() - 1] as u128) << (remainder.len() * 15);
    for (i, &b) in remainder.iter().enumerate() {
        num |= (b as u128) << (i * 15);
    }
    for _ in 0..num_bytes - out.len() {
        out.push(num as u8);
        num >>= 8;
    }

    debug_assert_eq!(out.capacity(), out.len());

    out.into_boxed_slice()
}
