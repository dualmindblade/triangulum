//! SHA-256 (FIPS 180-4), dependency-free, for save manifests: the
//! projection table's digest (checked against the one `crates/cubesphere`
//! documents) and the bake's identity.

static K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Clone)]
pub struct Sha256 {
    h: [u32; 8],
    buf: Vec<u8>,
    len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Self { h: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19], buf: Vec::new(), len: 0 }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        // Top up a partial block first; whole blocks are compressed straight
        // from the input (the bake's identity hashes ~560 MB at start-up).
        if !self.buf.is_empty() {
            let take = (64 - self.buf.len()).min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buf.len() < 64 {
                return;
            }
            let block = std::mem::take(&mut self.buf);
            compress_blocks(&mut self.h, &block);
        }
        let full = data.len() / 64 * 64;
        compress_blocks(&mut self.h, &data[..full]);
        self.buf.extend_from_slice(&data[full..]);
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.len.wrapping_mul(8);
        let mut tail = std::mem::take(&mut self.buf);
        tail.push(0x80);
        while tail.len() % 64 != 56 {
            tail.push(0);
        }
        tail.extend_from_slice(&bits.to_be_bytes());
        compress_blocks(&mut self.h, &tail);
        let mut out = [0u8; 32];
        for (i, w) in self.h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        out
    }
}

/// Compress whole 64-byte blocks: with the CPU's SHA instructions where it
/// has them (x86-64 SHA-NI: AMD since Zen, Intel since Ice Lake; about six
/// times the portable code), the portable code otherwise. Both give the
/// same digest (`the_sha_instructions_agree_with_the_portable_code`).
fn compress_blocks(h: &mut [u32; 8], blocks: &[u8]) {
    debug_assert_eq!(blocks.len() % 64, 0);
    if blocks.is_empty() {
        return;
    }
    #[cfg(target_arch = "x86_64")]
    if shani::available() {
        // SAFETY: the CPU has the instructions (checked just above).
        unsafe { shani::compress(h, blocks) };
        return;
    }
    for block in blocks.chunks_exact(64) {
        compress(h, block);
    }
}

#[cfg(target_arch = "x86_64")]
mod shani {
    use super::K;
    use std::arch::x86_64::*;

    pub fn available() -> bool {
        std::is_x86_feature_detected!("sha") && std::is_x86_feature_detected!("sse4.1") && std::is_x86_feature_detected!("ssse3")
    }

    /// FIPS 180-4 compression with `sha256rnds2` (two rounds), `sha256msg1`
    /// and `sha256msg2` (the message schedule), after Intel's reference
    /// sequence: the state is kept as ABEF and CDGH, four message words per
    /// group, sixteen groups per block.
    ///
    /// # Safety
    /// The CPU must support SHA, SSE4.1 and SSSE3 (`available`).
    #[target_feature(enable = "sha,sse2,ssse3,sse4.1")]
    pub unsafe fn compress(h: &mut [u32; 8], blocks: &[u8]) {
        // SAFETY (whole body): unaligned loads and stores within `h` (8
        // words), `blocks` (whole 64-byte blocks) and `K` (64 words).
        unsafe {
            let swap = _mm_set_epi64x(0x0c0d_0e0f_0809_0a0b, 0x0405_0607_0001_0203);
            let dcba = _mm_loadu_si128(h.as_ptr() as *const __m128i);
            let hgfe = _mm_loadu_si128(h.as_ptr().add(4) as *const __m128i);
            let cdab = _mm_shuffle_epi32(dcba, 0xB1);
            let efgh = _mm_shuffle_epi32(hgfe, 0x1B);
            let mut abef = _mm_alignr_epi8(cdab, efgh, 8);
            let mut cdgh = _mm_blend_epi16(efgh, cdab, 0xF0);
            for block in blocks.chunks_exact(64) {
                let (abef0, cdgh0) = (abef, cdgh);
                let p = block.as_ptr() as *const __m128i;
                let mut w = [
                    _mm_shuffle_epi8(_mm_loadu_si128(p), swap),
                    _mm_shuffle_epi8(_mm_loadu_si128(p.add(1)), swap),
                    _mm_shuffle_epi8(_mm_loadu_si128(p.add(2)), swap),
                    _mm_shuffle_epi8(_mm_loadu_si128(p.add(3)), swap),
                ];
                for g in 0..16 {
                    let wg = if g < 4 {
                        w[g]
                    } else {
                        // W[t] = s1(W[t-2]) + W[t-7] + s0(W[t-15]) + W[t-16], four at a time.
                        let t = _mm_sha256msg1_epu32(w[g % 4], w[(g + 1) % 4]);
                        let t = _mm_add_epi32(t, _mm_alignr_epi8(w[(g + 3) % 4], w[(g + 2) % 4], 4));
                        let n = _mm_sha256msg2_epu32(t, w[(g + 3) % 4]);
                        w[g % 4] = n;
                        n
                    };
                    let msg = _mm_add_epi32(wg, _mm_loadu_si128(K.as_ptr().add(4 * g) as *const __m128i));
                    cdgh = _mm_sha256rnds2_epu32(cdgh, abef, msg);
                    abef = _mm_sha256rnds2_epu32(abef, cdgh, _mm_shuffle_epi32(msg, 0x0E));
                }
                abef = _mm_add_epi32(abef, abef0);
                cdgh = _mm_add_epi32(cdgh, cdgh0);
            }
            let feba = _mm_shuffle_epi32(abef, 0x1B);
            let dchg = _mm_shuffle_epi32(cdgh, 0xB1);
            let dcba = _mm_blend_epi16(feba, dchg, 0xF0);
            let hgfe = _mm_alignr_epi8(dchg, feba, 8);
            _mm_storeu_si128(h.as_mut_ptr() as *mut __m128i, dcba);
            _mm_storeu_si128(h.as_mut_ptr().add(4) as *mut __m128i, hgfe);
        }
    }
}

fn compress(h: &mut [u32; 8], block: &[u8]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
        *x = x.wrapping_add(y);
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut s = Sha256::new();
    s.update(data);
    s.finish()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        // Two blocks, and an update split mid-block.
        let msg = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(hex(&sha256(msg)), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        let mut s = Sha256::new();
        s.update(&msg[..10]);
        s.update(&msg[10..]);
        assert_eq!(s.finish(), sha256(msg));
        assert_eq!(hex(&sha256(&[0x61u8; 1_000_000])), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    /// The SHA instructions and the portable code give the same digest, for
    /// every length around the block and padding boundaries and for inputs
    /// fed in uneven pieces.
    #[test]
    fn the_sha_instructions_agree_with_the_portable_code() {
        let portable = |data: &[u8]| -> [u32; 8] {
            let mut h = Sha256::new().h;
            let bits = (data.len() as u64).wrapping_mul(8);
            let mut m = data.to_vec();
            m.push(0x80);
            while m.len() % 64 != 56 {
                m.push(0);
            }
            m.extend_from_slice(&bits.to_be_bytes());
            for b in m.chunks_exact(64) {
                compress(&mut h, b);
            }
            h
        };
        let words = |d: [u8; 32]| -> [u32; 8] { std::array::from_fn(|i| u32::from_be_bytes(d[i * 4..i * 4 + 4].try_into().unwrap())) };
        let data: Vec<u8> = (0..5000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8).collect();
        for n in (0..300).chain([1000, 4095, 4096, 4097, 5000]) {
            assert_eq!(words(sha256(&data[..n])), portable(&data[..n]), "length {n}");
            let mut s = Sha256::new();
            for piece in data[..n].chunks(37) {
                s.update(piece);
            }
            assert_eq!(words(s.finish()), portable(&data[..n]), "length {n} in pieces");
        }
        #[cfg(target_arch = "x86_64")]
        println!("SHA instructions: {}", if shani::available() { "used" } else { "not available, portable code only" });
    }

    /// Not part of `cargo test`: hashing throughput (the bake's identity
    /// hashes about 560 MB at every start). `cargo test --release --
    /// --ignored sha256_speed --nocapture`.
    #[test]
    #[ignore]
    fn sha256_speed() {
        let data: Vec<u8> = (0..256u32 << 20).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let t = std::time::Instant::now();
        let mut s = Sha256::new();
        for c in data.chunks(8 << 20) {
            s.update(c);
        }
        let d = s.finish();
        let secs = t.elapsed().as_secs_f64();
        println!("sha256: {} MB in {:.3} s = {:.0} MB/s ({})", data.len() >> 20, secs, (data.len() >> 20) as f64 / secs, hex(&d[..4]));
        let t = std::time::Instant::now();
        let mut h = [0u32; 8];
        for b in data[..64 << 20].chunks_exact(64) {
            compress(&mut h, b);
        }
        let secs = t.elapsed().as_secs_f64();
        println!("portable code: 64 MB in {secs:.3} s = {:.0} MB/s ({:08x})", 64.0 / secs, h[0]);
    }
}
