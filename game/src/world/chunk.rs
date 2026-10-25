//! Palette-compressed 32^3 chunks.
//!
//! Voxel (x, y, z) lives at index x * 1024 + y * 32 + z (z = radial layer).
//! A chunk stores a palette of block ids and one bit-packed palette index per
//! voxel; a uniform chunk stores only its palette entry. Chunks containing
//! water also carry one water-level byte per voxel (0..255 = empty..full).

use super::BlockId;
use crate::grid::CHUNK;

pub const VOXELS: usize = CHUNK * CHUNK * CHUNK;

#[inline]
pub fn index(x: usize, y: usize, z: usize) -> usize {
    (x * CHUNK + y) * CHUNK + z
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    palette: Vec<BlockId>,
    /// Bits per index (0 for a uniform chunk).
    bits: u8,
    /// Packed indices, little-endian within each u64; empty when uniform.
    packed: Vec<u64>,
    /// Water level per voxel, present only when some voxel holds water.
    water: Option<Box<[u8]>>,
}

impl Chunk {
    /// A chunk of one block.
    pub fn uniform(id: BlockId) -> Self {
        Self { palette: vec![id], bits: 0, packed: Vec::new(), water: None }
    }

    /// Compress a dense id array (length VOXELS) and optional water levels.
    pub fn from_dense(ids: &[BlockId], water: Option<&[u8]>) -> Self {
        assert_eq!(ids.len(), VOXELS);
        let mut palette: Vec<BlockId> = Vec::new();
        // Fast path: uniform.
        let first = ids[0];
        if ids.iter().all(|&i| i == first) {
            let mut c = Self::uniform(first);
            c.water = water.and_then(pack_water);
            return c;
        }
        // Palette in order of first appearance (deterministic).
        let mut lut = [u16::MAX; 1 << 16];
        let mut idx = Vec::with_capacity(VOXELS);
        for &id in ids {
            let mut pi = lut[id as usize];
            if pi == u16::MAX {
                pi = palette.len() as u16;
                palette.push(id);
                lut[id as usize] = pi;
            }
            idx.push(pi);
        }
        let bits = bits_for(palette.len());
        let words = (VOXELS * bits as usize).div_ceil(64);
        let mut packed = vec![0u64; words];
        for (v, &pi) in idx.iter().enumerate() {
            let bit = v * bits as usize;
            let (w, off) = (bit / 64, bit % 64);
            packed[w] |= (pi as u64) << off;
            if off + bits as usize > 64 {
                packed[w + 1] |= (pi as u64) >> (64 - off);
            }
        }
        Self { palette, bits, packed, water: water.and_then(pack_water) }
    }

    #[inline]
    pub fn is_uniform(&self) -> bool {
        self.bits == 0
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize, z: usize) -> BlockId {
        if self.bits == 0 {
            return self.palette[0];
        }
        let bit = index(x, y, z) * self.bits as usize;
        let (w, off) = (bit / 64, bit % 64);
        let mask = (1u64 << self.bits) - 1;
        let mut v = self.packed[w] >> off;
        if off + self.bits as usize > 64 {
            v |= self.packed[w + 1] << (64 - off);
        }
        self.palette[(v & mask) as usize]
    }

    /// Water level of a voxel (0 when the chunk has no water layer).
    #[inline]
    pub fn water_level(&self, x: usize, y: usize, z: usize) -> u8 {
        self.water.as_ref().map_or(0, |w| w[index(x, y, z)])
    }

    pub fn has_water(&self) -> bool {
        self.water.is_some()
    }

    /// The water layer, dense, when the chunk has one.
    pub fn water_dense(&self) -> Option<Vec<u8>> {
        self.water.as_ref().map(|w| w.to_vec())
    }

    pub fn palette(&self) -> &[BlockId] {
        &self.palette
    }

    /// Expand to a dense id array.
    pub fn to_dense(&self) -> Vec<BlockId> {
        let mut out = vec![0; VOXELS];
        for x in 0..CHUNK {
            for y in 0..CHUNK {
                for z in 0..CHUNK {
                    out[index(x, y, z)] = self.get(x, y, z);
                }
            }
        }
        out
    }

    /// Heap bytes used.
    pub fn bytes(&self) -> usize {
        self.palette.len() * 2 + self.packed.len() * 8 + self.water.as_ref().map_or(0, |w| w.len())
    }

    /// Inverse of `to_bytes`.
    pub fn from_bytes(b: &[u8]) -> Result<Self, String> {
        let mut at = 0usize;
        let take = |at: &mut usize, n: usize| -> Result<&[u8], String> {
            let s = b.get(*at..*at + n).ok_or_else(|| format!("chunk bytes truncated at {}", *at))?;
            *at += n;
            Ok(s)
        };
        let bits = take(&mut at, 1)?[0];
        if bits > 16 {
            return Err(format!("chunk palette width {bits} bits is impossible"));
        }
        let plen = u16::from_le_bytes(take(&mut at, 2)?.try_into().unwrap()) as usize;
        let mut palette = Vec::with_capacity(plen);
        for _ in 0..plen {
            palette.push(u16::from_le_bytes(take(&mut at, 2)?.try_into().unwrap()));
        }
        if palette.is_empty() || (bits == 0 && palette.len() != 1) || (bits > 0 && palette.len() > (1usize << bits)) {
            return Err(format!("chunk palette of {} entries does not fit {bits} bits", palette.len()));
        }
        let words = if bits == 0 { 0 } else { (VOXELS * bits as usize).div_ceil(64) };
        let mut packed = Vec::with_capacity(words);
        for _ in 0..words {
            packed.push(u64::from_le_bytes(take(&mut at, 8)?.try_into().unwrap()));
        }
        let water = match take(&mut at, 1)?[0] {
            0 => None,
            1 => Some(take(&mut at, VOXELS)?.to_vec().into_boxed_slice()),
            f => return Err(format!("bad water flag {f}")),
        };
        if at != b.len() {
            return Err(format!("{} trailing bytes after chunk", b.len() - at));
        }
        let c = Self { palette, bits, packed, water };
        // Every packed index must address the palette.
        if bits > 0 {
            let mask = (1u64 << bits) - 1;
            for v in 0..VOXELS {
                let bit = v * bits as usize;
                let (w, off) = (bit / 64, bit % 64);
                let mut x = c.packed[w] >> off;
                if off + bits as usize > 64 {
                    x |= c.packed[w + 1] << (64 - off);
                }
                if (x & mask) as usize >= c.palette.len() {
                    return Err(format!("voxel {v} indexes palette entry {} of {}", x & mask, c.palette.len()));
                }
            }
        }
        Ok(c)
    }

    /// Canonical byte serialisation (for determinism tests and saves).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.bits);
        out.extend_from_slice(&(self.palette.len() as u16).to_le_bytes());
        for p in &self.palette {
            out.extend_from_slice(&p.to_le_bytes());
        }
        for w in &self.packed {
            out.extend_from_slice(&w.to_le_bytes());
        }
        match &self.water {
            Some(w) => {
                out.push(1);
                out.extend_from_slice(w);
            }
            None => out.push(0),
        }
        out
    }
}

fn bits_for(palette_len: usize) -> u8 {
    match palette_len {
        0 | 1 => 0,
        n => (usize::BITS - (n - 1).leading_zeros()) as u8,
    }
}

fn pack_water(levels: &[u8]) -> Option<Box<[u8]>> {
    assert_eq!(levels.len(), VOXELS);
    if levels.iter().all(|&l| l == 0) {
        None
    } else {
        Some(levels.to_vec().into_boxed_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_round_trip() {
        // 7 distinct ids -> 3 bits; indices straddle u64 boundaries.
        let mut ids = vec![0u16; VOXELS];
        for (v, id) in ids.iter_mut().enumerate() {
            *id = [0u16, 1, 3, 4, 1000, 65535, 7][(v * 7 + v / 13) % 7];
        }
        let mut water = vec![0u8; VOXELS];
        water[5] = 255;
        water[VOXELS - 1] = 17;
        let c = Chunk::from_dense(&ids, Some(&water));
        assert_eq!(c.bits, 3);
        assert_eq!(c.palette().len(), 7);
        assert_eq!(c.to_dense(), ids);
        assert_eq!(c.water_level(0, 0, 5), 255);
        assert_eq!(c.water_level(31, 31, 31), 17);
        assert_eq!(c.water_level(1, 2, 3), 0);
        // 2 ids -> 1 bit; 3 ids -> 2 bits; 256 ids -> 8 bits; 257 -> 9
        assert_eq!(bits_for(2), 1);
        assert_eq!(bits_for(3), 2);
        assert_eq!(bits_for(256), 8);
        assert_eq!(bits_for(257), 9);
        assert_eq!(bits_for(65536), 16);
    }

    #[test]
    fn uniform_fast_path() {
        let ids = vec![5u16; VOXELS];
        let c = Chunk::from_dense(&ids, None);
        assert!(c.is_uniform());
        assert_eq!(c.bytes(), 2);
        assert_eq!(c.get(31, 0, 17), 5);
        assert_eq!(c, Chunk::uniform(5));
        assert!(!c.has_water());
        let zero_water = vec![0u8; VOXELS];
        assert!(!Chunk::from_dense(&ids, Some(&zero_water)).has_water());
    }

    #[test]
    fn many_ids_round_trip() {
        let mut ids = vec![0u16; VOXELS];
        for (v, id) in ids.iter_mut().enumerate() {
            *id = (v % 1000) as u16;
        }
        let c = Chunk::from_dense(&ids, None);
        assert_eq!(c.bits, 10);
        assert_eq!(c.to_dense(), ids);
        let bytes = c.to_bytes();
        assert_eq!(bytes, Chunk::from_dense(&ids, None).to_bytes());
    }

    #[test]
    fn bytes_round_trip_and_reject_damage() {
        let mut ids = vec![0u16; VOXELS];
        for (v, id) in ids.iter_mut().enumerate() {
            *id = [0u16, 1, 3, 8][(v / 7) % 4];
        }
        let mut water = vec![0u8; VOXELS];
        water[100] = 255;
        for c in [Chunk::from_dense(&ids, Some(&water)), Chunk::from_dense(&ids, None), Chunk::uniform(7)] {
            let back = Chunk::from_bytes(&c.to_bytes()).unwrap();
            assert_eq!(back, c);
        }
        let bytes = Chunk::from_dense(&ids, None).to_bytes();
        assert!(Chunk::from_bytes(&bytes[..bytes.len() - 5]).is_err(), "truncated");
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(Chunk::from_bytes(&extra).is_err(), "trailing bytes");
        let mut bad = bytes.clone();
        bad[3] = 0xFF; // damage the first palette entry's bytes: still decodes (palette is data)...
        assert!(Chunk::from_bytes(&bad).is_ok());
        bad[1] = 1; // ...but a palette length that the indices overrun does not
        bad[2] = 0;
        assert!(Chunk::from_bytes(&bad).is_err(), "indices beyond the palette");
    }
}
