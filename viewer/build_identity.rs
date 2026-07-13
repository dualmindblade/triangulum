//! Shared build-identity stamp for the viewer and headless server.
//!
//! A bare `commit+dirty` marker is unsafe for multiplayer: two different
//! dirty worktrees at the same commit would claim to be the same build. Hash
//! every compiled in-workspace source/manifest plus the Rust toolchain so the
//! handshake rejects that case too. Runtime world bakes remain separately
//! SHA-256-gated by the protocol identity.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn emit(viewer: &Path, repo: &Path) {
    let commit = Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".into());

    let mut files = Vec::new();
    // Directory watches ensure adding a new source file also restamps; the
    // per-file watches below cover content changes without relying on mtime
    // behavior outside Cargo.
    for relative in ["src", "multiplayer", "server"] {
        println!("cargo:rerun-if-changed={}", viewer.join(relative).display());
    }
    for relative in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "build_identity.rs",
        "src",
        "multiplayer/Cargo.toml",
        "multiplayer/src",
        "server/Cargo.toml",
        "server/build.rs",
        "server/src",
    ] {
        collect(&viewer.join(relative), &mut files);
    }
    files.sort();
    files.dedup();

    // FNV-1a is used only to make accidental source identity collisions
    // vanishingly unlikely; immutable world assets use full SHA-256 at run
    // time. Length framing prevents path/content concatenation ambiguity.
    let mut digest = Fnv1a64::new();
    for path in files {
        let relative = path
            .strip_prefix(viewer)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!("read build identity source {}: {error}", path.display())
        });
        digest.framed(relative.as_bytes());
        digest.framed(&bytes);
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let rustc = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("-vV")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
        .unwrap_or_else(|| b"unknown-rustc".to_vec());
    digest.framed(&rustc);

    println!(
        "cargo:rustc-env=TRI_BUILD={commit}-s{:016x}",
        digest.finish()
    );
    println!(
        "cargo:rerun-if-changed={}",
        repo.join(".git/HEAD").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        repo.join(".git/index").display()
    );
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        out.push(path.to_path_buf());
        return;
    }
    if !path.is_dir() {
        return;
    }
    let mut children = std::fs::read_dir(path)
        .unwrap_or_else(|error| panic!("read build identity directory {}: {error}", path.display()))
        .map(|entry| entry.expect("read build identity directory entry").path())
        .collect::<Vec<_>>();
    children.sort();
    for child in children {
        collect(&child, out);
    }
}

struct Fnv1a64(u64);

impl Fnv1a64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn framed(&mut self, bytes: &[u8]) {
        self.feed(&(bytes.len() as u64).to_le_bytes());
        self.feed(bytes);
    }

    fn finish(self) -> u64 {
        self.0
    }
}
