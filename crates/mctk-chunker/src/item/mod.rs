//! Item stacks: the edition-neutral model (`ChunkerItemStack` / `ChunkerItemProperty`) and
//! its Java and Bedrock codecs (`JavaComponentItemStackResolver`, `BedrockItemStackResolver`).
//! Ported from Chunker (MIT, Hive Games).
//!
//! The stack's identifier is the Java item name. Java ↔ Bedrock item names come from the
//! GeyserMC item table in `mctk-mapping`; items whose Bedrock variant lives in the data
//! value (potions, tipped arrows, goat horns, suspicious stew) are handled here, as Chunker
//! does in its item identifier resolvers.

pub mod bedrock;
pub mod java;

use std::collections::BTreeMap;

use crate::ids::{Effect, Enchantment, Horn, Potion};
use crate::text::Text;

#[derive(Debug, Clone, PartialEq)]
pub struct ItemStack {
    /// Java item name, e.g. `minecraft:diamond_sword`.
    pub id: String,
    pub count: i32,
    pub props: ItemProps,
}

/// Everything an item can carry besides its type and count. `None` means "not set".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemProps {
    /// Durability used up (Java `minecraft:damage`, Bedrock `tag.Damage`).
    pub damage: Option<i32>,
    pub name: Option<Text>,
    pub lore: Option<Vec<Text>>,
    /// Dye colour of leather armor and similar (RGB).
    pub color: Option<u32>,
    /// Block names (Java spelling) for adventure mode.
    pub can_place_on: Option<Vec<String>>,
    pub can_destroy: Option<Vec<String>>,
    pub repair_cost: Option<i32>,
    pub enchantments: Option<BTreeMap<Enchantment, i32>>,
    /// Entity a spawn egg makes, when the item records one explicitly.
    pub spawn_egg: Option<String>,
    pub book_title: Option<String>,
    pub book_author: Option<String>,
    pub book_pages: Option<Vec<Text>>,
    pub horn: Option<Horn>,
    pub potion: Option<Potion>,
    /// (material, pattern) without the `minecraft:` prefix.
    pub trim: Option<(String, String)>,
    pub stew: Option<Vec<(Effect, i32)>>,
    pub fireworks: Option<Fireworks>,
    pub firework_explosion: Option<Explosion>,
    pub ominous_amplifier: Option<i32>,
    pub bundle: Option<Vec<ItemStack>>,
    /// Contents of a container item (shulker box), by slot.
    pub container: Option<Vec<(u8, ItemStack)>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fireworks {
    pub flight: i8,
    pub explosions: Vec<Explosion>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Explosion {
    /// Index into [`crate::ids::FIREWORK_SHAPES`].
    pub shape: usize,
    /// RGB colours.
    pub colors: Vec<u32>,
    pub fade: Vec<u32>,
    pub trail: bool,
    pub twinkle: bool,
}

impl ItemStack {
    pub fn new(id: impl Into<String>, count: i32) -> Self {
        ItemStack {
            id: id.into(),
            count,
            props: ItemProps::default(),
        }
    }

    pub fn is_air(&self) -> bool {
        self.id == "minecraft:air" || self.id.is_empty() || self.count <= 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Notes, text};
    use mctk_nbt::Tag;

    /// Java → Bedrock → Java must give the stack back.
    fn round_trip(stack: &ItemStack) -> ItemStack {
        let mut notes = Notes::default();
        let b = bedrock::write(stack, Some(3), &mut notes).expect("has a Bedrock form");
        let back = bedrock::read(&b, &mut notes).expect("reads back");
        assert!(notes.missing.is_empty(), "{:?}", notes.missing);
        back
    }

    #[test]
    fn plain_item_with_durability_name_and_enchantments() {
        let mut s = ItemStack::new("minecraft:diamond_sword", 1);
        s.props.damage = Some(7);
        s.props.name = Some(text::plain("Slayer"));
        s.props.repair_cost = Some(3);
        s.props.enchantments = Some(
            [(Enchantment::Sharpness, 5), (Enchantment::Mending, 1)]
                .into_iter()
                .collect(),
        );
        assert_eq!(round_trip(&s), s);
    }

    #[test]
    fn potions_arrows_horns_and_stew_use_the_data_value() {
        let mut potion = ItemStack::new("minecraft:splash_potion", 1);
        potion.props.potion = Some(Potion::StrongHealing);
        assert_eq!(round_trip(&potion), potion);

        let mut arrow = ItemStack::new("minecraft:tipped_arrow", 12);
        arrow.props.potion = Some(Potion::LongPoison);
        assert_eq!(round_trip(&arrow), arrow);
        let b = bedrock::write(&arrow, None, &mut Notes::default()).unwrap();
        assert_eq!(b.get("Name").and_then(Tag::as_str), Some("minecraft:arrow"));
        assert_eq!(b.get("Damage"), Some(&Tag::Short(27)));

        let mut horn = ItemStack::new("minecraft:goat_horn", 1);
        horn.props.horn = Some(Horn::Yearn);
        assert_eq!(round_trip(&horn), horn);

        let mut stew = ItemStack::new("minecraft:suspicious_stew", 1);
        stew.props.stew = Some(vec![(Effect::Blindness, 160)]);
        assert_eq!(round_trip(&stew), stew);
    }

    #[test]
    fn java_components_round_trip() {
        let mut s = ItemStack::new("minecraft:enchanted_book", 1);
        s.props.enchantments = Some([(Enchantment::Lunge, 2)].into_iter().collect());
        s.props.lore = Some(vec![text::plain("a"), text::plain("b")]);
        s.props.trim = Some(("gold".into(), "wild".into()));
        let java = java::write(&s, Some(5));
        let mut notes = Notes::default();
        let back = java::read(&java, &mut notes).unwrap();
        assert_eq!(back, s);
        assert!(notes.missing.is_empty());
        assert_eq!(java.get("Slot"), Some(&mctk_nbt::Tag::Byte(5)));
    }

    #[test]
    fn firework_colours_survive_the_bedrock_dye_table() {
        let mut s = ItemStack::new("minecraft:firework_star", 1);
        s.props.firework_explosion = Some(Explosion {
            shape: 2,
            colors: vec![crate::ids::DYES[14].2, crate::ids::DYES[4].2],
            fade: vec![],
            trail: true,
            twinkle: false,
        });
        let back = round_trip(&s);
        assert_eq!(back.props.firework_explosion, s.props.firework_explosion);
    }

    #[test]
    fn lunge_is_dropped_only_where_bedrock_lacks_it() {
        let mut s = ItemStack::new("minecraft:iron_sword", 1);
        s.props.enchantments = Some([(Enchantment::SweepingEdge, 3)].into_iter().collect());
        let mut notes = Notes::default();
        let b = bedrock::write(&s, None, &mut notes).unwrap();
        assert!(b.get("tag").is_none());
        assert_eq!(notes.missing.len(), 1);
    }

    #[test]
    fn unknown_items_are_reported() {
        let s = ItemStack::new("minecraft:definitely_not_an_item", 1);
        let mut notes = Notes::default();
        assert!(bedrock::write(&s, None, &mut notes).is_none());
        assert_eq!(notes.missing.len(), 1);
    }
}
