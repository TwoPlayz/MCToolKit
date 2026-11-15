//! The user's replacement choices, saved as a plain text file and reusable across
//! conversions.
//!
//! One line per override: `from<TAB>action[<TAB>to]`, where `from` is a block id
//! (`minecraft:leaf_litter`) or one exact state (`minecraft:oak_stairs[facing=north]`) and
//! `action` is `replace` (with `to` a state), `air` or `keep`. A state entry wins over a block
//! entry; anything without an entry falls back to the smart default.

use indexmap::IndexMap;
use mctk_java::BlockState;
use mctk_registry::{Registry, Renames};

use crate::suggest::{Action, Reason, suggest};

#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("line {line}: {msg}")]
    Line { line: usize, msg: String },
}

/// Parses `minecraft:name[a=b,c=d]`.
pub fn parse_state(text: &str) -> Option<BlockState> {
    let text = text.trim();
    let Some((name, rest)) = text.split_once('[') else {
        return (!text.is_empty()).then(|| BlockState::new(text));
    };
    let props = rest.strip_suffix(']')?;
    let mut state = BlockState::new(name);
    for p in props.split(',').filter(|p| !p.is_empty()) {
        let (k, v) = p.split_once('=')?;
        state.properties.insert(k.to_string(), v.to_string());
    }
    Some(state)
}

/// Where a resolved action came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The user chose it.
    User,
    /// The smart default.
    Default(Reason),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplacementMap {
    entries: IndexMap<String, Action>,
}

impl ReplacementMap {
    pub fn set(&mut self, from: impl Into<String>, action: Action) {
        self.entries.insert(from.into(), action);
    }

    pub fn get(&self, key: &str) -> Option<&Action> {
        self.entries.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Action)> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The action for `state` in `target`: the user's choice if there is one, else the smart
    /// default, else `None` (the state is already exact and stays as it is).
    pub fn resolve(
        &self,
        state: &BlockState,
        target: &Registry,
        renames: &Renames,
    ) -> Option<(Action, Source)> {
        let by_state = self.entries.get(&state.to_string());
        let by_name = self.entries.get(&state.name);
        match by_state.or(by_name) {
            // "Use this block": keep the rotation and other properties the new block shares. A
            // block the target lacks falls back to the smart default.
            Some(Action::Block(name)) => {
                if let Some(s) = crate::suggest::adapt(state, name, target) {
                    return Some((Action::Replace(s), Source::User));
                }
            }
            Some(a) => return Some((a.clone(), Source::User)),
            None => {}
        }
        suggest(state, target, renames).map(|g| (g.action, Source::Default(g.reason)))
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from("# mctk replacement map: from<TAB>replace<TAB>to | air | keep\n");
        for (from, action) in &self.entries {
            match action {
                Action::Replace(to) => out.push_str(&format!("{from}\treplace\t{to}\n")),
                Action::Block(to) => out.push_str(&format!("{from}\tblock\t{to}\n")),
                Action::Air => out.push_str(&format!("{from}\tair\n")),
                Action::Keep => out.push_str(&format!("{from}\tkeep\n")),
            }
        }
        out
    }

    pub fn from_text(text: &str) -> Result<Self, MapError> {
        let mut map = ReplacementMap::default();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let err = |msg: &str| MapError::Line {
                line: i + 1,
                msg: msg.into(),
            };
            let mut cols = line.split('\t');
            let from = cols.next().ok_or_else(|| err("missing block"))?;
            let action = match cols.next().ok_or_else(|| err("missing action"))? {
                "air" => Action::Air,
                "keep" => Action::Keep,
                "block" => Action::Block(cols.next().ok_or_else(|| err("block needs a target"))?.to_string()),
                "replace" => {
                    let to = cols.next().ok_or_else(|| err("replace needs a target"))?;
                    Action::Replace(parse_state(to).ok_or_else(|| err("bad target state"))?)
                }
                other => return Err(err(&format!("unknown action {other}"))),
            };
            map.entries.insert(from.to_string(), action);
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_states() {
        let s = parse_state("minecraft:oak_stairs[facing=north,half=top]").unwrap();
        assert_eq!(s.name, "minecraft:oak_stairs");
        assert_eq!(s.properties["half"], "top");
        assert_eq!(parse_state("minecraft:stone").unwrap().properties.len(), 0);
        assert!(parse_state("minecraft:x[a]").is_none());
    }

    #[test]
    fn text_round_trips() {
        let mut m = ReplacementMap::default();
        m.set("minecraft:leaf_litter", Action::Air);
        m.set(
            "minecraft:poplar_planks",
            Action::Replace(BlockState::new("minecraft:birch_planks")),
        );
        m.set("minecraft:x[a=b]", Action::Keep);
        assert_eq!(ReplacementMap::from_text(&m.to_text()).unwrap(), m);
        assert!(ReplacementMap::from_text("minecraft:a\tbogus").is_err());
    }

    #[test]
    fn block_action_keeps_shared_properties() {
        let mut target = Registry::new(mctk_registry::Edition::Java, "t");
        target.blocks.insert(
            "minecraft:oak_stairs".into(),
            mctk_registry::BlockDef {
                properties: IndexMap::from([("facing".to_string(), vec!["north".to_string(), "south".to_string()])]),
                defaults: IndexMap::from([("facing".to_string(), "north".to_string())]),
            },
        );
        let mut m = ReplacementMap::default();
        m.set("minecraft:pale_oak_stairs", Action::Block("minecraft:oak_stairs".into()));
        let st = BlockState::new("minecraft:pale_oak_stairs").with("facing", "south").with("half", "top");
        let (a, src) = m.resolve(&st, &target, &Renames::default()).unwrap();
        assert_eq!(src, Source::User);
        assert_eq!(a, Action::Replace(BlockState::new("minecraft:oak_stairs").with("facing", "south")));
        // Survives the text format.
        assert_eq!(ReplacementMap::from_text(&m.to_text()).unwrap(), m);
        // A block the target lacks falls back to the smart default instead of failing.
        m.set("minecraft:pale_oak_stairs", Action::Block("minecraft:nope".into()));
        assert!(m.resolve(&st, &target, &Renames::default()).is_some());
    }

    #[test]
    fn user_choice_beats_default_and_state_beats_block() {
        let mut target = Registry::new(mctk_registry::Edition::Java, "t");
        target.blocks.insert(
            "minecraft:oak_planks".into(),
            mctk_registry::BlockDef {
                properties: IndexMap::new(),
                defaults: IndexMap::new(),
            },
        );
        let renames = Renames::default();
        let poplar = BlockState::new("minecraft:poplar_planks");

        let mut m = ReplacementMap::default();
        let (a, src) = m.resolve(&poplar, &target, &renames).unwrap();
        assert_eq!(a, Action::Replace(BlockState::new("minecraft:oak_planks")));
        assert_eq!(src, Source::Default(Reason::SameShape));

        m.set("minecraft:poplar_planks", Action::Air);
        assert_eq!(m.resolve(&poplar, &target, &renames).unwrap(), (Action::Air, Source::User));

        let special = BlockState::new("minecraft:poplar_planks").with("a", "b");
        m.set(special.to_string(), Action::Keep);
        assert_eq!(m.resolve(&special, &target, &renames).unwrap().0, Action::Keep);
        // Already exact and not overridden: nothing to do.
        assert!(m.resolve(&BlockState::new("minecraft:oak_planks"), &target, &renames).is_none());
    }
}
