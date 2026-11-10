//! Live player position. Every source (replay file, F3+C clipboard, the in-game agent) implements
//! [`PositionSource`] and pushes [`Fix`]es into a sink; the map keeps the freshest one. This crate
//! never touches tiles or world generation, so it builds and tests on its own.

pub mod agent;
mod fix;
pub mod protocol;
pub mod replay;
pub mod session;
pub mod clipboard;

pub use fix::{spawn, Fix, FixSink, PositionSource, SourceKind, StopToken, Stopper, WorldHint};
