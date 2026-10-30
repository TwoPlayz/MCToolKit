//! Edition-neutral model of Minecraft world data and its Java / Bedrock codecs.
//!
//! This crate is a Rust port of parts of [Chunker](https://github.com/HiveGamesOSS/Chunker)
//! by Hive Games, licensed under the MIT licence (see `LICENSE-CHUNKER`). Chunker converts
//! worlds by reading each edition into an intermediate model and writing the model back
//! out; this crate keeps that design. The port covers the current game versions only (Java
//! 26.x, Bedrock 1.26.x); Chunker's handling of older versions is not ported.
//!
//! Ported: item stacks and their properties ([`item`]), text components ([`text`]), the id
//! tables they use ([`ids`]), every block entity ([`blockentity`]), the entity type table and
//! paintings ([`entity`]) and item frames ([`frame`]). Chunker's conversion of older game
//! versions and its block and biome resolvers are not ported.

pub mod blockentity;
pub mod entity;
pub mod font;
pub mod frame;
pub mod ids;
pub mod item;
pub mod text;

use std::collections::BTreeMap;

use mctk_nbt::{Compound, List, Tag, TagType};

/// Data a conversion could not carry over, by kind (Chunker's "missing mappings" log).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Notes {
    pub missing: BTreeMap<String, usize>,
}

impl Notes {
    pub fn miss(&mut self, what: impl Into<String>) {
        *self.missing.entry(what.into()).or_insert(0) += 1;
    }

    pub fn merge(&mut self, other: Notes) {
        for (k, v) in other.missing {
            *self.missing.entry(k).or_insert(0) += v;
        }
    }
}

pub(crate) fn int(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(Tag::as_i64)
}

pub(crate) fn string<'a>(c: &'a Compound, key: &str) -> Option<&'a str> {
    c.get(key).and_then(Tag::as_str)
}

pub(crate) fn compound<'a>(c: &'a Compound, key: &str) -> Option<&'a Compound> {
    c.get(key).and_then(Tag::as_compound)
}

/// Elements of a list tag (none when absent or not a list).
pub(crate) fn list<'a>(c: &'a Compound, key: &str) -> impl Iterator<Item = &'a Tag> {
    c.get(key)
        .and_then(Tag::as_list)
        .map(List::iter)
        .into_iter()
        .flatten()
}

/// The compound child `key`, created when missing.
pub(crate) fn child<'a>(c: &'a mut Compound, key: &str) -> &'a mut Compound {
    if !matches!(c.get(key), Some(Tag::Compound(_))) {
        c.insert(key.into(), Tag::Compound(Compound::new()));
    }
    match c.get_mut(key) {
        Some(Tag::Compound(inner)) => inner,
        _ => unreachable!("inserted above"),
    }
}

/// A list of compounds; empty lists are typed "End", as the games write them.
pub(crate) fn compound_list(items: Vec<Compound>) -> Tag {
    if items.is_empty() {
        return Tag::List(List::new(TagType::End));
    }
    Tag::List(
        List::from_tags(
            TagType::Compound,
            items.into_iter().map(Tag::Compound).collect(),
        )
        .expect("compounds"),
    )
}

pub(crate) fn string_list(items: Vec<String>) -> Tag {
    if items.is_empty() {
        return Tag::List(List::new(TagType::End));
    }
    Tag::List(
        List::from_tags(TagType::String, items.into_iter().map(Tag::String).collect())
            .expect("strings"),
    )
}

