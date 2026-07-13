use crate::WorldIdentity;
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct IdentityError(pub String);

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IdentityError {}

/// Hash only immutable world-defining bakes. Player edits, torches, and
/// multiplayer journals deliberately do not participate in identity.
pub fn immutable_asset_hashes(assets: &Path) -> Result<BTreeMap<String, String>, IdentityError> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(assets)
        .map_err(|e| IdentityError(format!("read assets {}: {e}", assets.display())))?
    {
        let entry = entry
            .map_err(|e| IdentityError(format!("enumerate assets {}: {e}", assets.display())))?;
        let file_type = entry
            .file_type()
            .map_err(|e| IdentityError(format!("inspect asset {}: {e}", entry.path().display())))?;
        if file_type.is_file() && immutable_name(entry.file_name().to_str().unwrap_or("")) {
            files.push(entry.path());
        }
    }
    files.sort();
    for required in [
        "meta.json",
        "face_0.bin",
        "face_1.bin",
        "face_2.bin",
        "face_3.bin",
        "face_4.bin",
        "face_5.bin",
        "rivers.bin",
        "weather.bin",
        "solar_tuning.json",
    ] {
        if !files
            .iter()
            .any(|path| path.file_name().and_then(|value| value.to_str()) == Some(required))
        {
            return Err(IdentityError(format!(
                "{} is missing required immutable asset {required}",
                assets.display()
            )));
        }
    }
    let mut hashes = BTreeMap::new();
    for path in files {
        let name = path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        hashes.insert(name, sha256_file(&path)?);
    }
    Ok(hashes)
}

fn immutable_name(name: &str) -> bool {
    matches!(
        name,
        "meta.json" | "rivers.bin" | "weather.bin" | "solar_tuning.json"
    ) || (name.starts_with("face_") && name.ends_with(".bin"))
}

fn sha256_file(path: &PathBuf) -> Result<String, IdentityError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| IdentityError(format!("open asset {}: {e}", path.display())))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|e| IdentityError(format!("hash asset {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Small streaming SHA-256 implementation kept here so the headless identity
/// boundary has no crypto dependency beyond the specified protocol stack.
/// The algorithm and constants are the FIPS 180-4 compression function.
struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    total_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            block_len: 0,
            total_len: 0,
        }
    }

    fn update(&mut self, mut bytes: &[u8]) {
        self.total_len = self.total_len.wrapping_add(bytes.len() as u64);
        if self.block_len > 0 {
            let take = (64 - self.block_len).min(bytes.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&bytes[..take]);
            self.block_len += take;
            bytes = &bytes[take..];
            if self.block_len == 64 {
                let block = self.block;
                self.compress(&block);
                self.block_len = 0;
            } else {
                return;
            }
        }
        while bytes.len() >= 64 {
            let block: &[u8; 64] = bytes[..64].try_into().unwrap();
            self.compress(block);
            bytes = &bytes[64..];
        }
        self.block[..bytes.len()].copy_from_slice(bytes);
        self.block_len = bytes.len();
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            self.block[self.block_len..].fill(0);
            let block = self.block;
            self.compress(&block);
            self.block = [0; 64];
        } else {
            self.block[self.block_len..56].fill(0);
        }
        self.block[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.block;
        self.compress(&block);
        let mut out = [0u8; 32];
        for (chunk, word) in out.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for (word, bytes) in w[..16].iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_be_bytes(bytes.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (state, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *state = state.wrapping_add(value);
        }
    }
}

pub fn load_world_identity(
    assets: &Path,
    build_hash: impl Into<String>,
) -> Result<WorldIdentity, IdentityError> {
    let meta_path = assets.join("meta.json");
    let raw = std::fs::read_to_string(&meta_path)
        .map_err(|e| IdentityError(format!("read {}: {e}", meta_path.display())))?;
    let meta: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| IdentityError(format!("parse {}: {e}", meta_path.display())))?;
    let seed = meta
        .get("seed")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| IdentityError(format!("{} has no integer seed", meta_path.display())))?;
    Ok(WorldIdentity {
        seed,
        build_hash: build_hash.into(),
        asset_hashes: immutable_asset_hashes(assets)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_standard_vectors() {
        let mut empty = Sha256::new();
        empty.update(b"");
        assert_eq!(
            empty
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let mut abc = Sha256::new();
        abc.update(b"a");
        abc.update(b"bc");
        assert_eq!(
            abc.finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn immutable_identity_requires_and_hashes_the_complete_bake_set() {
        let root = std::env::temp_dir().join(format!(
            "triangulum-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let required = [
            "meta.json",
            "face_0.bin",
            "face_1.bin",
            "face_2.bin",
            "face_3.bin",
            "face_4.bin",
            "face_5.bin",
            "rivers.bin",
            "weather.bin",
            "solar_tuning.json",
        ];
        for name in required {
            std::fs::write(root.join(name), name.as_bytes()).unwrap();
        }
        std::fs::write(root.join("edits_seed42.bin"), b"mutable").unwrap();
        let hashes = immutable_asset_hashes(&root).unwrap();
        assert_eq!(hashes.len(), required.len());
        assert!(!hashes.contains_key("edits_seed42.bin"));
        std::fs::remove_file(root.join("weather.bin")).unwrap();
        assert!(
            immutable_asset_hashes(&root)
                .unwrap_err()
                .to_string()
                .contains("weather.bin")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
