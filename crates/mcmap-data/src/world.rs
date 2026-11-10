use serde::{Deserialize, Serialize};
use std::fmt;

/// A Java Edition version, as the launcher names it ("26.3", "1.21.4", "26.4-snapshot-7").
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct McVersion(pub String);

impl McVersion {
    pub fn new(name: impl Into<String>) -> Self {
        McVersion(name.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for McVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
}

impl Dimension {
    /// The game's resource key, as F3+C and the agent report it.
    pub fn key(self) -> &'static str {
        match self {
            Dimension::Overworld => "minecraft:overworld",
            Dimension::Nether => "minecraft:the_nether",
            Dimension::End => "minecraft:the_end",
        }
    }

    pub fn from_key(key: &str) -> Option<Dimension> {
        match key {
            "minecraft:overworld" | "overworld" => Some(Dimension::Overworld),
            "minecraft:the_nether" | "the_nether" | "nether" => Some(Dimension::Nether),
            "minecraft:the_end" | "the_end" | "end" => Some(Dimension::End),
            _ => None,
        }
    }
}

/// World presets that change biome generation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Preset {
    #[default]
    Normal,
    LargeBiomes,
    Amplified,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Normal, Preset::LargeBiomes, Preset::Amplified];

    /// The Overworld noise settings the preset uses.
    pub fn settings(self) -> &'static str {
        match self {
            Preset::Normal => "minecraft:overworld",
            Preset::LargeBiomes => "minecraft:large_biomes",
            Preset::Amplified => "minecraft:amplified",
        }
    }

    /// The folder of its terrain density functions (`<dir>/offset`, `<dir>/factor`, ...).
    pub fn functions(self) -> &'static str {
        match self {
            Preset::Normal => "minecraft:overworld",
            Preset::LargeBiomes => "minecraft:overworld_large_biomes",
            Preset::Amplified => "minecraft:overworld_amplified",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Preset::Normal => "Default",
            Preset::LargeBiomes => "Large Biomes",
            Preset::Amplified => "Amplified",
        }
    }
}

/// Keys every cache and can change at runtime: the game switches worlds and dimensions without
/// the user touching the map.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorldId {
    pub seed: i64,
    pub version: McVersion,
    pub dimension: Dimension,
    pub preset: Preset,
}

impl WorldId {
    pub fn new(seed: i64, version: impl Into<String>) -> Self {
        WorldId { seed, version: McVersion::new(version), dimension: Dimension::Overworld, preset: Preset::Normal }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_keys_round_trip() {
        for d in [Dimension::Overworld, Dimension::Nether, Dimension::End] {
            assert_eq!(Dimension::from_key(d.key()), Some(d));
        }
        assert_eq!(Dimension::from_key("minecraft:somewhere"), None);
    }
}
