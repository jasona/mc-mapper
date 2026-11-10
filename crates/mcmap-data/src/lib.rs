//! Shared vocabulary for MC Mapper: which world, which version, and where the game's files are.
//!
//! Everything here is plain data with no UI or world-generation dependency, so every other crate
//! (including the live-position crate) can use it.

pub mod install;
pub mod java;
pub mod link;
pub mod pack;
mod world;

pub use world::{Dimension, McVersion, Preset, WorldId};
