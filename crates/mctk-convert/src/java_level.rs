//! A version-neutral model of a Java world's level settings, with a reader for each storage
//! layout and a writer for the classic one.
//!
//! * Classic (1.18 – 1.21): everything in `level.dat` under `Data`; game rules are strings
//!   named like `doMobSpawning`.
//! * Modern (26.x): `level.dat` keeps the basics and the rest lives in
//!   `data/minecraft/*.dat` (`world_gen_settings`, `game_rules`, `weather`, `world_clocks`);
//!   game rules are typed and named like `minecraft:spawn_mobs`.
//!
//! Game rule names and the two inverted rules were taken from real worlds of both versions.

use std::path::Path;

use indexmap::IndexMap;
use mctk_nbt::{Compound, Flavor, NamedTag, Tag};

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleValue {
    Bool(bool),
    Int(i32),
}

/// The world border. Classic worlds keep one in `level.dat`; modern worlds one per dimension.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldBorder {
    pub center_x: f64,
    pub center_z: f64,
    pub size: f64,
    pub safe_zone: f64,
    pub damage_per_block: f64,
    pub warning_blocks: i32,
    pub warning_time: i32,
    pub lerp_target: f64,
    pub lerp_time: i64,
}

/// What persists of the End's dragon fight between sessions.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DragonFight {
    pub gateways: Vec<i32>,
    pub killed: bool,
    pub previously_killed: bool,
    pub needs_state_scanning: bool,
}

fn dbl(c: &Compound, k: &str, default: f64) -> f64 {
    c.get(k).and_then(Tag::as_f64).unwrap_or(default)
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelData {
    pub name: String,
    pub game_type: i32,
    /// 0 peaceful … 3 hard.
    pub difficulty: i8,
    pub difficulty_locked: bool,
    pub hardcore: bool,
    pub allow_commands: bool,
    pub initialized: bool,
    pub was_modded: bool,
    pub spawn: [i32; 3],
    pub spawn_angle: f32,
    pub time: i64,
    pub day_time: i64,
    pub raining: bool,
    pub rain_time: i32,
    pub thundering: bool,
    pub thunder_time: i32,
    pub clear_weather_time: i32,
    pub last_played: i64,
    pub seed: i64,
    pub generate_structures: bool,
    pub bonus_chest: bool,
    /// The per-dimension generator settings, the same shape in both layouts.
    pub dimensions: Compound,
    /// Keyed by the modern id (`minecraft:spawn_mobs`), values already un-inverted.
    pub game_rules: IndexMap<String, RuleValue>,
    pub server_brands: Vec<String>,
    /// The host player's UUID: `singleplayer_uuid` (26.x) or the `Player` in `level.dat`.
    pub singleplayer_uuid: Option<[i32; 4]>,
    /// The classic layout keeps the host player inside `level.dat` (`Data.Player`).
    pub embedded_player: Option<Compound>,
    pub border: Option<WorldBorder>,
    /// Custom boss bars by id, with their names as NBT text (the modern form).
    pub boss_events: Compound,
    pub dragon_fight: Option<DragonFight>,
    /// Wandering trader (spawn delay, spawn chance).
    pub wandering_trader: Option<(i32, i32)>,
}

impl Default for LevelData {
    fn default() -> Self {
        LevelData {
            name: "World".into(),
            game_type: 0,
            difficulty: 2,
            difficulty_locked: false,
            hardcore: false,
            allow_commands: false,
            initialized: true,
            was_modded: false,
            spawn: [0, 64, 0],
            spawn_angle: 0.0,
            time: 0,
            day_time: 0,
            raining: false,
            rain_time: 0,
            thundering: false,
            thunder_time: 0,
            clear_weather_time: 0,
            last_played: 0,
            seed: 0,
            generate_structures: true,
            bonus_chest: false,
            dimensions: Compound::new(),
            game_rules: IndexMap::new(),
            server_brands: vec!["vanilla".into()],
            singleplayer_uuid: None,
            embedded_player: None,
            border: None,
            boss_events: Compound::new(),
            dragon_fight: None,
            wandering_trader: None,
        }
    }
}

/// The generators every vanilla world has, in the form of 1.18 and later.
fn default_dimensions() -> Compound {
    let source = |kind: &str, preset: Option<&str>| {
        let mut c = Compound::from([("type".to_string(), Tag::String(format!("minecraft:{kind}")))]);
        if let Some(p) = preset {
            c.insert("preset".into(), Tag::String(format!("minecraft:{p}")));
        }
        c
    };
    let dim = |name: &str, settings: &str, biome_source: Compound| {
        (
            format!("minecraft:{name}"),
            Tag::Compound(Compound::from([
                ("type".to_string(), Tag::String(format!("minecraft:{name}"))),
                (
                    "generator".to_string(),
                    Tag::Compound(Compound::from([
                        ("type".to_string(), Tag::String("minecraft:noise".into())),
                        ("settings".to_string(), Tag::String(format!("minecraft:{settings}"))),
                        ("biome_source".to_string(), Tag::Compound(biome_source)),
                    ])),
                ),
            ])),
        )
    };
    Compound::from([
        dim("overworld", "overworld", source("multi_noise", Some("overworld"))),
        dim("the_nether", "nether", source("multi_noise", Some("nether"))),
        dim("the_end", "end", source("the_end", None)),
    ])
}

/// Boss bar names are JSON text in the classic layout and NBT text in the modern one.
fn boss_events_text(events: &Compound, to_nbt: bool) -> Compound {
    events
        .iter()
        .map(|(id, e)| {
            let mut e = e.clone();
            if let Tag::Compound(c) = &mut e {
                if let Some(name) = c.get("Name").cloned() {
                    let converted = match (&name, to_nbt) {
                        (Tag::String(j), true) => crate::java_text::json_to_nbt(j),
                        (other, false) => Tag::String(crate::java_text::nbt_to_json(other)),
                        (other, true) => other.clone(),
                    };
                    c.insert("Name".into(), converted);
                }
            }
            (id.clone(), e)
        })
        .collect()
}

fn uuid_of(c: &Compound, key: &str) -> Option<[i32; 4]> {
    match c.get(key) {
        Some(Tag::IntArray(u)) => <[i32; 4]>::try_from(u.as_slice()).ok(),
        _ => None,
    }
}

/// (modern id, classic name, is a number, inverted). Rules that exist in only one of the two
/// layouts are left out and keep their defaults on the other side.
const RULES: &[(&str, &str, bool, bool)] = &[
    ("minecraft:spawn_wandering_traders", "doTraderSpawning", false, false),
    ("minecraft:block_drops", "doTileDrops", false, false),
    ("minecraft:reduced_debug_info", "reducedDebugInfo", false, false),
    ("minecraft:show_death_messages", "showDeathMessages", false, false),
    ("minecraft:immediate_respawn", "doImmediateRespawn", false, false),
    ("minecraft:block_explosion_drop_decay", "blockExplosionDropDecay", false, false),
    ("minecraft:spread_vines", "doVinesSpread", false, false),
    ("minecraft:max_entity_cramming", "maxEntityCramming", true, false),
    ("minecraft:forgive_dead_players", "forgiveDeadPlayers", false, false),
    ("minecraft:fall_damage", "fallDamage", false, false),
    ("minecraft:send_command_feedback", "sendCommandFeedback", false, false),
    ("minecraft:global_sound_events", "globalSoundEvents", false, false),
    ("minecraft:elytra_movement_check", "disableElytraMovementCheck", false, true),
    ("minecraft:freeze_damage", "freezeDamage", false, false),
    ("minecraft:mob_explosion_drop_decay", "mobExplosionDropDecay", false, false),
    ("minecraft:natural_health_regeneration", "naturalRegeneration", false, false),
    ("minecraft:mob_drops", "doMobLoot", false, false),
    ("minecraft:log_admin_commands", "logAdminCommands", false, false),
    ("minecraft:mob_griefing", "mobGriefing", false, false),
    ("minecraft:spawn_mobs", "doMobSpawning", false, false),
    ("minecraft:spectators_generate_chunks", "spectatorsGenerateChunks", false, false),
    ("minecraft:players_sleeping_percentage", "playersSleepingPercentage", true, false),
    ("minecraft:max_command_sequence_length", "maxCommandChainLength", true, false),
    ("minecraft:advance_weather", "doWeatherCycle", false, false),
    ("minecraft:max_block_modifications", "commandModificationBlockLimit", true, false),
    ("minecraft:drowning_damage", "drowningDamage", false, false),
    ("minecraft:show_advancement_messages", "announceAdvancements", false, false),
    ("minecraft:command_block_output", "commandBlockOutput", false, false),
    ("minecraft:respawn_radius", "spawnRadius", true, false),
    ("minecraft:raids", "disableRaids", false, true),
    ("minecraft:spawn_phantoms", "doInsomnia", false, false),
    ("minecraft:max_snow_accumulation_height", "snowAccumulationHeight", true, false),
    ("minecraft:limited_crafting", "doLimitedCrafting", false, false),
    ("minecraft:lava_source_conversion", "lavaSourceConversion", false, false),
    ("minecraft:tnt_explosion_drop_decay", "tntExplosionDropDecay", false, false),
    ("minecraft:universal_anger", "universalAnger", false, false),
    ("minecraft:keep_inventory", "keepInventory", false, false),
    ("minecraft:spawn_patrols", "doPatrolSpawning", false, false),
    ("minecraft:random_tick_speed", "randomTickSpeed", true, false),
    ("minecraft:fire_damage", "fireDamage", false, false),
    ("minecraft:entity_drops", "doEntityDrops", false, false),
    ("minecraft:advance_time", "doDaylightCycle", false, false),
    ("minecraft:spawn_wardens", "doWardenSpawning", false, false),
    ("minecraft:water_source_conversion", "waterSourceConversion", false, false),
];

const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];

fn flag(c: &Compound, key: &str, default: bool) -> bool {
    c.get(key).and_then(Tag::as_i64).map_or(default, |v| v != 0)
}

fn int(c: &Compound, key: &str, default: i64) -> i64 {
    c.get(key).and_then(Tag::as_i64).unwrap_or(default)
}

fn byte(v: bool) -> Tag {
    Tag::Byte(v as i8)
}

/// Reads a (gzip) NBT file and returns its `data` or `Data` compound; `None` if missing.
pub(crate) fn read_data(path: &Path, key: &str) -> Result<Option<Compound>, Error> {
    if !path.is_file() {
        return Ok(None);
    }
    let (_, root) = mctk_nbt::read_file(path, Flavor::Java)?;
    Ok(root.tag.get(key).and_then(Tag::as_compound).cloned())
}

impl LevelData {
    /// Reads the modern (26.x) layout from a world folder.
    pub fn read_modern(world: &Path) -> Result<Self, Error> {
        let data = read_data(&world.join("level.dat"), "Data")?
            .ok_or_else(|| Error::Invalid("level.dat has no Data".into()))?;
        let mut out = LevelData::default();
        let get_str = |k: &str| data.get(k).and_then(Tag::as_str).map(str::to_string);
        out.name = get_str("LevelName").unwrap_or(out.name);
        out.game_type = int(&data, "GameType", 0) as i32;
        out.allow_commands = flag(&data, "allowCommands", false);
        out.initialized = flag(&data, "initialized", true);
        out.was_modded = flag(&data, "WasModded", false);
        out.last_played = int(&data, "LastPlayed", 0);
        out.time = int(&data, "Time", 0);
        if let Some(Tag::List(l)) = data.get("ServerBrands") {
            out.server_brands = l.iter().filter_map(|t| t.as_str().map(str::to_string)).collect();
        }
        out.singleplayer_uuid = uuid_of(&data, "singleplayer_uuid");
        if let Some(d) = data.get("difficulty_settings").and_then(Tag::as_compound) {
            let name = d.get("difficulty").and_then(Tag::as_str).unwrap_or("normal");
            out.difficulty = DIFFICULTIES.iter().position(|n| *n == name).unwrap_or(2) as i8;
            out.hardcore = flag(d, "hardcore", false);
            out.difficulty_locked = flag(d, "locked", false);
        }
        if let Some(s) = data.get("spawn").and_then(Tag::as_compound) {
            if let Some(Tag::IntArray(p)) = s.get("pos") {
                if p.len() == 3 {
                    out.spawn = [p[0], p[1], p[2]];
                }
            }
            out.spawn_angle = s.get("yaw").and_then(Tag::as_f64).unwrap_or(0.0) as f32;
        }

        let dat = |name: &str| read_data(&world.join("data/minecraft").join(name), "data");
        if let Some(g) = dat("world_gen_settings.dat")? {
            out.seed = int(&g, "seed", 0);
            out.generate_structures = flag(&g, "generate_structures", true);
            out.bonus_chest = flag(&g, "bonus_chest", false);
            if let Some(d) = g.get("dimensions").and_then(Tag::as_compound) {
                out.dimensions = d.clone();
            }
        }
        if let Some(w) = dat("weather.dat")? {
            out.raining = flag(&w, "raining", false);
            out.thundering = flag(&w, "thundering", false);
            out.rain_time = int(&w, "rain_time", 0) as i32;
            out.thunder_time = int(&w, "thunder_time", 0) as i32;
            out.clear_weather_time = int(&w, "clear_weather_time", 0) as i32;
        }
        out.day_time = out.time;
        if let Some(c) = dat("world_clocks.dat")? {
            if let Some(t) = c
                .get("minecraft:overworld")
                .and_then(Tag::as_compound)
                .and_then(|o| o.get("total_ticks"))
                .and_then(Tag::as_i64)
            {
                out.day_time = t;
            }
        }
        if let Some(r) = dat("game_rules.dat")? {
            for (modern, _, number, _) in RULES {
                if let Some(v) = r.get(*modern).and_then(Tag::as_i64) {
                    let value = if *number { RuleValue::Int(v as i32) } else { RuleValue::Bool(v != 0) };
                    out.game_rules.insert((*modern).to_string(), value);
                }
            }
        }
        // Data that classic worlds keep inside level.dat has files of its own here.
        let dim_data = |dim: &str, file: &str| {
            read_data(&world.join("dimensions/minecraft").join(dim).join("data/minecraft").join(file), "data")
        };
        if let Some(b) = dim_data("overworld", "world_border.dat")? {
            let size = dbl(&b, "size", 59999968.0);
            out.border = Some(WorldBorder {
                center_x: dbl(&b, "center_x", 0.0),
                center_z: dbl(&b, "center_z", 0.0),
                size,
                safe_zone: dbl(&b, "safe_zone", 5.0),
                damage_per_block: dbl(&b, "damage_per_block", 0.2),
                warning_blocks: int(&b, "warning_blocks", 5) as i32,
                warning_time: int(&b, "warning_time", 15) as i32,
                lerp_target: dbl(&b, "lerp_target", size),
                lerp_time: int(&b, "lerp_time", 0),
            });
        }
        if let Some(f) = dim_data("the_end", "ender_dragon_fight.dat")? {
            out.dragon_fight = Some(DragonFight {
                gateways: match f.get("gateways") {
                    Some(Tag::IntArray(g)) => g.clone(),
                    _ => Vec::new(),
                },
                killed: flag(&f, "dragon_killed", false),
                previously_killed: flag(&f, "previously_killed", false),
                needs_state_scanning: flag(&f, "needs_state_scanning", false),
            });
        }
        if let Some(events) = dat("custom_boss_events.dat")? {
            out.boss_events = events;
        }
        if let Some(t) = dat("wandering_trader.dat")? {
            out.wandering_trader = Some((int(&t, "spawn_delay", 24000) as i32, int(&t, "spawn_chance", 25) as i32));
        }
        Ok(out)
    }

    /// Reads the classic (1.18 – 1.21) `level.dat`.
    pub fn read_classic(level_dat: &NamedTag) -> Result<Self, Error> {
        let data = level_dat
            .tag
            .get("Data")
            .and_then(Tag::as_compound)
            .ok_or_else(|| Error::Invalid("level.dat has no Data".into()))?;
        let mut out = LevelData::default();
        out.name = data.get("LevelName").and_then(Tag::as_str).unwrap_or("World").to_string();
        out.game_type = int(data, "GameType", 0) as i32;
        out.difficulty = int(data, "Difficulty", 2) as i8;
        out.difficulty_locked = flag(data, "DifficultyLocked", false);
        out.hardcore = flag(data, "hardcore", false);
        out.allow_commands = flag(data, "allowCommands", false);
        out.initialized = flag(data, "initialized", true);
        out.was_modded = flag(data, "WasModded", false);
        out.spawn = [
            int(data, "SpawnX", 0) as i32,
            int(data, "SpawnY", 64) as i32,
            int(data, "SpawnZ", 0) as i32,
        ];
        out.spawn_angle = data.get("SpawnAngle").and_then(Tag::as_f64).unwrap_or(0.0) as f32;
        out.time = int(data, "Time", 0);
        out.day_time = int(data, "DayTime", out.time);
        out.raining = flag(data, "raining", false);
        out.rain_time = int(data, "rainTime", 0) as i32;
        out.thundering = flag(data, "thundering", false);
        out.thunder_time = int(data, "thunderTime", 0) as i32;
        out.clear_weather_time = int(data, "clearWeatherTime", 0) as i32;
        out.last_played = int(data, "LastPlayed", 0);
        if let Some(Tag::List(l)) = data.get("ServerBrands") {
            out.server_brands = l.iter().filter_map(|t| t.as_str().map(str::to_string)).collect();
        }
        if let Some(p) = data.get("Player").and_then(Tag::as_compound) {
            out.singleplayer_uuid = uuid_of(p, "UUID");
            out.embedded_player = Some(p.clone());
        }
        if data.contains_key("BorderSize") {
            let size = dbl(data, "BorderSize", 59999968.0);
            out.border = Some(WorldBorder {
                center_x: dbl(data, "BorderCenterX", 0.0),
                center_z: dbl(data, "BorderCenterZ", 0.0),
                size,
                safe_zone: dbl(data, "BorderSafeZone", 5.0),
                damage_per_block: dbl(data, "BorderDamagePerBlock", 0.2),
                warning_blocks: dbl(data, "BorderWarningBlocks", 5.0) as i32,
                warning_time: dbl(data, "BorderWarningTime", 15.0) as i32,
                lerp_target: dbl(data, "BorderSizeLerpTarget", size),
                lerp_time: int(data, "BorderSizeLerpTime", 0),
            });
        }
        if let Some(Tag::Compound(events)) = data.get("CustomBossEvents") {
            out.boss_events = boss_events_text(events, true);
        }
        if let Some(f) = data.get("DragonFight").and_then(Tag::as_compound) {
            out.dragon_fight = Some(DragonFight {
                gateways: match f.get("Gateways") {
                    Some(Tag::IntArray(g)) => g.clone(),
                    _ => Vec::new(),
                },
                killed: flag(f, "DragonKilled", false),
                previously_killed: flag(f, "PreviouslyKilled", false),
                needs_state_scanning: flag(f, "NeedsStateScanning", false),
            });
        }
        if data.contains_key("WanderingTraderSpawnDelay") || data.contains_key("WanderingTraderSpawnChance") {
            out.wandering_trader = Some((int(data, "WanderingTraderSpawnDelay", 24000) as i32, int(data, "WanderingTraderSpawnChance", 25) as i32));
        }
        if let Some(g) = data.get("WorldGenSettings").and_then(Tag::as_compound) {
            out.seed = int(g, "seed", 0);
            out.generate_structures = flag(g, "generate_features", true);
            out.bonus_chest = flag(g, "bonus_chest", false);
            if let Some(d) = g.get("dimensions").and_then(Tag::as_compound) {
                out.dimensions = d.clone();
            }
        }
        // Before 1.16 the seed and the structures switch are loose keys.
        if !data.contains_key("WorldGenSettings") {
            out.seed = int(data, "RandomSeed", 0);
            out.generate_structures = flag(data, "MapFeatures", true);
        }
        // The generators of 1.16 and 1.17 carry the seed and a different biome source; those, and
        // worlds from before 1.16 that have none, get the default generators of the newer versions.
        let old_shape = out.dimensions.is_empty()
            || out.dimensions.values().any(|d| {
                matches!(
                    d.get("generator").and_then(|g| g.get("biome_source")).and_then(|b| b.get("type")),
                    Some(Tag::String(t)) if t == "minecraft:vanilla_layered" || t == "minecraft:fixed"
                )
            });
        if old_shape {
            out.dimensions = default_dimensions();
        }
        // The seed is written where each version wants it (see `to_classic`).
        for (_, dim) in out.dimensions.iter_mut() {
            if let Some(Tag::Compound(generator)) = dim.as_compound_mut().and_then(|d| d.get_mut("generator")) {
                generator.shift_remove("seed");
                if let Some(Tag::Compound(source)) = generator.get_mut("biome_source") {
                    source.shift_remove("seed");
                }
            }
        }
        if let Some(r) = data.get("GameRules").and_then(Tag::as_compound) {
            for (modern, classic, number, invert) in RULES {
                let Some(text) = r.get(*classic).and_then(Tag::as_str) else { continue };
                let value = if *number {
                    match text.parse() {
                        Ok(n) => RuleValue::Int(n),
                        Err(_) => continue,
                    }
                } else {
                    RuleValue::Bool((text == "true") != *invert)
                };
                out.game_rules.insert((*modern).to_string(), value);
            }
        }
        Ok(out)
    }

    /// Builds a classic `level.dat` for the version with the given `DataVersion` and name.
    pub fn to_classic(&self, data_version: i32, version_name: &str) -> NamedTag {
        let mut d = Compound::new();
        d.insert("LevelName".into(), Tag::String(self.name.clone()));
        d.insert("GameType".into(), Tag::Int(self.game_type));
        d.insert("Difficulty".into(), Tag::Byte(self.difficulty));
        d.insert("DifficultyLocked".into(), byte(self.difficulty_locked));
        d.insert("hardcore".into(), byte(self.hardcore));
        d.insert("allowCommands".into(), byte(self.allow_commands));
        d.insert("initialized".into(), byte(self.initialized));
        d.insert("WasModded".into(), byte(self.was_modded));
        d.insert("SpawnX".into(), Tag::Int(self.spawn[0]));
        d.insert("SpawnY".into(), Tag::Int(self.spawn[1]));
        d.insert("SpawnZ".into(), Tag::Int(self.spawn[2]));
        d.insert("SpawnAngle".into(), Tag::Float(self.spawn_angle));
        d.insert("Time".into(), Tag::Long(self.time));
        d.insert("DayTime".into(), Tag::Long(self.day_time));
        d.insert("raining".into(), byte(self.raining));
        d.insert("rainTime".into(), Tag::Int(self.rain_time));
        d.insert("thundering".into(), byte(self.thundering));
        d.insert("thunderTime".into(), Tag::Int(self.thunder_time));
        d.insert("clearWeatherTime".into(), Tag::Int(self.clear_weather_time));
        d.insert("LastPlayed".into(), Tag::Long(self.last_played));
        d.insert("DataVersion".into(), Tag::Int(data_version));
        d.insert("version".into(), Tag::Int(19133));
        d.insert(
            "Version".into(),
            Tag::Compound(Compound::from([
                ("Id".to_string(), Tag::Int(data_version)),
                ("Name".to_string(), Tag::String(version_name.into())),
                ("Series".to_string(), Tag::String("main".into())),
                ("Snapshot".to_string(), Tag::Byte(0)),
            ])),
        );
        let brands = self.server_brands.iter().map(|b| Tag::String(b.clone())).collect();
        d.insert(
            "ServerBrands".into(),
            Tag::List(mctk_nbt::List::from_tags(mctk_nbt::TagType::String, brands).expect("strings")),
        );

        if data_version >= crate::java_old::FIRST_NEW_LAYOUT {
            let mut gen_settings = Compound::new();
            gen_settings.insert("seed".into(), Tag::Long(self.seed));
            gen_settings.insert("generate_features".into(), byte(self.generate_structures));
            gen_settings.insert("bonus_chest".into(), byte(self.bonus_chest));
            let mut dimensions = self.dimensions.clone();
            // Up to 1.18.2 every generator (and the End's biome source) carries the world seed.
            if data_version < 3105 {
                for (_, dim) in dimensions.iter_mut() {
                    let Tag::Compound(dim) = dim else { continue };
                    let Some(Tag::Compound(gen_)) = dim.get_mut("generator") else { continue };
                    if matches!(gen_.get("type"), Some(Tag::String(t)) if t == "minecraft:noise") {
                        gen_.insert("seed".into(), Tag::Long(self.seed));
                    }
                    if let Some(Tag::Compound(source)) = gen_.get_mut("biome_source") {
                        if matches!(source.get("type"), Some(Tag::String(t)) if t == "minecraft:the_end") {
                            source.insert("seed".into(), Tag::Long(self.seed));
                        }
                    }
                }
            }
            gen_settings.insert("dimensions".into(), Tag::Compound(dimensions));
            d.insert("WorldGenSettings".into(), Tag::Compound(gen_settings));
        } else if data_version >= crate::java_old::FIRST_WORLD_GEN_SETTINGS {
            // 1.16 and 1.17.
            let settings = crate::java_old::world_gen_settings(self.seed, self.generate_structures, self.bonus_chest);
            d.insert("WorldGenSettings".into(), Tag::Compound(settings));
        } else {
            // 1.13 - 1.15 keep the seed and the generator beside the other settings.
            d.insert("RandomSeed".into(), Tag::Long(self.seed));
            d.insert("generatorName".into(), Tag::String("default".into()));
            d.insert("generatorVersion".into(), Tag::Int(1));
            d.insert("MapFeatures".into(), byte(self.generate_structures));
        }
        if let Some(p) = &self.embedded_player {
            d.insert("Player".into(), Tag::Compound(p.clone()));
        }

        // The game logs an error when this is missing. A fresh entry makes it re-scan the
        // End for the dragon's state.
        let fight = self.dragon_fight.clone().unwrap_or_else(|| DragonFight {
            gateways: (0..20).collect(),
            needs_state_scanning: true,
            ..Default::default()
        });
        // (Before 1.16 the fight is kept per dimension; the game starts a new one.)
        if data_version >= crate::java_old::FIRST_WORLD_GEN_SETTINGS {
            let mut f = Compound::new();
            if data_version >= crate::java_old::FIRST_ENTITY_FILES {
                f.insert("NeedsStateScanning".into(), byte(fight.needs_state_scanning));
            }
            f.insert("DragonKilled".into(), byte(fight.killed));
            f.insert("PreviouslyKilled".into(), byte(fight.previously_killed));
            f.insert("Gateways".into(), Tag::IntArray(fight.gateways));
            d.insert("DragonFight".into(), Tag::Compound(f));
        }
        if let Some(b) = &self.border {
            for (k, v) in [
                ("BorderCenterX", Tag::Double(b.center_x)),
                ("BorderCenterZ", Tag::Double(b.center_z)),
                ("BorderSize", Tag::Double(b.size)),
                ("BorderSafeZone", Tag::Double(b.safe_zone)),
                ("BorderDamagePerBlock", Tag::Double(b.damage_per_block)),
                ("BorderWarningBlocks", Tag::Double(b.warning_blocks as f64)),
                ("BorderWarningTime", Tag::Double(b.warning_time as f64)),
                ("BorderSizeLerpTarget", Tag::Double(b.lerp_target)),
                ("BorderSizeLerpTime", Tag::Long(b.lerp_time)),
            ] {
                d.insert(k.into(), v);
            }
        }
        if !self.boss_events.is_empty() {
            d.insert("CustomBossEvents".into(), Tag::Compound(boss_events_text(&self.boss_events, false)));
        }
        if let Some((delay, chance)) = self.wandering_trader {
            d.insert("WanderingTraderSpawnDelay".into(), Tag::Int(delay));
            d.insert("WanderingTraderSpawnChance".into(), Tag::Int(chance));
        }

        if data_version <= crate::java_numeric::LAST_NUMERIC {
            // 1.8 to 1.12: no `Version` before 1.9 and no DataVersion before 1.9.
            d.insert("SizeOnDisk".into(), Tag::Long(0));
            d.insert("generatorOptions".into(), Tag::String(String::new()));
            if data_version == 0 {
                d.shift_remove("DataVersion");
                d.shift_remove("Version");
            }
            for key in ["ServerBrands", "WanderingTraderSpawnDelay", "WanderingTraderSpawnChance"] {
                d.shift_remove(key);
            }
        }
        let mut rules = Compound::new();
        for (modern, classic, number, invert) in RULES {
            let Some(v) = self.game_rules.get(*modern) else { continue };
            let text = match (*v, *number) {
                (RuleValue::Int(n), true) => n.to_string(),
                (RuleValue::Bool(b), false) => (b != *invert).to_string(),
                _ => continue,
            };
            rules.insert((*classic).to_string(), Tag::String(text));
        }
        d.insert("GameRules".into(), Tag::Compound(rules));

        NamedTag {
            name: String::new(),
            tag: Tag::Compound(Compound::from([("Data".to_string(), Tag::Compound(d))])),
        }
    }
}

/// Writes `{data: ..., DataVersion}` as a gzip NBT file, creating its folder.
pub(crate) fn write_data_file(path: &Path, data: Compound, data_version: i32) -> Result<(), Error> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let root = NamedTag {
        name: String::new(),
        tag: Tag::Compound(Compound::from([
            ("data".to_string(), Tag::Compound(data)),
            ("DataVersion".to_string(), Tag::Int(data_version)),
        ])),
    };
    let raw = mctk_nbt::to_bytes(&root, Flavor::Java)?;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&raw)?;
    std::fs::write(path, e.finish()?)?;
    Ok(())
}

impl LevelData {
    /// Writes the modern (26.x) layout into the world folder `world`: `level.dat` plus the
    /// `data/minecraft/*.dat` files.
    pub fn write_modern(&self, world: &Path, data_version: i32, version_name: &str) -> Result<(), Error> {
        use std::io::Write;
        let mut d = Compound::new();
        d.insert("LevelName".into(), Tag::String(self.name.clone()));
        d.insert("GameType".into(), Tag::Int(self.game_type));
        d.insert("allowCommands".into(), byte(self.allow_commands));
        d.insert("initialized".into(), byte(self.initialized));
        d.insert("WasModded".into(), byte(self.was_modded));
        d.insert("LastPlayed".into(), Tag::Long(self.last_played));
        d.insert("Time".into(), Tag::Long(self.time));
        d.insert("DataVersion".into(), Tag::Int(data_version));
        d.insert("version".into(), Tag::Int(19133));
        if let Some(u) = self.singleplayer_uuid {
            d.insert("singleplayer_uuid".into(), Tag::IntArray(u.to_vec()));
        }
        d.insert(
            "version_history".into(),
            Tag::List(mctk_nbt::List::from_tags(mctk_nbt::TagType::Int, vec![Tag::Int(data_version)]).expect("ints")),
        );
        d.insert(
            "Version".into(),
            Tag::Compound(Compound::from([
                ("Id".to_string(), Tag::Int(data_version)),
                ("Name".to_string(), Tag::String(version_name.into())),
                ("Series".to_string(), Tag::String("main".into())),
                ("Snapshot".to_string(), Tag::Byte(0)),
            ])),
        );
        let strings = |items: &[String]| {
            let tags = items.iter().map(|s| Tag::String(s.clone())).collect();
            Tag::List(mctk_nbt::List::from_tags(mctk_nbt::TagType::String, tags).expect("strings"))
        };
        d.insert("ServerBrands".into(), strings(&self.server_brands));
        d.insert(
            "DataPacks".into(),
            Tag::Compound(Compound::from([
                ("Enabled".to_string(), strings(&["vanilla".to_string()])),
                ("Disabled".to_string(), strings(&[])),
            ])),
        );
        d.insert(
            "difficulty_settings".into(),
            Tag::Compound(Compound::from([
                (
                    "difficulty".to_string(),
                    Tag::String(DIFFICULTIES[self.difficulty.clamp(0, 3) as usize].into()),
                ),
                ("hardcore".to_string(), byte(self.hardcore)),
                ("locked".to_string(), byte(self.difficulty_locked)),
            ])),
        );
        d.insert(
            "spawn".into(),
            Tag::Compound(Compound::from([
                ("pos".to_string(), Tag::IntArray(self.spawn.to_vec())),
                ("dimension".to_string(), Tag::String("minecraft:overworld".into())),
                ("yaw".to_string(), Tag::Float(self.spawn_angle)),
                ("pitch".to_string(), Tag::Float(0.0)),
            ])),
        );
        let root = NamedTag {
            name: String::new(),
            tag: Tag::Compound(Compound::from([("Data".to_string(), Tag::Compound(d))])),
        };
        let raw = mctk_nbt::to_bytes(&root, Flavor::Java)?;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&raw)?;
        std::fs::create_dir_all(world)?;
        std::fs::write(world.join("level.dat"), e.finish()?)?;

        let file = |name: &str, data: Compound| {
            write_data_file(&world.join("data/minecraft").join(name), data, data_version)
        };
        file(
            "world_gen_settings.dat",
            Compound::from([
                ("seed".to_string(), Tag::Long(self.seed)),
                ("generate_structures".to_string(), byte(self.generate_structures)),
                ("bonus_chest".to_string(), byte(self.bonus_chest)),
                ("dimensions".to_string(), Tag::Compound(self.dimensions.clone())),
            ]),
        )?;
        let mut rules = Compound::new();
        for (modern, _, number, _) in RULES {
            match (self.game_rules.get(*modern), *number) {
                (Some(RuleValue::Int(n)), true) => rules.insert((*modern).to_string(), Tag::Int(*n)),
                (Some(RuleValue::Bool(b)), false) => rules.insert((*modern).to_string(), byte(*b)),
                _ => None,
            };
        }
        file("game_rules.dat", rules)?;
        file(
            "weather.dat",
            Compound::from([
                ("raining".to_string(), byte(self.raining)),
                ("rain_time".to_string(), Tag::Int(self.rain_time)),
                ("thundering".to_string(), byte(self.thundering)),
                ("thunder_time".to_string(), Tag::Int(self.thunder_time)),
                ("clear_weather_time".to_string(), Tag::Int(self.clear_weather_time)),
            ]),
        )?;
        let clock = |ticks: i64| {
            Tag::Compound(Compound::from([("total_ticks".to_string(), Tag::Long(ticks))]))
        };
        file(
            "world_clocks.dat",
            Compound::from([
                ("minecraft:overworld".to_string(), clock(self.day_time)),
                ("minecraft:the_end".to_string(), clock(self.time)),
            ]),
        )?;
        file(
            "scheduled_events.dat",
            Compound::from([("events".to_string(), Tag::List(mctk_nbt::List::new(mctk_nbt::TagType::Compound)))]),
        )?;
        file("custom_boss_events.dat", boss_events_text(&self.boss_events, true))?;
        if let Some((delay, chance)) = self.wandering_trader {
            file(
                "wandering_trader.dat",
                Compound::from([("spawn_delay".to_string(), Tag::Int(delay)), ("spawn_chance".to_string(), Tag::Int(chance))]),
            )?;
        }
        // Per dimension: the world border (the nether's centre is scaled down by 8, as the game
        // does) and the End's dragon fight.
        let dim_file = |dim: &str, name: &str, data: Compound| {
            write_data_file(
                &world.join("dimensions/minecraft").join(dim).join("data/minecraft").join(name),
                data,
                data_version,
            )
        };
        if let Some(b) = &self.border {
            for (dim, scale) in [("overworld", 1.0), ("the_nether", 8.0), ("the_end", 1.0)] {
                dim_file(
                    dim,
                    "world_border.dat",
                    Compound::from([
                        ("center_x".to_string(), Tag::Double(b.center_x / scale)),
                        ("center_z".to_string(), Tag::Double(b.center_z / scale)),
                        ("size".to_string(), Tag::Double(b.size)),
                        ("safe_zone".to_string(), Tag::Double(b.safe_zone)),
                        ("damage_per_block".to_string(), Tag::Double(b.damage_per_block)),
                        ("warning_blocks".to_string(), Tag::Int(b.warning_blocks)),
                        ("warning_time".to_string(), Tag::Int(b.warning_time)),
                        ("lerp_target".to_string(), Tag::Double(b.lerp_target)),
                        ("lerp_time".to_string(), Tag::Long(b.lerp_time)),
                    ]),
                )?;
            }
        }
        if let Some(f) = &self.dragon_fight {
            dim_file(
                "the_end",
                "ender_dragon_fight.dat",
                Compound::from([
                    ("gateways".to_string(), Tag::IntArray(f.gateways.clone())),
                    ("dragon_killed".to_string(), byte(f.killed)),
                    ("previously_killed".to_string(), byte(f.previously_killed)),
                    ("needs_state_scanning".to_string(), byte(f.needs_state_scanning)),
                ]),
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_round_trips() {
        let mut l = LevelData {
            name: "M".into(),
            seed: 99,
            ..LevelData::default()
        };
        l.spawn = [5, 70, -9];
        l.difficulty = 1;
        l.time = 1000;
        l.day_time = 2000;
        l.raining = true;
        l.rain_time = 123;
        l.dimensions.insert("minecraft:the_end".into(), Tag::Compound(Compound::new()));
        l.game_rules.insert("minecraft:raids".into(), RuleValue::Bool(false));
        l.game_rules.insert("minecraft:respawn_radius".into(), RuleValue::Int(4));

        let dir = std::env::temp_dir().join(format!("mctk-level-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        l.write_modern(&dir, 5023, "26.3").unwrap();
        let back = LevelData::read_modern(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(back, l);
    }

    #[test]
    fn classic_round_trips_and_inverts_rules() {
        let mut l = LevelData {
            name: "T".into(),
            seed: -77,
            ..LevelData::default()
        };
        l.spawn = [1, 2, 3];
        l.difficulty = 3;
        l.dimensions.insert("minecraft:overworld".into(), Tag::Compound(Compound::new()));
        l.game_rules.insert("minecraft:raids".into(), RuleValue::Bool(true));
        l.game_rules.insert("minecraft:random_tick_speed".into(), RuleValue::Int(7));
        l.game_rules.insert("minecraft:elytra_movement_check".into(), RuleValue::Bool(false));

        // The classic level.dat always carries a dragon fight; a world without one gets the fresh
        // entry that makes the game re-scan the End.
        l.dragon_fight = Some(DragonFight { gateways: (0..20).collect(), needs_state_scanning: true, ..Default::default() });
        l.border = Some(WorldBorder {
            center_x: 100.5,
            center_z: -3.0,
            size: 500.0,
            safe_zone: 5.0,
            damage_per_block: 0.2,
            warning_blocks: 5,
            warning_time: 15,
            lerp_target: 500.0,
            lerp_time: 0,
        });
        l.wandering_trader = Some((24000, 25));
        l.boss_events.insert("mctk:boss".into(), Tag::Compound(Compound::from([("Max".to_string(), Tag::Int(200)), ("Name".to_string(), Tag::String("Boss".into()))])));
        let nbt = l.to_classic(3465, "1.20.1");
        let rules = nbt.tag.get("Data").unwrap().get("GameRules").unwrap();
        // `raids` is stored inverted as `disableRaids`.
        assert_eq!(rules.get("disableRaids").and_then(Tag::as_str), Some("false"));
        assert_eq!(rules.get("disableElytraMovementCheck").and_then(Tag::as_str), Some("true"));
        assert_eq!(rules.get("randomTickSpeed").and_then(Tag::as_str), Some("7"));

        let back = LevelData::read_classic(&nbt).unwrap();
        assert_eq!(back, l);
    }
}
