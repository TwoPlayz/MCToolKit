use mctk_nbt::{NamedTag, Tag};

/// World spawn point.
#[derive(Debug, Clone, PartialEq)]
pub struct Spawn {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub dimension: Option<String>,
}

/// Summary of `level.dat`, plus the full tree for anything not modelled.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelInfo {
    pub name: String,
    pub data_version: Option<i32>,
    pub version_name: Option<String>,
    pub last_played: Option<i64>,
    pub game_type: Option<i32>,
    pub spawn: Option<Spawn>,
    pub root: NamedTag,
}

impl LevelInfo {
    pub fn from_nbt(root: NamedTag) -> Self {
        let data = root.tag.get("Data");
        let int = |key: &str| data.and_then(|d| d.get(key)).and_then(Tag::as_i64);
        let spawn = data.and_then(|d| d.get("spawn")).and_then(|s| {
            // 26.x: spawn: {pos: [I; x, y, z], dimension: "minecraft:overworld", ...}
            match s.get("pos") {
                Some(Tag::IntArray(p)) if p.len() == 3 => Some(Spawn {
                    x: p[0],
                    y: p[1],
                    z: p[2],
                    dimension: s.get("dimension").and_then(Tag::as_str).map(str::to_string),
                }),
                _ => None,
            }
        });
        // Older versions: SpawnX / SpawnY / SpawnZ.
        let spawn = spawn.or_else(|| {
            Some(Spawn {
                x: int("SpawnX")? as i32,
                y: int("SpawnY")? as i32,
                z: int("SpawnZ")? as i32,
                dimension: None,
            })
        });
        LevelInfo {
            name: data
                .and_then(|d| d.get("LevelName"))
                .and_then(Tag::as_str)
                .unwrap_or_default()
                .to_string(),
            data_version: int("DataVersion").map(|v| v as i32),
            version_name: data
                .and_then(|d| d.get("Version"))
                .and_then(|v| v.get("Name"))
                .and_then(Tag::as_str)
                .map(str::to_string),
            last_played: int("LastPlayed"),
            game_type: int("GameType").map(|v| v as i32),
            spawn,
            root,
        }
    }
}
