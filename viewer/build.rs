//! Stamp the binary with its git commit so every screenshot sidecar and
//! window title records WHICH build produced it. A day of rapid pushes
//! taught us that "which binary took this photo" is the first triage
//! question — now it answers itself.

use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let dirty = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    println!(
        "cargo:rustc-env=TRI_BUILD={}{}",
        hash,
        if dirty { "+dirty" } else { "" }
    );
    // re-stamp when the checked-out commit moves
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");
}
