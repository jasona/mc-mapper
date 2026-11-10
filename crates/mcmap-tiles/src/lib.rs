//! Headless map rendering: the tile pyramid and camera, tile pixels, and the sprite atlases the
//! map draws markers from. Nothing here depends on the UI.

pub mod atlas;
pub mod biomes;
pub mod checker;
pub mod pyramid;
pub mod relief;

pub use pyramid::{Camera, TileKey, TILE};
