//! The self-contained export packet.
//!
//! The packet is a projection of the manifest, not a copy of it: a reader holding
//! only the packet can resolve every citation with no state store, no network and no
//! second application. What is deliberately absent matters as much as what is
//! carried, so both are decided in `building`.
//!
//! `encoding` holds the packet's own digest and the exact serialization; it is this
//! project's encoding rather than a general canonicalization standard, and the
//! distinction is worth keeping visible.

pub mod base64;
pub mod building;
pub mod encoding;

pub use base64::base64;
pub use building::{build, bytes};
pub use encoding::{DEFAULT_EXPORT_BYTES, FORMAT, PROFILE, packet_bytes, packet_digest};
