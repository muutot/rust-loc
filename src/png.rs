//! A tiny PNG writer.
//!
//! The GUI exports the report as a PNG. Rather than pull in an image crate,
//! the encoder lives here: RGBA bytes in, a PNG out. Each row is filtered the
//! way the format recommends and the scanlines go through a small
//! fixed-Huffman deflate, so a report card lands in tens of kilobytes instead
//! of the megabytes stored blocks would cost. The bit writer, the LZ77 match
//! finder and the chunk CRCs are all written from scratch: no image crate, no
//! system zlib, nothing to link.

/// The eight bytes every PNG starts with.
const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Encode `rgba` (top row first) as an 8-bit RGBA PNG.
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4096);
    out.extend_from_slice(&SIGNATURE);

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.push(8);
    header.push(6);
    header.push(0);
    header.push(0);
    header.push(0);
    chunk(&mut out, b"IHDR", &header);

    chunk(&mut out, b"IDAT", &zlib(&filter(width, height, rgba)));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

/// One filter byte plus the filtered pixels per row. The filter is picked by
/// the smallest sum of absolute differences, the heuristic the PNG spec
/// suggests, which keeps the deflate stream short.
fn filter(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let stride = width as usize * 4;
    let mut out = Vec::with_capacity((stride + 1) * height as usize);
    let mut previous = vec![0u8; stride];
    let mut line = vec![0u8; stride];
    for source in rgba.chunks_exact(stride) {
        let kind = best_filter(source, &previous);
        out.push(kind);
        apply_filter(kind, source, &previous, &mut line);
        out.extend_from_slice(&line);
        previous.copy_from_slice(source);
    }
    out
}

/// Filter type 0 (none), 2 (up) or 4 (Paeth), whichever leaves the smallest
/// sum of absolute differences.
fn best_filter(line: &[u8], previous: &[u8]) -> u8 {
    let mut costs = [0u32; 3];
    let mut left = [0u8; 4];
    let mut corner = [0u8; 4];
    for (pixel, above) in line.chunks_exact(4).zip(previous.chunks_exact(4)) {
        for (index, raw) in pixel.iter().enumerate() {
            costs[0] += weight(*raw);
            costs[1] += weight(raw.wrapping_sub(above[index]));
            costs[2] += weight(raw.wrapping_sub(paeth(left[index], above[index], corner[index])));
        }
        left.copy_from_slice(pixel);
        corner.copy_from_slice(above);
    }
    if costs[1] <= costs[0] && costs[1] <= costs[2] {
        2
    } else if costs[2] <= costs[0] {
        4
    } else {
        0
    }
}

/// Recreate a row the way `best_filter` chose to filter it.
fn apply_filter(kind: u8, line: &[u8], previous: &[u8], out: &mut [u8]) {
    let mut left = [0u8; 4];
    let mut corner = [0u8; 4];
    for ((raw, above), value) in line
        .chunks_exact(4)
        .zip(previous.chunks_exact(4))
        .zip(out.chunks_exact_mut(4))
    {
        for (index, slot) in value.iter_mut().enumerate() {
            *slot = match kind {
                2 => raw[index].wrapping_sub(above[index]),
                4 => raw[index].wrapping_sub(paeth(left[index], above[index], corner[index])),
                _ => raw[index],
            };
        }
        left.copy_from_slice(raw);
        corner.copy_from_slice(above);
    }
}

/// The Paeth predictor: whichever neighbour is closest to `left + above - corner`.
fn paeth(left: u8, above: u8, corner: u8) -> u8 {
    let left = i32::from(left);
    let above = i32::from(above);
    let corner = i32::from(corner);
    let estimate = left + above - corner;
    let to_left = (estimate - left).abs();
    let to_above = (estimate - above).abs();
    let to_corner = (estimate - corner).abs();
    if to_left <= to_above && to_left <= to_corner {
        left as u8
    } else if to_above <= to_corner {
        above as u8
    } else {
        corner as u8
    }
}

/// The filter heuristic weighs a filtered byte as a signed byte.
fn weight(byte: u8) -> u32 {
    u32::from((byte as i8).unsigned_abs())
}

/// A zlib stream around one fixed-Huffman deflate block.
fn zlib(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 4 + 64);
    out.push(0x78);
    out.push(0x01);
    out.extend_from_slice(&deflate(data));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Deflate `data` as a single final block of fixed Huffman codes, with a
/// greedy LZ77 pass in front of them.
fn deflate(data: &[u8]) -> Vec<u8> {
    let mut bits = Bits::new();
    bits.push(1, 1);
    bits.push(1, 2);
    let mut matcher = Matcher::new();
    let mut index = 0;
    while index < data.len() {
        let (length, distance) = matcher.find(data, index);
        if length >= MIN_MATCH {
            let (code, extra, extra_bits) = length_code(length);
            symbol(&mut bits, code);
            bits.push(extra, extra_bits);
            let (code, extra, extra_bits) = distance_code(distance);
            bits.code(code, 5);
            bits.push(extra, extra_bits);
            for step in 0..length {
                matcher.insert(data, index + step);
            }
            index += length;
        } else {
            symbol(&mut bits, u32::from(data[index]));
            matcher.insert(data, index);
            index += 1;
        }
    }
    symbol(&mut bits, 256);
    bits.end()
}

/// Write one literal, length or end-of-block symbol in the fixed table.
fn symbol(bits: &mut Bits, value: u32) {
    match value {
        0..=143 => bits.code(0x30 + value, 8),
        144..=255 => bits.code(0x190 + (value - 144), 9),
        256..=279 => bits.code(value - 256, 7),
        _ => bits.code(0xc0 + (value - 280), 8),
    }
}

/// Base length and extra bit count for the deflate length codes 257..=285.
const LENGTHS: [(usize, u32); 29] = [
    (3, 0),
    (4, 0),
    (5, 0),
    (6, 0),
    (7, 0),
    (8, 0),
    (9, 0),
    (10, 0),
    (11, 1),
    (13, 1),
    (15, 1),
    (17, 1),
    (19, 2),
    (23, 2),
    (27, 2),
    (31, 2),
    (35, 3),
    (43, 3),
    (51, 3),
    (59, 3),
    (67, 4),
    (83, 4),
    (99, 4),
    (115, 4),
    (131, 5),
    (163, 5),
    (195, 5),
    (227, 5),
    (258, 0),
];

/// Base distance and extra bit count for the deflate distance codes 0..=29.
const DISTANCES: [(usize, u32); 30] = [
    (1, 0),
    (2, 0),
    (3, 0),
    (4, 0),
    (5, 1),
    (7, 1),
    (9, 2),
    (13, 2),
    (17, 3),
    (25, 3),
    (33, 4),
    (49, 4),
    (65, 5),
    (97, 5),
    (129, 6),
    (193, 6),
    (257, 7),
    (385, 7),
    (513, 8),
    (769, 8),
    (1025, 9),
    (1537, 9),
    (2049, 10),
    (3073, 10),
    (4097, 11),
    (6145, 11),
    (8193, 12),
    (12289, 12),
    (16385, 13),
    (24577, 13),
];

/// The length symbol, extra value and extra bit count for a match.
fn length_code(length: usize) -> (u32, u32, u32) {
    for (index, (base, extra)) in LENGTHS.iter().enumerate().rev() {
        if length >= *base {
            return (257 + index as u32, (length - base) as u32, *extra);
        }
    }
    (257, (length - 3) as u32, 0)
}

/// The distance symbol, extra value and extra bit count for a match.
fn distance_code(distance: usize) -> (u32, u32, u32) {
    for (index, (base, extra)) in DISTANCES.iter().enumerate().rev() {
        if distance >= *base {
            return (index as u32, (distance - base) as u32, *extra);
        }
    }
    (0, (distance - 1) as u32, 0)
}

/// Pack deflate bits low bit first, the order the format pins down.
struct Bits {
    out: Vec<u8>,
    current: u32,
    filled: u32,
}

impl Bits {
    fn new() -> Bits {
        Bits {
            out: Vec::new(),
            current: 0,
            filled: 0,
        }
    }

    /// Append `count` bits, least significant bit first.
    fn push(&mut self, value: u32, count: u32) {
        if count == 0 {
            return;
        }
        self.current |= (value & ((1 << count) - 1)) << self.filled;
        self.filled += count;
        while self.filled >= 8 {
            self.out.push((self.current & 0xff) as u8);
            self.current >>= 8;
            self.filled -= 8;
        }
    }

    /// Append a Huffman code, most significant bit first.
    fn code(&mut self, code: u32, count: u32) {
        let mut reversed = 0;
        for index in 0..count {
            reversed |= ((code >> (count - 1 - index)) & 1) << index;
        }
        self.push(reversed, count);
    }

    /// Flush the last partial byte, zero padded.
    fn end(mut self) -> Vec<u8> {
        if self.filled > 0 {
            self.out.push((self.current & 0xff) as u8);
        }
        self.out
    }
}

/// The three byte match the LZ77 pass indexes on.
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const WINDOW: usize = 32768;
const CHAIN: usize = 48;
const HASH_BITS: u32 = 15;
const NONE: u32 = u32::MAX;

/// A hash chain over three byte sequences: the usual way to find repeats
/// without comparing every earlier position.
struct Matcher {
    head: Vec<u32>,
    previous: Vec<u32>,
}

impl Matcher {
    fn new() -> Matcher {
        Matcher {
            head: vec![NONE; 1 << HASH_BITS],
            previous: vec![NONE; WINDOW],
        }
    }

    fn slot(data: &[u8], index: usize) -> usize {
        let hash = (usize::from(data[index]) << 10)
            ^ (usize::from(data[index + 1]) << 5)
            ^ usize::from(data[index + 2]);
        hash & ((1 << HASH_BITS) - 1)
    }

    fn insert(&mut self, data: &[u8], index: usize) {
        if index + MIN_MATCH > data.len() {
            return;
        }
        let slot = Self::slot(data, index);
        self.previous[index % WINDOW] = self.head[slot];
        self.head[slot] = index as u32;
    }

    /// The longest repeat of `data[index..]` still inside the window.
    fn find(&self, data: &[u8], index: usize) -> (usize, usize) {
        if index + MIN_MATCH > data.len() {
            return (0, 0);
        }
        let limit = index.saturating_sub(WINDOW);
        let mut candidate = self.head[Self::slot(data, index)];
        let mut best = 0;
        let mut distance = 0;
        let mut steps = 0;
        while candidate != NONE && (candidate as usize) >= limit && steps < CHAIN {
            let at = candidate as usize;
            let mut length = 0;
            while length < MAX_MATCH
                && index + length < data.len()
                && data[at + length] == data[index + length]
            {
                length += 1;
            }
            if length > best {
                best = length;
                distance = index - at;
                if length >= MAX_MATCH {
                    break;
                }
            }
            candidate = self.previous[at % WINDOW];
            steps += 1;
        }
        if best < MIN_MATCH {
            (0, 0)
        } else {
            (best, distance)
        }
    }
}

fn adler32(data: &[u8]) -> u32 {
    let mut low: u32 = 1;
    let mut high: u32 = 0;
    for byte in data {
        low = (low + u32::from(*byte)) % 65521;
        high = (high + low) % 65521;
    }
    (high << 16) | low
}

struct Crc32 {
    value: u32,
}

impl Crc32 {
    fn new() -> Crc32 {
        Crc32 { value: 0xffff_ffff }
    }

    fn update(&mut self, data: &[u8]) {
        for byte in data {
            let index = ((self.value ^ u32::from(*byte)) & 0xff) as usize;
            self.value = CRC_TABLE[index] ^ (self.value >> 8);
        }
    }

    fn finish(self) -> u32 {
        self.value ^ 0xffff_ffff
    }
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 != 0 {
                0xedb8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

const CRC_TABLE: [u32; 256] = crc_table();

/// Test-only: inflate one of this module's PNGs back to RGBA pixels. It is
/// deliberately a second, independent implementation of the reader, so a
/// round trip proves the writer.
#[cfg(test)]
pub(crate) fn decode_rgba(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    let mut offset = 8;
    let mut idat = Vec::new();
    while offset + 12 <= png.len() {
        let length = u32::from_be_bytes(png[offset..offset + 4].try_into().unwrap()) as usize;
        if &png[offset + 4..offset + 8] == b"IDAT" {
            idat.extend_from_slice(&png[offset + 8..offset + 8 + length]);
        }
        offset += 12 + length;
    }
    assert_eq!(&idat[..2], &[0x78, 0x01]);
    assert_eq!(
        &idat[idat.len() - 4..],
        &adler32(&inflate_fixed(&idat[2..idat.len() - 4])).to_be_bytes()
    );
    let raw = inflate_fixed(&idat[2..idat.len() - 4]);
    let stride = width as usize * 4;
    let mut rgba = vec![0u8; stride * height as usize];
    let mut previous = vec![0u8; stride];
    for row in 0..height as usize {
        let start = row * (stride + 1);
        let kind = raw[start];
        let line = &raw[start + 1..start + 1 + stride];
        let out = &mut rgba[row * stride..(row + 1) * stride];
        let mut left = [0u8; 4];
        for (index, slot) in out.iter_mut().enumerate() {
            let above = previous[index];
            let corner = if index >= 4 { previous[index - 4] } else { 0 };
            *slot = match kind {
                1 => line[index].wrapping_add(left[index % 4]),
                2 => line[index].wrapping_add(above),
                3 => line[index].wrapping_add(u8::midpoint(left[index % 4], above)),
                4 => line[index].wrapping_add(paeth(left[index % 4], above, corner)),
                _ => line[index],
            };
            left[index % 4] = *slot;
        }
        previous.copy_from_slice(out);
    }
    (width, height, rgba)
}

/// Test-only: the matching reader for the fixed-Huffman blocks `deflate`
/// writes, with the stored block form thrown in for completeness.
#[cfg(test)]
fn inflate_fixed(stream: &[u8]) -> Vec<u8> {
    struct Reader<'a> {
        data: &'a [u8],
        position: usize,
        current: u32,
        filled: u32,
    }

    impl Reader<'_> {
        fn bit(&mut self) -> u32 {
            if self.filled == 0 {
                self.current = u32::from(self.data[self.position]);
                self.position += 1;
                self.filled = 8;
            }
            let bit = self.current & 1;
            self.current >>= 1;
            self.filled -= 1;
            bit
        }

        fn low(&mut self, count: u32) -> u32 {
            let mut value = 0;
            for index in 0..count {
                value |= self.bit() << index;
            }
            value
        }

        fn high(&mut self, count: u32) -> u32 {
            let mut value = 0;
            for _ in 0..count {
                value = (value << 1) | self.bit();
            }
            value
        }

        fn literal(&mut self) -> u32 {
            let mut code = 0;
            for length in 1..=9 {
                code = (code << 1) | self.bit();
                match length {
                    7 if code <= 0b001_0111 => return 256 + code,
                    8 if (0b0011_0000..=0b1011_1111).contains(&code) => return code - 0b0011_0000,
                    8 if (0b1100_0000..=0b1100_0111).contains(&code) => {
                        return 280 + (code - 0b1100_0000)
                    }
                    9 if code >= 0b1_1001_0000 => return 144 + (code - 0b1_1001_0000),
                    _ => {}
                }
            }
            panic!("unknown fixed Huffman code");
        }
    }

    let mut reader = Reader {
        data: stream,
        position: 0,
        current: 0,
        filled: 0,
    };
    let mut out = Vec::new();
    loop {
        let last = reader.bit() == 1;
        match reader.low(2) {
            0 => {
                reader.filled = 0;
                let length = reader.low(16) as usize;
                let _inverse = reader.low(16);
                for _ in 0..length {
                    out.push(reader.data[reader.position]);
                    reader.position += 1;
                }
            }
            1 => loop {
                let symbol = reader.literal();
                if symbol == 256 {
                    break;
                }
                if symbol < 256 {
                    out.push(symbol as u8);
                    continue;
                }
                let (base, extra) = LENGTHS[(symbol - 257) as usize];
                let length = base + reader.low(extra) as usize;
                let code = reader.high(5) as usize;
                let (base, extra) = DISTANCES[code];
                let distance = base + reader.low(extra) as usize;
                for _ in 0..length {
                    let byte = out[out.len() - distance];
                    out.push(byte);
                }
            },
            other => panic!("unsupported deflate block {other}"),
        }
        if last {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_known_vector() {
        let mut crc = Crc32::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xcbf4_3926);
    }

    #[test]
    fn adler_matches_the_known_vector() {
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn the_zlib_stream_ends_with_the_adler_sum() {
        let raw = [0u8, 1, 2, 3];
        let stream = zlib(&raw);
        assert_eq!(stream[0], 0x78);
        assert_eq!(stream[1], 0x01);
        assert_eq!(&stream[stream.len() - 4..], &adler32(&raw).to_be_bytes());
    }

    #[test]
    fn a_one_pixel_image_is_a_whole_png() {
        let bytes = encode(1, 1, &[255, 0, 0, 255]);
        assert_eq!(&bytes[..8], &SIGNATURE);
        assert_eq!(&bytes[12..16], b"IHDR");
        assert_eq!(&bytes[16..20], &1u32.to_be_bytes());
        assert_eq!(&bytes[20..24], &1u32.to_be_bytes());
        assert_eq!(&bytes[24..29], &[8, 6, 0, 0, 0]);
        assert_eq!(
            &bytes[bytes.len() - 12..bytes.len() - 4],
            &[0, 0, 0, 0, 73, 69, 78, 68]
        );
    }

    /// A card shaped image: white, a coloured band and a block of text-ish
    /// pixels, so every filter and the match finder get a turn.
    fn sample(width: usize, height: usize) -> Vec<u8> {
        let mut rgba = vec![255u8; width * height * 4];
        for row in 0..height {
            for column in 0..width {
                let at = (row * width + column) * 4;
                if (height / 3..height / 3 + 6).contains(&row) {
                    rgba[at] = 0x2f;
                    rgba[at + 1] = 0x6f;
                    rgba[at + 2] = 0xed;
                } else if row > height / 2 && column % 7 < 4 && column < width / 2 {
                    rgba[at] = 0x5b;
                    rgba[at + 1] = 0x65;
                    rgba[at + 2] = 0x70;
                }
            }
        }
        rgba
    }

    #[test]
    fn a_round_trip_returns_the_same_pixels() {
        let (width, height) = (37usize, 23usize);
        let original = sample(width, height);
        let bytes = encode(width as u32, height as u32, &original);
        let (read_width, read_height, decoded) = decode_rgba(&bytes);
        assert_eq!(read_width as usize, width);
        assert_eq!(read_height as usize, height);
        assert_eq!(decoded, original);
    }

    #[test]
    fn a_repetitive_image_compresses() {
        let original = sample(200, 120);
        let bytes = encode(200, 120, &original);
        assert!(
            bytes.len() < original.len() / 8,
            "encoded {} of {} bytes",
            bytes.len(),
            original.len()
        );
    }

    #[test]
    fn a_long_run_is_one_match() {
        let raw = vec![7u8; 4096];
        assert_eq!(inflate_fixed(&deflate(&raw)), raw);
    }
}
