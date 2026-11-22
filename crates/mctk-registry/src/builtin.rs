//! The registries shipped with MCToolKit, one per Java version we have generated.

use crate::Registry;

/// (version, table). Newest first. Each table is read from the real game's own data reports (see
/// `mctk-gen`). Only versions with a known data version can be a target.
const JAVA: &[(&str, &str)] = &[
    ("26.3", include_str!("../data/java/26.3.tsv")),
    ("26.2", include_str!("../data/java/26.2.tsv")),
    ("26.1.2", include_str!("../data/java/26.1.2.tsv")),
    ("26.1.1", include_str!("../data/java/26.1.1.tsv")),
    ("26.1", include_str!("../data/java/26.1.tsv")),
    ("1.21.11", include_str!("../data/java/1.21.11.tsv")),
    ("1.21.10", include_str!("../data/java/1.21.10.tsv")),
    ("1.21.9", include_str!("../data/java/1.21.9.tsv")),
    ("1.21.8", include_str!("../data/java/1.21.8.tsv")),
    ("1.21.7", include_str!("../data/java/1.21.7.tsv")),
    ("1.21.6", include_str!("../data/java/1.21.6.tsv")),
    ("1.21.5", include_str!("../data/java/1.21.5.tsv")),
    ("1.21.4", include_str!("../data/java/1.21.4.tsv")),
    ("1.21.3", include_str!("../data/java/1.21.3.tsv")),
    ("1.21.2", include_str!("../data/java/1.21.2.tsv")),
    ("1.21.1", include_str!("../data/java/1.21.1.tsv")),
    ("1.21", include_str!("../data/java/1.21.tsv")),
    ("1.20.6", include_str!("../data/java/1.20.6.tsv")),
    ("1.20.5", include_str!("../data/java/1.20.5.tsv")),
    ("1.20.4", include_str!("../data/java/1.20.4.tsv")),
    ("1.20.3", include_str!("../data/java/1.20.3.tsv")),
    ("1.20.2", include_str!("../data/java/1.20.2.tsv")),
    ("1.20.1", include_str!("../data/java/1.20.1.tsv")),
    ("1.20", include_str!("../data/java/1.20.tsv")),
    ("1.19.4", include_str!("../data/java/1.19.4.tsv")),
    ("1.19.3", include_str!("../data/java/1.19.3.tsv")),
    ("1.19.2", include_str!("../data/java/1.19.2.tsv")),
    ("1.19.1", include_str!("../data/java/1.19.1.tsv")),
    ("1.19", include_str!("../data/java/1.19.tsv")),
    ("1.18.2", include_str!("../data/java/1.18.2.tsv")),
    ("1.18.1", include_str!("../data/java/1.18.1.tsv")),
    ("1.18", include_str!("../data/java/1.18.tsv")),
    ("1.17.1", include_str!("../data/java/1.17.1.tsv")),
    ("1.17", include_str!("../data/java/1.17.tsv")),
    ("1.16.5", include_str!("../data/java/1.16.5.tsv")),
    ("1.16.4", include_str!("../data/java/1.16.4.tsv")),
    ("1.16.3", include_str!("../data/java/1.16.3.tsv")),
    ("1.16.2", include_str!("../data/java/1.16.2.tsv")),
    ("1.16.1", include_str!("../data/java/1.16.1.tsv")),
    ("1.16", include_str!("../data/java/1.16.tsv")),
    ("1.15.2", include_str!("../data/java/1.15.2.tsv")),
    ("1.15.1", include_str!("../data/java/1.15.1.tsv")),
    ("1.15", include_str!("../data/java/1.15.tsv")),
    ("1.14.4", include_str!("../data/java/1.14.4.tsv")),
    ("1.14.3", include_str!("../data/java/1.14.3.tsv")),
    ("1.14.2", include_str!("../data/java/1.14.2.tsv")),
    ("1.14.1", include_str!("../data/java/1.14.1.tsv")),
    ("1.14", include_str!("../data/java/1.14.tsv")),
    ("1.13.2", include_str!("../data/java/1.13.2.tsv")),
    ("1.13.1", include_str!("../data/java/1.13.1.tsv")),
    ("1.13", include_str!("../data/java/1.13.tsv")),
    ("1.12.2", include_str!("../data/java/1.12.2.tsv")),
    ("1.12.1", include_str!("../data/java/1.12.1.tsv")),
    ("1.12", include_str!("../data/java/1.12.tsv")),
    ("1.11.2", include_str!("../data/java/1.11.2.tsv")),
    ("1.11.1", include_str!("../data/java/1.11.1.tsv")),
    ("1.11", include_str!("../data/java/1.11.tsv")),
    ("1.10.2", include_str!("../data/java/1.10.2.tsv")),
    ("1.10.1", include_str!("../data/java/1.10.1.tsv")),
    ("1.10", include_str!("../data/java/1.10.tsv")),
    ("1.9.4", include_str!("../data/java/1.9.4.tsv")),
    ("1.9.3", include_str!("../data/java/1.9.3.tsv")),
    ("1.9.2", include_str!("../data/java/1.9.2.tsv")),
    ("1.9.1", include_str!("../data/java/1.9.1.tsv")),
    ("1.9", include_str!("../data/java/1.9.tsv")),
    ("1.8.9", include_str!("../data/java/1.8.9.tsv")),
    ("1.8.8", include_str!("../data/java/1.8.8.tsv")),
    ("1.8.7", include_str!("../data/java/1.8.7.tsv")),
    ("1.8.6", include_str!("../data/java/1.8.6.tsv")),
    ("1.8.5", include_str!("../data/java/1.8.5.tsv")),
    ("1.8.4", include_str!("../data/java/1.8.4.tsv")),
    ("1.8.3", include_str!("../data/java/1.8.3.tsv")),
    ("1.8.2", include_str!("../data/java/1.8.2.tsv")),
    ("1.8.1", include_str!("../data/java/1.8.1.tsv")),
    ("1.8", include_str!("../data/java/1.8.tsv")),
];

/// The Java versions a world can be converted to, newest first.
pub fn java_versions() -> Vec<&'static str> {
    JAVA.iter().map(|(v, _)| *v).collect()
}

/// `(DataVersion, version name)` of every built-in Java version, newest first. Only the header
/// of each table is read, so this is cheap.
pub fn java_data_versions() -> Vec<(i32, &'static str)> {
    JAVA.iter()
        .filter_map(|(name, text)| {
            let dv = text
                .lines()
                .take(8)
                .find_map(|l| l.strip_prefix("data_version "))?
                .trim()
                .parse()
                .ok()?;
            Some((dv, *name))
        })
        .collect()
}

/// The registry of a built-in Java version.
pub fn java(version: &str) -> Option<Registry> {
    let (_, text) = JAVA.iter().find(|(v, _)| *v == version)?;
    Some(Registry::from_text(text).expect("built-in registry is valid"))
}

/// Bedrock tables, built by asking the real Bedrock Dedicated Server (see `mctk-gen`). State
/// values are stored with their NBT type: `b:1` (byte), `i:3` (int), `s:top` (string).
const BEDROCK: &[(&str, &str, &str)] = &[
    (
        "1.26.61.1",
        include_str!("../data/bedrock/1.26.61.1.tsv"),
        include_str!("../data/bedrock/1.26.61.1.aliases.tsv"),
    ),
    (
        "1.21.93.1",
        include_str!("../data/bedrock/1.21.93.1.tsv"),
        include_str!("../data/bedrock/1.21.93.1.aliases.tsv"),
    ),
    (
        "1.21.80.3",
        include_str!("../data/bedrock/1.21.80.3.tsv"),
        include_str!("../data/bedrock/1.21.80.3.aliases.tsv"),
    ),
    (
        "1.21.70.03",
        include_str!("../data/bedrock/1.21.70.03.tsv"),
        include_str!("../data/bedrock/1.21.70.03.aliases.tsv"),
    ),
    (
        "1.21.60.10",
        include_str!("../data/bedrock/1.21.60.10.tsv"),
        include_str!("../data/bedrock/1.21.60.10.aliases.tsv"),
    ),
    (
        "1.21.50.10",
        include_str!("../data/bedrock/1.21.50.10.tsv"),
        include_str!("../data/bedrock/1.21.50.10.aliases.tsv"),
    ),
    (
        "1.21.40.03",
        include_str!("../data/bedrock/1.21.40.03.tsv"),
        include_str!("../data/bedrock/1.21.40.03.aliases.tsv"),
    ),
    (
        "1.21.30.03",
        include_str!("../data/bedrock/1.21.30.03.tsv"),
        include_str!("../data/bedrock/1.21.30.03.aliases.tsv"),
    ),
    (
        "1.21.20.03",
        include_str!("../data/bedrock/1.21.20.03.tsv"),
        include_str!("../data/bedrock/1.21.20.03.aliases.tsv"),
    ),
    (
        "1.21.2.02",
        include_str!("../data/bedrock/1.21.2.02.tsv"),
        include_str!("../data/bedrock/1.21.2.02.aliases.tsv"),
    ),
    (
        "1.21.0.03",
        include_str!("../data/bedrock/1.21.0.03.tsv"),
        include_str!("../data/bedrock/1.21.0.03.aliases.tsv"),
    ),
    (
        "1.20.80.05",
        include_str!("../data/bedrock/1.20.80.05.tsv"),
        include_str!("../data/bedrock/1.20.80.05.aliases.tsv"),
    ),
    (
        "1.20.50.03",
        include_str!("../data/bedrock/1.20.50.03.tsv"),
        include_str!("../data/bedrock/1.20.50.03.aliases.tsv"),
    ),
    (
        "1.20.40.01",
        include_str!("../data/bedrock/1.20.40.01.tsv"),
        include_str!("../data/bedrock/1.20.40.01.aliases.tsv"),
    ),
    (
        "1.20.30.02",
        include_str!("../data/bedrock/1.20.30.02.tsv"),
        include_str!("../data/bedrock/1.20.30.02.aliases.tsv"),
    ),
    (
        "1.20.15.01",
        include_str!("../data/bedrock/1.20.15.01.tsv"),
        include_str!("../data/bedrock/1.20.15.01.aliases.tsv"),
    ),
    (
        "1.20.10.01",
        include_str!("../data/bedrock/1.20.10.01.tsv"),
        include_str!("../data/bedrock/1.20.10.01.aliases.tsv"),
    ),
    (
        "1.20.0.01",
        include_str!("../data/bedrock/1.20.0.01.tsv"),
        include_str!("../data/bedrock/1.20.0.01.aliases.tsv"),
    ),
    (
        "1.19.80.02",
        include_str!("../data/bedrock/1.19.80.02.tsv"),
        include_str!("../data/bedrock/1.19.80.02.aliases.tsv"),
    ),
    (
        "1.19.70.02",
        include_str!("../data/bedrock/1.19.70.02.tsv"),
        include_str!("../data/bedrock/1.19.70.02.aliases.tsv"),
    ),
    (
        "1.19.50.02",
        include_str!("../data/bedrock/1.19.50.02.tsv"),
        include_str!("../data/bedrock/1.19.50.02.aliases.tsv"),
    ),
    (
        "1.19.40.02",
        include_str!("../data/bedrock/1.19.40.02.tsv"),
        include_str!("../data/bedrock/1.19.40.02.aliases.tsv"),
    ),
    (
        "1.19.30.04",
        include_str!("../data/bedrock/1.19.30.04.tsv"),
        include_str!("../data/bedrock/1.19.30.04.aliases.tsv"),
    ),
    (
        "1.19.20.02",
        include_str!("../data/bedrock/1.19.20.02.tsv"),
        include_str!("../data/bedrock/1.19.20.02.aliases.tsv"),
    ),
    (
        "1.19.10.03",
        include_str!("../data/bedrock/1.19.10.03.tsv"),
        include_str!("../data/bedrock/1.19.10.03.aliases.tsv"),
    ),
    (
        "1.18.33.02",
        include_str!("../data/bedrock/1.18.33.02.tsv"),
        include_str!("../data/bedrock/1.18.33.02.aliases.tsv"),
    ),
    (
        "1.18.30.04",
        include_str!("../data/bedrock/1.18.30.04.tsv"),
        include_str!("../data/bedrock/1.18.30.04.aliases.tsv"),
    ),
    (
        "1.18.12.01",
        include_str!("../data/bedrock/1.18.12.01.tsv"),
        include_str!("../data/bedrock/1.18.12.01.aliases.tsv"),
    ),
    (
        "1.18.2.03",
        include_str!("../data/bedrock/1.18.2.03.tsv"),
        include_str!("../data/bedrock/1.18.2.03.aliases.tsv"),
    ),
    (
        "1.18.0.02",
        include_str!("../data/bedrock/1.18.0.02.tsv"),
        include_str!("../data/bedrock/1.18.0.02.aliases.tsv"),
    ),
    (
        "1.17.41.01",
        include_str!("../data/bedrock/1.17.41.01.tsv"),
        include_str!("../data/bedrock/1.17.41.01.aliases.tsv"),
    ),
];

/// The Bedrock versions with a table, newest first.
pub fn bedrock_versions() -> Vec<&'static str> {
    BEDROCK.iter().map(|(v, _, _)| *v).collect()
}

/// The registry of a built-in Bedrock version.
pub fn bedrock(version: &str) -> Option<Registry> {
    let (_, text, _) = BEDROCK.iter().find(|(v, _, _)| *v == version)?;
    Some(Registry::from_text(text).expect("built-in registry is valid"))
}

/// Bedrock biome numeric ids (`(name, id)`) that were read back from the real server's saved
/// chunks. Only biomes the server located and saved are listed.
pub fn bedrock_biome_ids(version: &str) -> Vec<(&'static str, i32)> {
    const IDS: &[(&str, &str)] = &[("1.26.61.1", include_str!("../data/bedrock/1.26.61.1.biome_ids.tsv"))];
    IDS.iter()
        .find(|(v, _)| *v == version)
        .map(|(_, t)| {
            t.lines()
                .filter(|l| !l.starts_with('#'))
                .filter_map(|l| {
                    let (name, id) = l.split_once('\t')?;
                    Some((name, id.trim().parse().ok()?))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What the Java game itself makes of a legacy (1.8-era, numeric) block: `id<TAB>data<TAB>state`
/// for every id 0-255 and data 0-15, read back from a real 26.3 server's world upgrade.
pub fn legacy_blocks_text() -> &'static str {
    include_str!("../data/legacy/blocks.tsv")
}

/// The same for items: `id<TAB>damage<TAB>"item"<TAB>components`.
pub fn legacy_items_text() -> &'static str {
    include_str!("../data/legacy/items.tsv")
}

/// What the Bedrock server makes of every legacy (numeric, 1.12-era) block: `id<TAB>data<TAB>name<TAB>states`
/// with typed values, read back from a real server's upgrade of chunks in the old format.
pub fn bedrock_legacy_blocks_text() -> &'static str {
    include_str!("../data/bedrock/legacy_blocks.tsv")
}

/// The block and item ids the legacy console game defines: `block<TAB>id` and `item<TAB>id`.
pub fn legacy_console_ids_text() -> &'static str {
    include_str!("../data/legacy/console_ids.tsv")
}

/// Legacy biome id to Java biome, again from the game's own upgrade.
pub fn legacy_biomes_text() -> &'static str {
    include_str!("../data/legacy/biomes.tsv")
}

/// Ids the game of a built-in Bedrock version turns into another block when it reads them
/// (`minecraft:carpet` becomes `minecraft:white_carpet`): `(old id, id it becomes)`.
pub fn bedrock_aliases(version: &str) -> Vec<(&'static str, &'static str)> {
    BEDROCK
        .iter()
        .find(|(v, _, _)| *v == version)
        .map(|(_, _, a)| a.lines().filter_map(|l| l.split_once('\t')).collect())
        .unwrap_or_default()
}


/// Pre-1.13 string id and numeric id of every block and item.
pub fn legacy_ids_text() -> &'static str {
    include_str!("../data/legacy/ids.tsv")
}

/// Pre-1.11 entity names and the entity they became.
pub fn legacy_entities_text() -> &'static str {
    include_str!("../data/legacy/entities.tsv")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_parses_and_has_a_data_version() {
        for v in java_versions() {
            let r = java(v).unwrap();
            assert_eq!(r.version, v);
            assert!(r.data_version.is_some(), "{v}");
            assert!(r.has_block("minecraft:stone") && r.has_item("minecraft:stone"));
        }
        assert!(java("0.0").is_none());
    }

    #[test]
    fn bedrock_table_knows_real_states_and_ranges() {
        use crate::{BlockCheck, StateProblem};
        let r = bedrock("1.26.61.1").unwrap();
        assert_eq!(r.edition, crate::Edition::Bedrock);
        assert!(r.blocks.len() > 1200, "{}", r.blocks.len());
        // Typed values, as the game saves them.
        let ok = [("weirdo_direction", "i:3"), ("upside_down_bit", "b:1"), ("minecraft:corner", "s:inner_left")];
        assert_eq!(r.check_block("minecraft:oak_stairs", ok), BlockCheck::Exact);
        // Ranges are the game's own: 4 stair directions, 16 redstone power levels, 64 bookshelf slots.
        let bad = r.check_block("minecraft:oak_stairs", [("weirdo_direction", "i:4"), ("upside_down_bit", "b:0"), ("minecraft:corner", "s:none")]);
        assert_eq!(bad, BlockCheck::BadState(vec![StateProblem::BadValue { property: "weirdo_direction".into(), value: "i:4".into() }]));
        assert!(r.blocks["minecraft:redstone_wire"].properties["redstone_signal"].contains(&"i:15".to_string()));
        assert!(!r.blocks["minecraft:redstone_wire"].properties["redstone_signal"].contains(&"i:16".to_string()));
        assert!(r.blocks["minecraft:chiseled_bookshelf"].properties["books_stored"].contains(&"i:63".to_string()));
        // Ids that are not blocks are not in the table.
        assert!(!r.has_block("minecraft:pig") && !r.has_block("minecraft:black_wool_double_slab"));
        assert_eq!(r.default_state("minecraft:oak_log").unwrap()["pillar_axis"], "s:y");
    }

    #[test]
    fn bedrock_aliases_name_real_blocks() {
        let r = bedrock("1.26.61.1").unwrap();
        let aliases = bedrock_aliases("1.26.61.1");
        assert!(aliases.len() > 50);
        assert!(aliases.contains(&("minecraft:carpet", "minecraft:white_carpet")));
        // Most targets are blocks of the table (a few are the game's placeholders).
        let known = aliases.iter().filter(|(_, to)| r.has_block(to)).count();
        assert!(known * 10 > aliases.len() * 8, "{known} of {}", aliases.len());
    }
}
