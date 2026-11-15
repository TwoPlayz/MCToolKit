//! Block id renames between versions (same block, new name).

use indexmap::IndexMap;

use crate::{Error, bad};

/// `old id -> new id`, usable in both directions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Renames {
    forward: IndexMap<String, String>,
}

impl Renames {
    /// Parses `old<TAB>new` lines; blank lines and `#` comments are ignored.
    pub fn from_text(text: &str) -> Result<Self, Error> {
        let mut forward = IndexMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (old, new) = line
                .split_once('\t')
                .ok_or_else(|| bad(format!("bad rename line: {line}")))?;
            forward.insert(old.to_string(), new.to_string());
        }
        Ok(Renames { forward })
    }

    /// The renames recorded for Java blocks.
    pub fn java_blocks() -> Self {
        Self::from_text(include_str!("../data/java/block_renames.tsv"))
            .expect("built-in rename table is valid")
    }

    /// The other name of `id`: the new name if `id` is an old one, the old name if it is a
    /// new one.
    pub fn counterpart(&self, id: &str) -> Option<&str> {
        self.forward
            .get(id)
            .map(String::as_str)
            .or_else(|| self.forward.iter().find(|(_, n)| *n == id).map(|(o, _)| o.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counterpart_goes_both_ways() {
        let r = Renames::java_blocks();
        assert_eq!(r.counterpart("minecraft:grass"), Some("minecraft:short_grass"));
        assert_eq!(r.counterpart("minecraft:short_grass"), Some("minecraft:grass"));
        assert_eq!(r.counterpart("minecraft:stone"), None);
    }
}
