//! triangulum-viewer library: cube-sphere LOD rendering over the planetgen
//! dataset. The binary in main.rs is a thin shell over these modules; tests
//! (notably the Python-parity noise goldens) link against the library.

pub mod camera;
pub mod moon;
#[cfg(feature = "multiplayer")]
pub mod net;
pub mod noise;
pub mod noise_grad;
pub mod orbits;
pub mod planet;
pub mod player;
pub mod renderer;
pub mod rivers;
pub mod terrain;
pub mod ui;
pub mod voxel;
pub mod weather;

// The release `cargo test --lib` gate historically targets only this viewer
// package. Compile the exact headless protocol/clock/journal sources into that
// test target as well, without adding the network crate or Tokio to normal
// library/example builds. Their own package tests use the same source files.
#[cfg(test)]
#[path = "../multiplayer/src/protocol.rs"]
mod multiplayer_protocol_contract;
#[cfg(test)]
pub(crate) use multiplayer_protocol_contract::{
    BodyId, ClockEvent, ClockEventKind, ClockState, EditRecord, EditRequest, WorldIdentity,
};
#[cfg(test)]
#[path = "../multiplayer/src/clock.rs"]
mod multiplayer_clock_contract;
#[cfg(test)]
#[path = "../multiplayer/src/identity.rs"]
mod multiplayer_identity_contract;
#[cfg(test)]
#[path = "../multiplayer/src/invite.rs"]
mod multiplayer_invite_contract;
#[cfg(test)]
#[path = "../multiplayer/src/journal.rs"]
mod multiplayer_journal_contract;
