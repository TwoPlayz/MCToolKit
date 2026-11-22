//! The replacement plan shown to the user before a conversion: every block the target
//! version cannot represent exactly, grouped by block, with the smart default and the user's
//! choice for each.

use indexmap::IndexMap;
use mctk_java::BlockState;
use mctk_registry::{Registry, Renames};

use crate::map::ReplacementMap;
use crate::suggest::{Action, Reason, Suggestion, suggest};
use crate::Scan;

/// What the user wants done with a block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Choice {
    /// The smart default for each state.
    #[default]
    Suggested,
    /// Remove the block.
    Air,
    /// Write it unchanged even though the target does not know it.
    Keep,
    /// Use this block (by id), carrying rotation and other shared properties over.
    Block(String),
}

/// One source block and every unsupported state of it found in the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanGroup {
    /// Block id, e.g. `minecraft:leaf_litter`.
    pub name: String,
    /// Each unsupported state with its block count and smart default, most used first.
    pub states: Vec<(BlockState, u64, Suggestion)>,
    pub choice: Choice,
}

impl PlanGroup {
    pub fn total(&self) -> u64 {
        self.states.iter().map(|(_, n, _)| n).sum()
    }

    /// The smart default of the most used state: what the user sees as "suggested".
    pub fn suggestion(&self) -> &Suggestion {
        &self.states[0].2
    }

    /// Short text for what the suggestion does, e.g. `minecraft:grass` or `remove`.
    pub fn suggestion_text(&self) -> String {
        match &self.suggestion().action {
            Action::Replace(s) => s.name.clone(),
            Action::Block(n) => n.clone(),
            Action::Air => "remove".into(),
            Action::Keep => "keep".into(),
        }
    }
}

/// Everything that needs a decision, biggest first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    pub groups: Vec<PlanGroup>,
}

impl Plan {
    /// The groups for the blocks of `scan` that `target` cannot represent exactly.
    pub fn build(scan: &Scan, target: &Registry, renames: &Renames) -> Plan {
        Self::build_where(scan, target, renames, |_| false)
    }

    /// [`Plan::build`] leaving out the states `supported` accepts (what the target writes by
    /// means the registry does not describe).
    pub fn build_where(scan: &Scan, target: &Registry, renames: &Renames, supported: impl Fn(&BlockState) -> bool) -> Plan {
        let mut by_name: IndexMap<String, Vec<(BlockState, u64, Suggestion)>> = IndexMap::new();
        for u in scan.unsupported(target) {
            if supported(&u.state) {
                continue;
            }
            let Some(g) = suggest(&u.state, target, renames) else { continue };
            by_name.entry(u.state.name.clone()).or_default().push((u.state, u.count, g));
        }
        let mut groups: Vec<PlanGroup> = by_name
            .into_iter()
            .map(|(name, states)| PlanGroup { name, states, choice: Choice::Suggested })
            .collect();
        groups.sort_by(|a, b| b.total().cmp(&a.total()).then_with(|| a.name.cmp(&b.name)));
        Plan { groups }
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The user's choices as a map for the converter. Groups left on the smart default add no
    /// entry, so the converter keeps deciding per state for them.
    pub fn to_map(&self) -> ReplacementMap {
        let mut map = ReplacementMap::default();
        for g in &self.groups {
            match &g.choice {
                Choice::Suggested => {}
                Choice::Air => map.set(g.name.clone(), Action::Air),
                Choice::Keep => map.set(g.name.clone(), Action::Keep),
                Choice::Block(b) => map.set(g.name.clone(), Action::Block(b.clone())),
            }
        }
        map
    }

    /// Applies a saved map: each group takes the entry saved for its block, if any.
    pub fn apply_map(&mut self, map: &ReplacementMap) {
        for g in &mut self.groups {
            g.choice = match map.get(&g.name) {
                Some(Action::Air) => Choice::Air,
                Some(Action::Keep) => Choice::Keep,
                Some(Action::Block(b)) => Choice::Block(b.clone()),
                Some(Action::Replace(s)) => Choice::Block(s.name.clone()),
                None => Choice::Suggested,
            };
        }
    }

    /// Puts every group back on its smart default.
    pub fn reset(&mut self) {
        for g in &mut self.groups {
            g.choice = Choice::Suggested;
        }
    }

    /// How many groups the user has changed from the smart default.
    pub fn changed(&self) -> usize {
        self.groups.iter().filter(|g| g.choice != Choice::Suggested).count()
    }

    /// How many blocks (not groups) end up removed under the current choices.
    pub fn blocks_removed(&self) -> u64 {
        self.groups
            .iter()
            .map(|g| match &g.choice {
                Choice::Air => g.total(),
                Choice::Suggested => g
                    .states
                    .iter()
                    .filter(|(_, _, s)| s.action == Action::Air)
                    .map(|(_, n, _)| n)
                    .sum(),
                _ => 0,
            })
            .sum()
    }
}

/// Blocks of `target` whose id contains `query` (ignoring case and any `minecraft:`), ids
/// that start with it first, at most `limit`. An empty query lists the first `limit`.
pub fn candidates(target: &Registry, query: &str, limit: usize) -> Vec<String> {
    let q = query.trim().to_lowercase();
    let q = q.strip_prefix("minecraft:").unwrap_or(&q);
    let mut starts = Vec::new();
    let mut contains = Vec::new();
    for id in target.blocks.keys() {
        let b = id.strip_prefix("minecraft:").unwrap_or(id);
        if b.starts_with(q) {
            starts.push(id.clone());
        } else if b.contains(q) {
            contains.push(id.clone());
        }
    }
    starts.extend(contains);
    starts.truncate(limit);
    starts
}

/// Why a suggestion was made, as a short label.
pub fn reason_label(r: Reason) -> &'static str {
    match r {
        Reason::AdaptedState => "properties adjusted",
        Reason::Renamed => "renamed",
        Reason::SameShape => "similar block",
        Reason::NoEquivalent => "no equivalent",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mctk_registry::{BlockDef, Edition};

    fn target() -> Registry {
        let mut r = Registry::new(Edition::Java, "t");
        for n in ["minecraft:grass", "minecraft:oak_planks", "minecraft:oak_stairs", "minecraft:stone_stairs", "minecraft:stone"] {
            r.blocks.insert(
                n.into(),
                BlockDef { properties: IndexMap::new(), defaults: IndexMap::new() },
            );
        }
        r
    }

    fn scan() -> Scan {
        let mut s = Scan::default();
        s.blocks.insert(BlockState::new("minecraft:stone"), 1000);
        s.blocks.insert(BlockState::new("minecraft:short_grass"), 90);
        s.blocks.insert(BlockState::new("minecraft:leaf_litter").with("segment_amount", "1"), 40);
        s.blocks.insert(BlockState::new("minecraft:leaf_litter").with("segment_amount", "2"), 25);
        s.blocks.insert(BlockState::new("minecraft:poplar_planks"), 5);
        s
    }

    #[test]
    fn groups_unsupported_blocks_by_id_most_used_first() {
        let plan = Plan::build(&scan(), &target(), &Renames::java_blocks());
        let names: Vec<&str> = plan.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["minecraft:short_grass", "minecraft:leaf_litter", "minecraft:poplar_planks"]);
        let litter = &plan.groups[1];
        assert_eq!(litter.states.len(), 2);
        assert_eq!(litter.total(), 65);
        assert_eq!(plan.groups[0].suggestion_text(), "minecraft:grass");
        assert_eq!(plan.groups[1].suggestion_text(), "remove");
        assert_eq!(plan.blocks_removed(), 65);
    }

    #[test]
    fn choices_become_a_map_and_come_back() {
        let mut plan = Plan::build(&scan(), &target(), &Renames::java_blocks());
        assert!(plan.to_map().is_empty());
        plan.groups[1].choice = Choice::Block("minecraft:grass".into());
        plan.groups[2].choice = Choice::Keep;
        assert_eq!(plan.changed(), 2);
        let map = plan.to_map();
        assert_eq!(map.get("minecraft:leaf_litter"), Some(&Action::Block("minecraft:grass".into())));
        assert_eq!(map.len(), 2);
        assert_eq!(plan.blocks_removed(), 0);

        let mut fresh = Plan::build(&scan(), &target(), &Renames::java_blocks());
        fresh.apply_map(&ReplacementMap::from_text(&map.to_text()).unwrap());
        assert_eq!(fresh, plan);
        fresh.reset();
        assert_eq!(fresh.changed(), 0);
    }

    #[test]
    fn candidate_search_ranks_prefixes_first() {
        let t = target();
        assert_eq!(candidates(&t, "oak", 10), ["minecraft:oak_planks", "minecraft:oak_stairs"]);
        assert_eq!(candidates(&t, "STAIRS", 10), ["minecraft:oak_stairs", "minecraft:stone_stairs"]);
        // Prefix matches come first, then blocks that merely contain the text.
        assert_eq!(
            candidates(&t, "minecraft:st", 10),
            ["minecraft:stone_stairs", "minecraft:stone", "minecraft:oak_stairs"]
        );
        assert_eq!(candidates(&t, "", 2).len(), 2);
        assert!(candidates(&t, "zzz", 5).is_empty());
    }
}
