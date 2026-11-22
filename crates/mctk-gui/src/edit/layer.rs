//! The edit layer: chunks changed by the World Explorer's tools, kept in memory in each edition's
//! own form until the world is saved. The 3D view reads through it (so edits show at once), and
//! every tool use is one step to undo.
//!
//! - Java: a chunk's NBT (`None`: the chunk deleted) and, from 1.17, its entities' record.
//! - Bedrock: database records by key (`None`: the record deleted), each noted with its chunk.
//! - Console: a chunk's NBT (`None`: deleted).

use std::collections::{BTreeSet, HashMap};

use mctk_nbt::NamedTag;

/// A chunk: its dimension (`minecraft:overworld`...) and position.
pub type ChunkId = (String, i32, i32);

/// A Java chunk as changed.
#[derive(Clone, Debug, PartialEq)]
pub struct JavaChunk {
    /// The chunk's NBT; `None` once deleted.
    pub root: Option<NamedTag>,
    /// Its entities' record (1.17 and later keep them apart): `None` when unchanged (read from
    /// the world), `Some(None)` when deleted.
    pub entities: Option<Option<NamedTag>>,
}

/// What a key held in the layer before (or after) a step: `None` when the layer did not have it.
#[derive(Clone, Debug)]
enum Entry {
    Java(ChunkId, Option<JavaChunk>),
    Console(ChunkId, Option<Option<NamedTag>>),
    Bedrock(Vec<u8>, ChunkId, Option<Option<Vec<u8>>>),
}

/// One tool use: what the keys it changed held before and after.
#[derive(Debug)]
struct Step {
    label: String,
    before: Vec<Entry>,
    after: Vec<Entry>,
}

/// How many steps are kept to undo.
const HISTORY: usize = 100;

#[derive(Default)]
pub struct EditLayer {
    pub java: HashMap<ChunkId, JavaChunk>,
    pub console: HashMap<ChunkId, Option<NamedTag>>,
    pub bedrock: HashMap<Vec<u8>, Option<Vec<u8>>>,
    /// The chunk each Bedrock record belongs to.
    bedrock_chunk: HashMap<Vec<u8>, ChunkId>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// The step being made (between [`EditLayer::begin`] and [`EditLayer::commit`]).
    open: Option<Step>,
    /// Bumped on every change, for views to notice.
    pub generation: u64,
}

impl EditLayer {
    /// Starts a step; the changes until [`EditLayer::commit`] are undone together.
    /// Goes on with the last step (a brush stroke painting as the pointer moves): what follows
    /// is undone with what it already did. A new step when there is none to go on with.
    pub fn resume(&mut self, label: impl Into<String>) {
        match self.undo.pop() {
            Some(step) if self.redo.is_empty() => self.open = Some(Step { after: Vec::new(), ..step }),
            Some(step) => {
                self.undo.push(step);
                self.begin(label);
            }
            None => self.begin(label),
        }
    }

    pub fn begin(&mut self, label: impl Into<String>) {
        self.open = Some(Step { label: label.into(), before: Vec::new(), after: Vec::new() });
    }

    fn note(&mut self, e: Entry) {
        let Some(step) = &mut self.open else { return };
        let same = |a: &Entry| match (a, &e) {
            (Entry::Java(x, _), Entry::Java(y, _)) | (Entry::Console(x, _), Entry::Console(y, _)) => x == y,
            (Entry::Bedrock(x, ..), Entry::Bedrock(y, ..)) => x == y,
            _ => false,
        };
        if !step.before.iter().any(same) {
            step.before.push(e);
        }
    }

    pub fn set_java(&mut self, id: ChunkId, chunk: JavaChunk) {
        self.note(Entry::Java(id.clone(), self.java.get(&id).cloned()));
        self.java.insert(id, chunk);
    }

    pub fn set_console(&mut self, id: ChunkId, root: Option<NamedTag>) {
        self.note(Entry::Console(id.clone(), self.console.get(&id).cloned()));
        self.console.insert(id, root);
    }

    pub fn set_bedrock(&mut self, chunk: ChunkId, key: Vec<u8>, value: Option<Vec<u8>>) {
        self.note(Entry::Bedrock(key.clone(), chunk.clone(), self.bedrock.get(&key).cloned()));
        self.bedrock_chunk.insert(key.clone(), chunk);
        self.bedrock.insert(key, value);
    }

    /// Ends the step; gives the chunks it changed (empty when it changed nothing).
    pub fn commit(&mut self) -> Vec<ChunkId> {
        let Some(mut step) = self.open.take() else { return Vec::new() };
        if step.before.is_empty() {
            return Vec::new();
        }
        step.after = step.before.iter().map(|e| self.current(e)).collect();
        let chunks = chunks_of(&step.before);
        self.undo.push(step);
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.generation += 1;
        chunks
    }

    /// What the layer holds now for the key of `e`.
    fn current(&self, e: &Entry) -> Entry {
        match e {
            Entry::Java(id, _) => Entry::Java(id.clone(), self.java.get(id).cloned()),
            Entry::Console(id, _) => Entry::Console(id.clone(), self.console.get(id).cloned()),
            Entry::Bedrock(k, c, _) => Entry::Bedrock(k.clone(), c.clone(), self.bedrock.get(k).cloned()),
        }
    }

    fn put(&mut self, e: &Entry) {
        match e {
            Entry::Java(id, v) => match v {
                Some(v) => {
                    self.java.insert(id.clone(), v.clone());
                }
                None => {
                    self.java.remove(id);
                }
            },
            Entry::Console(id, v) => match v {
                Some(v) => {
                    self.console.insert(id.clone(), v.clone());
                }
                None => {
                    self.console.remove(id);
                }
            },
            Entry::Bedrock(k, c, v) => match v {
                Some(v) => {
                    self.bedrock.insert(k.clone(), v.clone());
                    self.bedrock_chunk.insert(k.clone(), c.clone());
                }
                None => {
                    self.bedrock.remove(k);
                    self.bedrock_chunk.remove(k);
                }
            },
        }
    }

    /// The steps that can be undone (newest first) and redone (the next one first), by name.
    pub fn history(&self) -> (Vec<String>, Vec<String>) {
        (self.undo.iter().rev().map(|s| s.label.clone()).collect(), self.redo.iter().rev().map(|s| s.label.clone()).collect())
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Takes back the last step: what it was called and the chunks it changed.
    pub fn undo(&mut self) -> Option<(String, Vec<ChunkId>)> {
        let step = self.undo.pop()?;
        for e in &step.before {
            self.put(e);
        }
        let out = (step.label.clone(), chunks_of(&step.before));
        self.redo.push(step);
        self.generation += 1;
        Some(out)
    }

    pub fn redo(&mut self) -> Option<(String, Vec<ChunkId>)> {
        let step = self.redo.pop()?;
        for e in &step.after {
            self.put(e);
        }
        let out = (step.label.clone(), chunks_of(&step.after));
        self.undo.push(step);
        self.generation += 1;
        Some(out)
    }

    /// Whether anything waits to be saved.
    pub fn is_dirty(&self) -> bool {
        !self.java.is_empty() || !self.console.is_empty() || !self.bedrock.is_empty()
    }

    /// Every chunk with unsaved changes.
    pub fn chunks(&self) -> BTreeSet<ChunkId> {
        let mut out: BTreeSet<ChunkId> = self.java.keys().cloned().collect();
        out.extend(self.console.keys().cloned());
        out.extend(self.bedrock.keys().filter_map(|k| self.bedrock_chunk.get(k).cloned()));
        out
    }

    /// Whether a chunk has unsaved changes.
    pub fn has_chunk(&self, id: &ChunkId) -> bool {
        self.java.contains_key(id) || self.console.contains_key(id) || self.bedrock_chunk.values().any(|c| c == id)
    }

    /// Whether a chunk is deleted (and not saved yet). A Bedrock chunk is, once its version
    /// record (which every chunk has) is.
    pub fn is_deleted(&self, id: &ChunkId) -> bool {
        self.java.get(id).is_some_and(|j| j.root.is_none())
            || self.console.get(id).is_some_and(Option::is_none)
            || self.bedrock_chunk.iter().any(|(k, c)| c == id && matches!(k.last(), Some(44 | 118)) && k.len() % 4 == 1 && self.bedrock.get(k).is_some_and(Option::is_none))
    }

    /// The deleted chunks of a dimension (x, z).
    pub fn deleted_in(&self, dim: &str) -> Vec<(i32, i32)> {
        self.chunks().into_iter().filter(|c| c.0 == dim && self.is_deleted(c)).map(|(_, x, z)| (x, z)).collect()
    }

    /// A chunk was saved from a tab that showed it with these changes: the layer lets go of it (what can be
    /// undone goes with it, when there was anything of the chunk).
    pub fn forget(&mut self, id: &ChunkId) {
        let had = self.has_chunk(id);
        self.java.remove(id);
        self.console.remove(id);
        let keys: Vec<Vec<u8>> = self.bedrock_chunk.iter().filter(|(_, c)| *c == id).map(|(k, _)| k.clone()).collect();
        for k in keys {
            self.bedrock.remove(&k);
            self.bedrock_chunk.remove(&k);
        }
        if had {
            self.undo.clear();
            self.redo.clear();
            self.generation += 1;
        }
    }

    /// After saving: nothing waits any more, and nothing can be undone.
    pub fn clear(&mut self) {
        *self = EditLayer { generation: self.generation + 1, ..Default::default() };
    }
}

fn chunks_of(entries: &[Entry]) -> Vec<ChunkId> {
    let mut out: Vec<ChunkId> = entries
        .iter()
        .map(|e| match e {
            Entry::Java(id, _) | Entry::Console(id, _) | Entry::Bedrock(_, id, _) => id.clone(),
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(n: i32) -> NamedTag {
        NamedTag { name: String::new(), tag: mctk_nbt::Tag::Int(n) }
    }

    #[test]
    fn steps_undo_and_redo_together() {
        let mut l = EditLayer::default();
        let a: ChunkId = ("minecraft:overworld".into(), 0, 0);
        l.begin("one");
        l.set_console(a.clone(), Some(tag(1)));
        l.set_console(a.clone(), Some(tag(2)));
        assert_eq!(l.commit(), vec![a.clone()]);
        l.begin("two");
        l.set_bedrock(a.clone(), vec![1, 2], None);
        l.commit();
        assert!(l.is_dirty() && l.has_chunk(&a));
        assert_eq!(l.undo().map(|u| u.0).as_deref(), Some("two"));
        assert!(l.bedrock.is_empty());
        assert_eq!(l.undo().map(|u| u.0).as_deref(), Some("one"));
        assert!(!l.is_dirty(), "back to nothing changed");
        l.redo();
        assert_eq!(l.console.get(&a), Some(&Some(tag(2))), "the step's end state, not its middle");
        l.redo();
        assert_eq!(l.bedrock.get(&vec![1, 2]), Some(&None));
        // A new step forgets what could be redone.
        l.undo();
        l.begin("three");
        l.set_console(a.clone(), None);
        l.commit();
        assert!(!l.can_redo());
        assert!(l.is_deleted(&a));
    }
}
