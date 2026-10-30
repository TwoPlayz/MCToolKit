//! Id tables shared by item stacks and block entities, for current versions (Java 26.x,
//! Bedrock 1.26.x). Ported from Chunker's `*Resolver` classes (MIT, Hive Games).

/// Defines an enum whose variants carry their Java name and Bedrock id (either may be
/// missing when the edition does not have it).
macro_rules! id_table {
    ($(#[$m:meta])* $name:ident: $bedrock:ty { $($variant:ident => $java:expr, $bed:expr;)* }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name { $($variant,)* }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant,)*];

            /// The Java Edition name (`minecraft:...`).
            pub fn java(self) -> Option<&'static str> {
                match self { $($name::$variant => $java,)* }
            }

            /// The Bedrock Edition id.
            pub fn bedrock(self) -> Option<$bedrock> {
                match self { $($name::$variant => $bed,)* }
            }

            /// Looks up a Java name; the `minecraft:` prefix is optional.
            pub fn from_java(name: &str) -> Option<$name> {
                let name = name.strip_prefix("minecraft:").unwrap_or(name);
                Self::ALL.iter().copied().find(|v| {
                    v.java().is_some_and(|j| j.strip_prefix("minecraft:") == Some(name))
                })
            }

            pub fn from_bedrock(id: $bedrock) -> Option<$name> {
                Self::ALL.iter().copied().find(|v| v.bedrock() == Some(id))
            }
        }
    };
}

id_table! {
    /// Enchantments (`ChunkerEnchantmentType`).
    Enchantment: i16 {
        Protection => Some("minecraft:protection"), Some(0);
        FireProtection => Some("minecraft:fire_protection"), Some(1);
        FeatherFalling => Some("minecraft:feather_falling"), Some(2);
        BlastProtection => Some("minecraft:blast_protection"), Some(3);
        ProjectileProtection => Some("minecraft:projectile_protection"), Some(4);
        Thorns => Some("minecraft:thorns"), Some(5);
        Respiration => Some("minecraft:respiration"), Some(6);
        DepthStrider => Some("minecraft:depth_strider"), Some(7);
        AquaAffinity => Some("minecraft:aqua_affinity"), Some(8);
        Sharpness => Some("minecraft:sharpness"), Some(9);
        Smite => Some("minecraft:smite"), Some(10);
        BaneOfArthropods => Some("minecraft:bane_of_arthropods"), Some(11);
        Knockback => Some("minecraft:knockback"), Some(12);
        FireAspect => Some("minecraft:fire_aspect"), Some(13);
        Looting => Some("minecraft:looting"), Some(14);
        SweepingEdge => Some("minecraft:sweeping_edge"), None;
        Efficiency => Some("minecraft:efficiency"), Some(15);
        SilkTouch => Some("minecraft:silk_touch"), Some(16);
        Unbreaking => Some("minecraft:unbreaking"), Some(17);
        Fortune => Some("minecraft:fortune"), Some(18);
        Power => Some("minecraft:power"), Some(19);
        Punch => Some("minecraft:punch"), Some(20);
        Flame => Some("minecraft:flame"), Some(21);
        Infinity => Some("minecraft:infinity"), Some(22);
        LuckOfTheSea => Some("minecraft:luck_of_the_sea"), Some(23);
        Lure => Some("minecraft:lure"), Some(24);
        FrostWalker => Some("minecraft:frost_walker"), Some(25);
        Mending => Some("minecraft:mending"), Some(26);
        BindingCurse => Some("minecraft:binding_curse"), Some(27);
        VanishingCurse => Some("minecraft:vanishing_curse"), Some(28);
        Impaling => Some("minecraft:impaling"), Some(29);
        Riptide => Some("minecraft:riptide"), Some(30);
        Loyalty => Some("minecraft:loyalty"), Some(31);
        Channeling => Some("minecraft:channeling"), Some(32);
        Multishot => Some("minecraft:multishot"), Some(33);
        Piercing => Some("minecraft:piercing"), Some(34);
        QuickCharge => Some("minecraft:quick_charge"), Some(35);
        SoulSpeed => Some("minecraft:soul_speed"), Some(36);
        SwiftSneak => Some("minecraft:swift_sneak"), Some(37);
        WindBurst => Some("minecraft:wind_burst"), Some(38);
        Density => Some("minecraft:density"), Some(39);
        Breach => Some("minecraft:breach"), Some(40);
        Lunge => Some("minecraft:lunge"), Some(41);
    }
}

id_table! {
    /// Status effects (`ChunkerEffectType`).
    Effect: i32 {
        Empty => None, Some(0);
        Speed => Some("minecraft:speed"), Some(1);
        Slowness => Some("minecraft:slowness"), Some(2);
        Haste => Some("minecraft:haste"), Some(3);
        MiningFatigue => Some("minecraft:mining_fatigue"), Some(4);
        Strength => Some("minecraft:strength"), Some(5);
        InstantHealth => Some("minecraft:instant_health"), Some(6);
        InstantDamage => Some("minecraft:instant_damage"), Some(7);
        JumpBoost => Some("minecraft:jump_boost"), Some(8);
        Nausea => Some("minecraft:nausea"), Some(9);
        Regeneration => Some("minecraft:regeneration"), Some(10);
        Resistance => Some("minecraft:resistance"), Some(11);
        FireResistance => Some("minecraft:fire_resistance"), Some(12);
        WaterBreathing => Some("minecraft:water_breathing"), Some(13);
        Invisibility => Some("minecraft:invisibility"), Some(14);
        Blindness => Some("minecraft:blindness"), Some(15);
        NightVision => Some("minecraft:night_vision"), Some(16);
        Hunger => Some("minecraft:hunger"), Some(17);
        Weakness => Some("minecraft:weakness"), Some(18);
        Poison => Some("minecraft:poison"), Some(19);
        Wither => Some("minecraft:wither"), Some(20);
        HealthBoost => Some("minecraft:health_boost"), Some(21);
        Absorption => Some("minecraft:absorption"), Some(22);
        Saturation => Some("minecraft:saturation"), Some(23);
        Levitation => Some("minecraft:levitation"), Some(24);
        FatalPoison => None, Some(25);
        ConduitPower => Some("minecraft:conduit_power"), Some(26);
        SlowFalling => Some("minecraft:slow_falling"), Some(27);
        BadOmen => Some("minecraft:bad_omen"), Some(28);
        HeroOfTheVillage => Some("minecraft:hero_of_the_village"), Some(29);
        Darkness => Some("minecraft:darkness"), Some(30);
        TrialOmen => Some("minecraft:trial_omen"), Some(31);
        WindCharged => Some("minecraft:wind_charged"), Some(32);
        Weaving => Some("minecraft:weaving"), Some(33);
        Oozing => Some("minecraft:oozing"), Some(34);
        Infested => Some("minecraft:infested"), Some(35);
        RaidOmen => Some("minecraft:raid_omen"), Some(36);
        BreathOfTheNautilus => Some("minecraft:breath_of_the_nautilus"), Some(37);
        Luck => Some("minecraft:luck"), None;
        Unluck => Some("minecraft:unluck"), None;
        DolphinsGrace => Some("minecraft:dolphins_grace"), None;
    }
}

id_table! {
    /// Potion contents (`ChunkerPotionType`). On Bedrock the id is the item's data value.
    Potion: i16 {
        Water => Some("minecraft:water"), Some(0);
        Mundane => Some("minecraft:mundane"), Some(1);
        LongMundane => None, Some(2);
        Thick => Some("minecraft:thick"), Some(3);
        Awkward => Some("minecraft:awkward"), Some(4);
        NightVision => Some("minecraft:night_vision"), Some(5);
        LongNightVision => Some("minecraft:long_night_vision"), Some(6);
        Invisibility => Some("minecraft:invisibility"), Some(7);
        LongInvisibility => Some("minecraft:long_invisibility"), Some(8);
        Leaping => Some("minecraft:leaping"), Some(9);
        LongLeaping => Some("minecraft:long_leaping"), Some(10);
        StrongLeaping => Some("minecraft:strong_leaping"), Some(11);
        FireResistance => Some("minecraft:fire_resistance"), Some(12);
        LongFireResistance => Some("minecraft:long_fire_resistance"), Some(13);
        Swiftness => Some("minecraft:swiftness"), Some(14);
        LongSwiftness => Some("minecraft:long_swiftness"), Some(15);
        StrongSwiftness => Some("minecraft:strong_swiftness"), Some(16);
        Slowness => Some("minecraft:slowness"), Some(17);
        LongSlowness => Some("minecraft:long_slowness"), Some(18);
        WaterBreathing => Some("minecraft:water_breathing"), Some(19);
        LongWaterBreathing => Some("minecraft:long_water_breathing"), Some(20);
        Healing => Some("minecraft:healing"), Some(21);
        StrongHealing => Some("minecraft:strong_healing"), Some(22);
        Harming => Some("minecraft:harming"), Some(23);
        StrongHarming => Some("minecraft:strong_harming"), Some(24);
        Poison => Some("minecraft:poison"), Some(25);
        LongPoison => Some("minecraft:long_poison"), Some(26);
        StrongPoison => Some("minecraft:strong_poison"), Some(27);
        Regeneration => Some("minecraft:regeneration"), Some(28);
        LongRegeneration => Some("minecraft:long_regeneration"), Some(29);
        StrongRegeneration => Some("minecraft:strong_regeneration"), Some(30);
        Strength => Some("minecraft:strength"), Some(31);
        LongStrength => Some("minecraft:long_strength"), Some(32);
        StrongStrength => Some("minecraft:strong_strength"), Some(33);
        Weakness => Some("minecraft:weakness"), Some(34);
        LongWeakness => Some("minecraft:long_weakness"), Some(35);
        Wither => None, Some(36);
        TurtleMaster => Some("minecraft:turtle_master"), Some(37);
        LongTurtleMaster => Some("minecraft:long_turtle_master"), Some(38);
        StrongTurtleMaster => Some("minecraft:strong_turtle_master"), Some(39);
        SlowFalling => Some("minecraft:slow_falling"), Some(40);
        LongSlowFalling => Some("minecraft:long_slow_falling"), Some(41);
        StrongSlowness => Some("minecraft:strong_slowness"), Some(42);
        WindCharged => Some("minecraft:wind_charged"), Some(43);
        Weaving => Some("minecraft:weaving"), Some(44);
        Oozing => Some("minecraft:oozing"), Some(45);
        Infested => Some("minecraft:infested"), Some(46);
        Luck => Some("minecraft:luck"), None;
    }
}

id_table! {
    /// Goat horn instruments; on Bedrock the id is the item's data value.
    Horn: i16 {
        Ponder => Some("minecraft:ponder_goat_horn"), Some(0);
        Sing => Some("minecraft:sing_goat_horn"), Some(1);
        Seek => Some("minecraft:seek_goat_horn"), Some(2);
        Feel => Some("minecraft:feel_goat_horn"), Some(3);
        Admire => Some("minecraft:admire_goat_horn"), Some(4);
        Call => Some("minecraft:call_goat_horn"), Some(5);
        Yearn => Some("minecraft:yearn_goat_horn"), Some(6);
        Dream => Some("minecraft:dream_goat_horn"), Some(7);
    }
}

/// Entity ids Bedrock spells differently from Java, as (Bedrock id, Java id).
pub const ENTITY_RENAMES: [(&str, &str); 8] = [
    ("minecraft:villager_v2", "minecraft:villager"),
    ("minecraft:zombie_villager_v2", "minecraft:zombie_villager"),
    ("minecraft:xp_orb", "minecraft:experience_orb"),
    ("minecraft:ender_crystal", "minecraft:end_crystal"),
    ("minecraft:evocation_illager", "minecraft:evoker"),
    ("minecraft:vindication_illager", "minecraft:vindicator"),
    ("minecraft:zombie_pigman", "minecraft:zombified_piglin"),
    ("minecraft:eye_of_ender_signal", "minecraft:eye_of_ender"),
];

/// The Bedrock id of a Java entity: Chunker's type table, then the older ids below, and
/// otherwise the id as it is.
pub fn entity_java_to_bedrock(java: &str) -> &str {
    crate::entity::java_to_bedrock_type(java)
        .or_else(|| ENTITY_RENAMES.iter().find(|(_, j)| *j == java).map(|(b, _)| *b))
        .unwrap_or(java)
}

pub fn entity_bedrock_to_java(bedrock: &str) -> &str {
    crate::entity::bedrock_to_java_type(bedrock)
        .or_else(|| ENTITY_RENAMES.iter().find(|(b, _)| *b == bedrock).map(|(_, j)| *j))
        .unwrap_or(bedrock)
}

/// Banner patterns as (Java id without `minecraft:`, Bedrock short code), in Chunker's order.
pub const BANNER_PATTERNS: [(&str, &str); 43] = [
    ("base", "b"),
    ("square_bottom_left", "bl"),
    ("square_bottom_right", "br"),
    ("square_top_left", "tl"),
    ("square_top_right", "tr"),
    ("stripe_bottom", "bs"),
    ("stripe_top", "ts"),
    ("stripe_left", "ls"),
    ("stripe_right", "rs"),
    ("stripe_center", "cs"),
    ("stripe_middle", "ms"),
    ("stripe_downright", "drs"),
    ("stripe_downleft", "dls"),
    ("small_stripes", "ss"),
    ("cross", "cr"),
    ("straight_cross", "sc"),
    ("triangle_bottom", "bt"),
    ("triangle_top", "tt"),
    ("triangles_bottom", "bts"),
    ("triangles_top", "tts"),
    ("diagonal_left", "ld"),
    ("diagonal_up_right", "rd"),
    ("diagonal_up_left", "lud"),
    ("diagonal_right", "rud"),
    ("circle", "mc"),
    ("rhombus", "mr"),
    ("half_vertical", "vh"),
    ("half_horizontal", "hh"),
    ("half_vertical_right", "vhr"),
    ("half_horizontal_bottom", "hhb"),
    ("border", "bo"),
    ("curly_border", "cbo"),
    ("gradient", "gra"),
    ("gradient_up", "gru"),
    ("bricks", "bri"),
    ("globe", "glb"),
    ("creeper", "cre"),
    ("skull", "sku"),
    ("flower", "flo"),
    ("mojang", "moj"),
    ("piglin", "pig"),
    ("flow", "flw"),
    ("guster", "gus"),
];

/// Index of a banner pattern by Java id (`minecraft:` prefix optional).
pub fn banner_pattern_from_java(id: &str) -> Option<usize> {
    let id = id.strip_prefix("minecraft:").unwrap_or(id);
    BANNER_PATTERNS.iter().position(|(j, _)| *j == id)
}

pub fn banner_pattern_from_bedrock(code: &str) -> Option<usize> {
    BANNER_PATTERNS.iter().position(|(_, b)| *b == code)
}

/// Armor trim materials and patterns: Java names them `minecraft:<x>`, Bedrock `<x>`.
pub const TRIM_MATERIALS: [&str; 11] = [
    "amethyst", "copper", "diamond", "emerald", "gold", "iron", "lapis", "netherite", "quartz",
    "redstone", "resin",
];
pub const TRIM_PATTERNS: [&str; 18] = [
    "bolt", "coast", "dune", "eye", "flow", "host", "raiser", "rib", "sentry", "shaper",
    "silence", "snout", "spire", "tide", "vex", "ward", "wayfinder", "wild",
];

/// Firework explosion shapes; the Bedrock id is the index.
pub const FIREWORK_SHAPES: [&str; 5] = ["small_ball", "large_ball", "star", "creeper", "burst"];

/// Dye colours in Java's id order with (Bedrock RGB, Java RGB) (`ChunkerDyeColor`).
pub const DYES: [(&str, u32, u32); 16] = [
    ("white", 0xF0F0F0, 0xF0F0F0),
    ("orange", 0xF9801D, 0xEB8844),
    ("magenta", 0xC74EBD, 0xC354CD),
    ("light_blue", 0x3AB3DA, 0x6689D3),
    ("yellow", 0xFED83D, 0xDECF2A),
    ("lime", 0x80C71F, 0x41CD34),
    ("pink", 0xF38BAA, 0xD88198),
    ("gray", 0x474F52, 0x434343),
    ("light_gray", 0x9D9D97, 0xABABAB),
    ("cyan", 0x169C9C, 0x287697),
    ("purple", 0x8932B8, 0x7B2FBE),
    ("blue", 0x3C44AA, 0x253192),
    ("brown", 0x835432, 0x51301A),
    ("green", 0x5E7C16, 0x3B511A),
    ("red", 0xB02E26, 0xB3312C),
    ("black", 0x1D1D21, 0x1E1B1B),
];

/// The dye (Java id) closest to an RGB colour, comparing both editions' shades.
pub fn closest_dye(rgb: u32) -> usize {
    let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    let distance = |c: u32| {
        let d = |a: u32, b: u32| (a as i64 - b as i64).pow(2);
        d((c >> 16) & 255, r) + d((c >> 8) & 255, g) + d(c & 255, b)
    };
    (0..DYES.len())
        .min_by_key(|&i| distance(DYES[i].1).min(distance(DYES[i].2)))
        .unwrap_or(15)
}

/// Bedrock suspicious stew data values and the effect (with duration in ticks) each gives.
pub const STEWS: [(Effect, i32); 11] = [
    (Effect::NightVision, 100),
    (Effect::JumpBoost, 120),
    (Effect::Weakness, 180),
    (Effect::Blindness, 160),
    (Effect::Poison, 240),
    (Effect::Saturation, 7),
    (Effect::Saturation, 6),
    (Effect::FireResistance, 60),
    (Effect::Regeneration, 160),
    (Effect::Wither, 160),
    (Effect::NightVision, 80),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_one_to_one() {
        for e in Enchantment::ALL {
            if let Some(j) = e.java() {
                assert_eq!(Enchantment::from_java(j), Some(*e));
            }
            if let Some(b) = e.bedrock() {
                assert_eq!(Enchantment::from_bedrock(b), Some(*e));
            }
        }
        for p in Potion::ALL {
            if let Some(b) = p.bedrock() {
                assert_eq!(Potion::from_bedrock(b), Some(*p));
            }
        }
        assert_eq!(Enchantment::from_java("sharpness"), Some(Enchantment::Sharpness));
        assert_eq!(Enchantment::Lunge.bedrock(), Some(41));
    }

    #[test]
    fn closest_dye_matches_exact_shades() {
        for (i, (_, bedrock, java)) in DYES.iter().enumerate() {
            assert_eq!(closest_dye(*bedrock), i);
            assert_eq!(closest_dye(*java), i);
        }
    }
}

