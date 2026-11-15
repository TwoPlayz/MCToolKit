//! Per-version registries: which blocks, block-state properties, items, entities and
//! biomes exist in one edition and version.
//!
//! The data is generated from the games themselves (see [`java_reports`]) and stored in a
//! small line-based text format ([`Registry::to_text`] / [`Registry::from_text`]) so
//! changes between versions show up in a plain diff. A registry answers one question the
//! converter keeps needing: "is this exactly representable in that version?"
//! ([`Registry::check_block`]).

pub mod builtin;
pub mod java_reports;
mod renames;

pub use renames::Renames;

use indexmap::IndexMap;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid registry data: {0}")]
    Invalid(String),
}

fn bad(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

/// The game edition a registry describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edition {
    Java,
    Bedrock,
    Console,
}

impl Edition {
    pub fn as_str(self) -> &'static str {
        match self {
            Edition::Java => "java",
            Edition::Bedrock => "bedrock",
            Edition::Console => "console",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "java" => Some(Edition::Java),
            "bedrock" => Some(Edition::Bedrock),
            "console" => Some(Edition::Console),
            _ => None,
        }
    }
}

/// One block and the properties its states may carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDef {
    /// Property name -> allowed values, in the game's declaration order.
    pub properties: IndexMap<String, Vec<String>>,
    /// Property name -> value of the default state.
    pub defaults: IndexMap<String, String>,
}

/// How well a block state fits a registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockCheck {
    /// Block, every property and every value exist.
    Exact,
    /// The block exists but the state does not: unknown properties, bad values or
    /// properties missing. Carries the problems found.
    BadState(Vec<StateProblem>),
    /// The block does not exist in this version.
    UnknownBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateProblem {
    UnknownProperty(String),
    BadValue { property: String, value: String },
    MissingProperty(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    pub edition: Edition,
    /// Human version label, e.g. `26.3`.
    pub version: String,
    /// Java `DataVersion` of this version, when known.
    pub data_version: Option<i32>,
    pub blocks: IndexMap<String, BlockDef>,
    pub items: Vec<String>,
    pub entities: Vec<String>,
    pub biomes: Vec<String>,
    pub block_entities: Vec<String>,
    /// The world-generation structures chunks may record (Java); empty when not known.
    pub structures: Vec<String>,
    /// Facts about the version that are not lists (Bedrock: `block_version`, `network_version`,
    /// `storage_version`, `client_version`), written as `meta key<TAB>value` lines.
    pub meta: IndexMap<String, String>,
    /// Bedrock: how the newest version's block states are spelled in this older version, as
    /// `(newest, this)` pairs of `name|key=b:1,key2=s:x` (`D` lines). A state not listed is
    /// spelled the same in both.
    pub downgrades: Vec<(String, String)>,
}

impl Registry {
    pub fn new(edition: Edition, version: impl Into<String>) -> Self {
        Registry {
            edition,
            version: version.into(),
            data_version: None,
            blocks: IndexMap::new(),
            items: Vec::new(),
            entities: Vec::new(),
            biomes: Vec::new(),
            block_entities: Vec::new(),
            structures: Vec::new(),
            meta: IndexMap::new(),
            downgrades: Vec::new(),
        }
    }

    pub fn has_block(&self, name: &str) -> bool {
        self.blocks.contains_key(name)
    }

    pub fn has_item(&self, name: &str) -> bool {
        self.items.iter().any(|i| i == name)
    }

    pub fn has_entity(&self, name: &str) -> bool {
        self.entities.iter().any(|i| i == name)
    }

    /// Checks a block state against this version's definition.
    pub fn check_block<'a>(
        &self,
        name: &str,
        properties: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> BlockCheck {
        let Some(def) = self.blocks.get(name) else {
            return BlockCheck::UnknownBlock;
        };
        let mut problems = Vec::new();
        let mut given: Vec<&str> = Vec::new();
        for (k, v) in properties {
            match def.properties.get(k) {
                None => problems.push(StateProblem::UnknownProperty(k.to_string())),
                Some(allowed) => {
                    given.push(k);
                    if !allowed.iter().any(|a| a == v) {
                        problems.push(StateProblem::BadValue {
                            property: k.to_string(),
                            value: v.to_string(),
                        });
                    }
                }
            }
        }
        // Java 26.x leaves out properties that equal the default, so an absent property is
        // only a problem when the version defines no default for it.
        for k in def.properties.keys() {
            if !given.contains(&k.as_str()) && !def.defaults.contains_key(k) {
                problems.push(StateProblem::MissingProperty(k.clone()));
            }
        }
        if problems.is_empty() {
            BlockCheck::Exact
        } else {
            BlockCheck::BadState(problems)
        }
    }

    /// `properties` with every property the block defines, in declaration order, taking
    /// missing ones from the default state. `None` if the block does not exist or a given
    /// property is unknown. Formats that store whole states (Bedrock) need this form.
    pub fn complete_state<'a>(
        &self,
        name: &str,
        properties: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Option<IndexMap<String, String>> {
        let def = self.blocks.get(name)?;
        let given: Vec<(&str, &str)> = properties.into_iter().collect();
        if given.iter().any(|(k, _)| !def.properties.contains_key(*k)) {
            return None;
        }
        Some(
            def.properties
                .keys()
                .filter_map(|k| {
                    let v = given
                        .iter()
                        .find(|(g, _)| g == k)
                        .map(|(_, v)| v.to_string())
                        .or_else(|| def.defaults.get(k).cloned())?;
                    Some((k.clone(), v))
                })
                .collect(),
        )
    }

    /// The default state of `name` as property pairs, if the block exists.
    pub fn default_state(&self, name: &str) -> Option<&IndexMap<String, String>> {
        self.blocks.get(name).map(|d| &d.defaults)
    }

    /// Serialises to the line format:
    ///
    /// ```text
    /// #mctk-registry 1
    /// edition java
    /// version 26.3
    /// B minecraft:oak_log<TAB>axis=x|y|z<TAB>axis=y
    /// I minecraft:stone
    /// E minecraft:pig
    /// M minecraft:plains
    /// T minecraft:chest
    /// S minecraft:village_plains
    /// ```
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("#mctk-registry 1\n");
        out.push_str(&format!("edition {}\n", self.edition.as_str()));
        out.push_str(&format!("version {}\n", self.version));
        if let Some(dv) = self.data_version {
            out.push_str(&format!("data_version {dv}\n"));
        }
        for (key, value) in &self.meta {
            out.push_str(&format!("meta {key}\t{value}\n"));
        }
        for (name, def) in &self.blocks {
            let props: Vec<String> = def
                .properties
                .iter()
                .map(|(k, vs)| format!("{k}={}", vs.join("|")))
                .collect();
            let defaults: Vec<String> = def
                .defaults
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            out.push_str(&format!(
                "B {name}\t{}\t{}\n",
                props.join(","),
                defaults.join(",")
            ));
        }
        for (tag, list) in [
            ('I', &self.items),
            ('E', &self.entities),
            ('M', &self.biomes),
            ('T', &self.block_entities),
            ('S', &self.structures),
        ] {
            for name in list {
                out.push_str(&format!("{tag} {name}\n"));
            }
        }
        for (newest, this) in &self.downgrades {
            out.push_str(&format!("D {newest}\t{this}\n"));
        }
        out
    }

    pub fn from_text(text: &str) -> Result<Self, Error> {
        let mut lines = text.lines();
        if lines.next() != Some("#mctk-registry 1") {
            return Err(bad("missing #mctk-registry 1 header"));
        }
        let mut edition = None;
        let mut version = None;
        let mut reg = Registry::new(Edition::Java, "");
        for line in lines {
            if let Some(rest) = line.strip_prefix("edition ") {
                edition = Some(Edition::parse(rest).ok_or_else(|| bad("unknown edition"))?);
            } else if let Some(rest) = line.strip_prefix("version ") {
                version = Some(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("data_version ") {
                reg.data_version = Some(rest.parse().map_err(|_| bad("bad data_version"))?);
            } else if let Some(rest) = line.strip_prefix("meta ") {
                let (k, v) = rest.split_once('\t').ok_or_else(|| bad("bad meta line"))?;
                reg.meta.insert(k.to_string(), v.to_string());
            } else if let Some(rest) = line.strip_prefix("B ") {
                let mut cols = rest.split('\t');
                let name = cols.next().ok_or_else(|| bad("empty block line"))?;
                let props = cols.next().unwrap_or("");
                let defaults = cols.next().unwrap_or("");
                let mut def = BlockDef {
                    properties: IndexMap::new(),
                    defaults: IndexMap::new(),
                };
                for p in props.split(',').filter(|s| !s.is_empty()) {
                    let (k, vs) = p.split_once('=').ok_or_else(|| bad("bad property"))?;
                    def.properties
                        .insert(k.to_string(), vs.split('|').map(str::to_string).collect());
                }
                for p in defaults.split(',').filter(|s| !s.is_empty()) {
                    let (k, v) = p.split_once('=').ok_or_else(|| bad("bad default"))?;
                    def.defaults.insert(k.to_string(), v.to_string());
                }
                reg.blocks.insert(name.to_string(), def);
            } else if let Some(rest) = line.strip_prefix("I ") {
                reg.items.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("E ") {
                reg.entities.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("M ") {
                reg.biomes.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("T ") {
                reg.block_entities.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("S ") {
                reg.structures.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("D ") {
                let (a, b) = rest.split_once('\t').ok_or_else(|| bad("bad downgrade line"))?;
                reg.downgrades.push((a.to_string(), b.to_string()));
            } else if !line.is_empty() {
                return Err(bad(format!("unrecognised line: {line}")));
            }
        }
        reg.edition = edition.ok_or_else(|| bad("missing edition"))?;
        reg.version = version.ok_or_else(|| bad("missing version"))?;
        Ok(reg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Registry {
        let mut r = Registry::new(Edition::Java, "test");
        r.data_version = Some(1234);
        r.blocks.insert(
            "minecraft:oak_log".into(),
            BlockDef {
                properties: IndexMap::from([(
                    "axis".to_string(),
                    vec!["x".to_string(), "y".to_string(), "z".to_string()],
                )]),
                defaults: IndexMap::from([("axis".to_string(), "y".to_string())]),
            },
        );
        r.blocks.insert(
            "minecraft:air".into(),
            BlockDef {
                properties: IndexMap::new(),
                defaults: IndexMap::new(),
            },
        );
        r.items.push("minecraft:stone".into());
        r.entities.push("minecraft:pig".into());
        r.biomes.push("minecraft:plains".into());
        r.block_entities.push("minecraft:chest".into());
        r
    }

    #[test]
    fn text_round_trips() {
        let r = sample();
        assert_eq!(Registry::from_text(&r.to_text()).unwrap(), r);
    }

    #[test]
    fn checks_blocks_and_states() {
        let r = sample();
        assert_eq!(
            r.check_block("minecraft:oak_log", [("axis", "x")]),
            BlockCheck::Exact
        );
        assert_eq!(r.check_block("minecraft:air", []), BlockCheck::Exact);
        assert_eq!(r.check_block("minecraft:nope", []), BlockCheck::UnknownBlock);
        assert_eq!(
            r.check_block("minecraft:oak_log", [("axis", "w")]),
            BlockCheck::BadState(vec![StateProblem::BadValue {
                property: "axis".into(),
                value: "w".into()
            }])
        );
        assert_eq!(
            r.check_block("minecraft:oak_log", [("facing", "north")]),
            BlockCheck::BadState(vec![StateProblem::UnknownProperty("facing".into())])
        );
        // A property equal to the default may be left out (Java 26.x), so this is exact.
        assert_eq!(r.check_block("minecraft:oak_log", []), BlockCheck::Exact);
    }

    #[test]
    fn completes_states_from_defaults() {
        let r = sample();
        let full = r.complete_state("minecraft:oak_log", []).unwrap();
        assert_eq!(full["axis"], "y");
        let full = r.complete_state("minecraft:oak_log", [("axis", "z")]).unwrap();
        assert_eq!(full["axis"], "z");
        assert!(r.complete_state("minecraft:oak_log", [("bogus", "1")]).is_none());
        assert!(r.complete_state("minecraft:nope", []).is_none());
    }
}
