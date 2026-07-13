//! Headless multiplayer contract shared by the Triangulum server, client,
//! and scripted integration client. This crate deliberately has no renderer,
//! window, or platform-event dependencies.

mod clock;
mod identity;
mod invite;
mod journal;
mod protocol;

pub use clock::{AuthoritativeClock, ClockSlew};
pub use identity::{IdentityError, immutable_asset_hashes, load_world_identity};
pub use invite::{Invite, InviteError, parse_invite};
pub use journal::{EditJournal, JournalError, JournalStore, load_legacy_edits, write_legacy_edits};
pub use protocol::*;
