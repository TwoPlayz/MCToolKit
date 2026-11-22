//! The World Explorer as a world editor: changes kept in memory (shown at once in the 3D view,
//! each tool use one step to undo) until the world is saved, in every edition's own form.

pub mod blocks;
pub mod chunks;
pub mod layer;
pub mod schematic;
pub mod states;
pub mod tools;
pub mod transform;

pub use layer::EditLayer;

#[cfg(test)]
mod tests {
    use crate::workspace::{EditTool, Workspace};
    use crate::world_picker::UiEdition;

    /// The test world as made, as a Bedrock world and as a console save (by the converters), in
    /// a folder of their own: (label, path, edition) each.
    fn three_worlds(tag: &str) -> Option<(std::path::PathBuf, Vec<(&'static str, std::path::PathBuf, UiEdition)>)> {
        let src = std::path::PathBuf::from(std::env::var("MCTK_SNAPSHOT_WORLD").ok()?);
        let tmp = std::env::temp_dir().join(format!("mctk-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let java = tmp.join("java");
        for d in ["region", "entities"] {
            std::fs::create_dir_all(java.join(d)).unwrap();
            for f in std::fs::read_dir(src.join(d)).unwrap().flatten() {
                std::fs::copy(f.path(), java.join(d).join(f.file_name())).unwrap();
            }
        }
        std::fs::copy(src.join("level.dat"), java.join("level.dat")).unwrap();
        let bedrock = tmp.join("bedrock");
        mctk_convert::java_to_bedrock(&src, &bedrock).expect("to Bedrock");
        let console = tmp.join("savegame.dat");
        mctk_convert::java_to_console(&src, &console, mctk_console::Platform::Xbox360, &Default::default(), &Default::default(), &Default::default()).expect("to console");
        Some((tmp, vec![("Java", java, UiEdition::Java), ("Bedrock", bedrock, UiEdition::Bedrock), ("console", console, UiEdition::Xbox360)]))
    }

    /// The block states in a box as the tools see them now (through the edit layer).
    fn states(ws: &Workspace, b: ([i32; 3], [i32; 3])) -> crate::edit::tools::Clipboard {
        let edits = ws.map.edits.clone();
        let mut layer = edits.write().unwrap();
        let base = ws.edit_base("minecraft:overworld").unwrap();
        let ed = crate::edit::chunks::Editor { layer: &mut layer, base, dim: "minecraft:overworld".into(), notes: Vec::new() };
        super::tools::copy(&ed, b)
    }

    /// In each edition: the showcase saved as each kind of schematic and read back gives the same
    /// blocks (every one in the world's own kind; nearly every one in the others, which may lack
    /// a block), and one read back pastes into the world.
    #[test]
    fn schematics_export_import_and_paste_in_every_edition() {
        use super::schematic::Format;
        let Some((tmp, worlds)) = three_worlds("schem") else { return };
        let showcase = ([16, 63, 0], [31, 75, 15]);
        for (label, path, ed) in worlds {
            let we = ed.world();
            let mut ws = Workspace::open(&path, ed, &crate::settings::Settings::default(), false, false).expect("opens");
            let original = states(&ws, showcase);
            let own = match ed {
                UiEdition::Java => Format::Sponge,
                UiEdition::Bedrock => Format::McStructure,
                _ => Format::Legacy,
            };
            ws.map.sel_box = Some(showcase);
            for f in Format::ALL {
                let file = tmp.join(format!("{label}.{}", f.extension()));
                ws.export_selection(&file).unwrap_or_else(|e| panic!("{label} {f:?}: {e}"));
                ws.import_schematic(&file).unwrap_or_else(|e| panic!("{label} {f:?}: {e}"));
                let back = ws.map.clipboard.clone().unwrap();
                assert_eq!(back.size, original.size, "{label} {f:?}: size");
                let (mut same, mut all) = (0, 0);
                for y in 0..original.size[1] {
                    for z in 0..original.size[2] {
                        for x in 0..original.size[0] {
                            let a = original.get([x, y, z]);
                            if a.is_air() {
                                continue;
                            }
                            all += 1;
                            let b = back.get([x, y, z]);
                            same += usize::from(a.name == b.name);
                            if a.name != b.name && f == own {
                                println!("  {label} {f:?}: {a} came back as {b}");
                            }
                        }
                    }
                }
                println!("{label} as {f:?}: {same} of {all} blocks the same, {} block entities", back.block_entities.len());
                if f == own || (ed == UiEdition::Java && f == Format::Structure) {
                    assert_eq!(same, all, "{label} {f:?}: every block, in its own kind of file");
                } else {
                    assert!(same * 10 >= all * 9, "{label} {f:?}: nearly every block ({same} of {all})");
                }
            }
            // The world's own kind read back and pasted beside the showcase.
            ws.import_schematic(&tmp.join(format!("{label}.{}", own.extension()))).unwrap();
            let at = [16, 100, 0];
            ws.run_edit(EditTool::Paste { origin: at, place: Default::default(), with_air: true }).unwrap();
            let solid = |ws: &Workspace, b: ([i32; 3], [i32; 3])| crate::gpu_world::read_box(&ws.path, we, &ws.map.edits.read().unwrap(), b.0, b.1).0;
            let target = (at, [31, 112, 15]);
            assert!(solid(&ws, target) * 10 >= solid(&ws, showcase) * 9, "{label}: the pasted copy is there ({} of {})", solid(&ws, target), solid(&ws, showcase));
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// In each edition: the zoo's mobs go with Cut and Paste (moved: none left behind, all there,
    /// each with a new identity), and through the world's own kind of schematic; the brush paints
    /// a ball.
    #[test]
    fn entities_move_with_the_box_and_the_brush_paints_in_every_edition() {
        use super::schematic::Format;
        let Some((tmp, worlds)) = three_worlds("ents") else { return };
        let zoo = ([32, 0, 0], [47, 120, 15]);
        let there = [32, 0, 32];
        let moved = (there, [47, 120, 47]);
        for (label, path, ed) in worlds {
            let we = ed.world();
            let mut ws = Workspace::open(&path, ed, &crate::settings::Settings::default(), false, false).expect("opens");
            let read = |ws: &Workspace, b: ([i32; 3], [i32; 3])| crate::gpu_world::read_box(&ws.path, we, &ws.map.edits.read().unwrap(), b.0, b.1);
            let mobs = read(&ws, zoo).1;
            assert!(mobs >= 10, "{label}: the zoo ({mobs})");
            // Through the world's own kind of schematic, the mobs too.
            let own = match ed {
                UiEdition::Java => Format::Sponge,
                UiEdition::Bedrock => Format::McStructure,
                _ => Format::Legacy,
            };
            ws.map.sel_box = Some(zoo);
            let file = tmp.join(format!("{label}-zoo.{}", own.extension()));
            ws.export_selection(&file).unwrap();
            ws.import_schematic(&file).unwrap();
            assert_eq!(ws.map.clipboard.as_ref().unwrap().entities.len(), mobs, "{label}: the schematic holds the mobs");
            // Moved: cut here, pasted there.
            ws.copy_box(true).unwrap();
            assert_eq!(read(&ws, zoo).1, 0, "{label}: cut: none left");
            ws.run_edit(EditTool::Paste { origin: there, place: Default::default(), with_air: false }).unwrap();
            assert_eq!(read(&ws, moved).1, mobs, "{label}: pasted: all there");
            // Pasted again: copies, each its own.
            ws.run_edit(EditTool::Paste { origin: [there[0], there[1], there[2] + 16], place: super::transform::Placement { turns: 1, mirror: false }, with_air: false }).unwrap();
            let ids = |ws: &Workspace| {
                let edits = ws.map.edits.clone();
                let mut layer = edits.write().unwrap();
                let base = ws.edit_base("minecraft:overworld").unwrap();
                let e = crate::edit::chunks::Editor { layer: &mut layer, base, dim: "minecraft:overworld".into(), notes: Vec::new() };
                super::tools::entities_in(&e, (there, [47, 120, 63]))
                    .iter()
                    .map(|c| format!("{:?}{:?}{:?}", c.get("UUID"), c.get("UUIDMost"), c.get("UniqueID")))
                    .collect::<Vec<_>>()
            };
            let all = ids(&ws);
            assert_eq!(all.len(), mobs * 2, "{label}: two lots");
            // (The test world's mobs may have no id: the game gives those one.)
            let named: Vec<&String> = all.iter().filter(|s| *s != "NoneNoneNone").collect();
            let unique: std::collections::BTreeSet<&&String> = named.iter().collect();
            assert_eq!(unique.len(), named.len(), "{label}: every one its own identity");
            println!("{label}: {} of the copies carry an id, all different", named.len());
            // One of them selected in the 3D view (by where it is shown) and deleted.
            let shown = crate::gpu_world::entity_places(&ws.path, we, &ws.map.edits.read().unwrap(), 2, 2);
            let first = *shown.first().expect("mobs in the chunk pasted into");
            ws.run_edit(EditTool::DeleteEntity(first)).unwrap();
            let after = crate::gpu_world::entity_places(&ws.path, we, &ws.map.edits.read().unwrap(), 2, 2);
            assert_eq!(after.len(), shown.len() - 1, "{label}: the one selected is gone");
            // Undone: it is back (for what follows).
            ws.show_explorer = true;
            ws.undo_any().unwrap();
            ws.show_explorer = false;
            let back = crate::gpu_world::entity_places(&ws.path, we, &ws.map.edits.read().unwrap(), 2, 2);
            assert_eq!(back.len(), shown.len(), "{label}: undo brings it back");
            // The brush: a ball of glass up in the air.
            ws.run_edit(EditTool::Brush { center: [8, 220, 8], radius: 3, sphere: true, palette: vec![mctk_java::BlockState::new("minecraft:glass")], only: Default::default(), stroke: false }).unwrap();
            let ball = read(&ws, ([5, 217, 5], [11, 223, 11])).0;
            // A stroke (dragged): three balls, one step to undo.
            let line = ([0, 240, 0], [30, 242, 2]);
            for (i, x) in [3, 13, 23].into_iter().enumerate() {
                ws.run_edit(EditTool::Brush { center: [x, 241, 1], radius: 1, sphere: false, palette: vec![mctk_java::BlockState::new("minecraft:stone")], only: Default::default(), stroke: i > 0 }).unwrap();
            }
            assert_eq!(read(&ws, line).0, 81, "{label}: three cubes painted");
            ws.show_explorer = true;
            ws.undo_any().unwrap();
            ws.show_explorer = false;
            assert_eq!(read(&ws, line).0, 0, "{label}: one undo takes the whole stroke back");
            assert!(ball > 100 && ball < 343, "{label}: a ball, not a cube ({ball})");
            ws.save().unwrap();
            let ws = Workspace::open(&path, ed, &crate::settings::Settings::default(), false, false).expect("opens again");
            assert_eq!(read(&ws, moved).1, mobs, "{label}: saved: the mobs moved");
            assert_eq!(read(&ws, zoo).1, 0, "{label}: saved: none left");
            println!("{label}: {mobs} mobs moved and copied; ball of {ball}");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// In each edition: a box filled with gold, copied and pasted beside itself, the copy's gold
    /// replaced by diamond; a stair copied and pasted turned (facing east instead of north), and
    /// replaced by another stair that keeps its facing; all there at once, and after saving.
    #[test]
    fn fill_replace_copy_and_paste_in_every_edition() {
        let Some((tmp, worlds)) = three_worlds("fill") else { return };
        let gold = mctk_java::BlockState::new("minecraft:gold_block");
        let a = ([20, 200, 4], [23, 203, 7]);
        let b_at = [26, 200, 4];
        let b = (b_at, [29, 203, 7]);
        let stair_at = ([20, 210, 4], [20, 210, 4]);
        let turned = ([24, 210, 4], [24, 210, 4]);
        for (label, path, ed) in worlds {
            let we = ed.world();
            let mut ws = Workspace::open(&path, ed, &crate::settings::Settings::default(), false, false).expect("opens");
            let solid = |ws: &Workspace, bx: ([i32; 3], [i32; 3])| crate::gpu_world::read_box(&ws.path, we, &ws.map.edits.read().unwrap(), bx.0, bx.1).0;
            assert_eq!(solid(&ws, a), 0, "{label}: the sky is empty there");
            ws.map.sel_box = Some(a);
            ws.run_edit(EditTool::Fill(vec![gold.clone()])).unwrap();
            assert_eq!(solid(&ws, a), 64, "{label}: filled at once");
            ws.copy_box(false).unwrap();
            let place = super::transform::Placement::default();
            ws.run_edit(EditTool::Paste { origin: b_at, place, with_air: true }).unwrap();
            assert_eq!(solid(&ws, b), 64, "{label}: pasted");
            ws.map.sel_box = Some(b);
            ws.run_edit(EditTool::Replace { from: vec!["minecraft:gold_block".into()], to: vec![mctk_java::BlockState::new("minecraft:diamond_block")], keep: true }).unwrap();
            let names: Vec<String> = states(&ws, b).palette.iter().map(|s| s.name.clone()).collect();
            assert_eq!(names, vec!["minecraft:air".to_string(), "minecraft:diamond_block".to_string()], "{label}: replaced");

            // A stair facing north, pasted turned once: facing east; replaced by cobblestone stairs,
            // still facing east.
            let stair = crate::edit::states::default_state("minecraft:oak_stairs").with("facing", "north");
            ws.map.sel_box = Some(stair_at);
            ws.run_edit(EditTool::Fill(vec![stair])).unwrap();
            ws.copy_box(false).unwrap();
            ws.run_edit(EditTool::Paste { origin: turned.0, place: super::transform::Placement { turns: 1, mirror: false }, with_air: false }).unwrap();
            ws.map.sel_box = Some(turned);
            ws.run_edit(EditTool::Replace { from: vec!["minecraft:oak_stairs".into()], to: vec![crate::edit::states::default_state("minecraft:cobblestone_stairs")], keep: true }).unwrap();
            let got = states(&ws, turned).get([0, 0, 0]).clone();
            assert_eq!((got.name.as_str(), got.properties.get("facing").map(String::as_str)), ("minecraft:cobblestone_stairs", Some("east")), "{label}: turned and kept facing");

            ws.save().unwrap();
            let ws = Workspace::open(&path, ed, &crate::settings::Settings::default(), false, false).expect("opens again");
            assert_eq!(solid(&ws, a), 64, "{label}: saved: the gold");
            let names: Vec<String> = states(&ws, b).palette.iter().map(|s| s.name.clone()).collect();
            assert!(names.contains(&"minecraft:diamond_block".to_string()) && !names.contains(&"minecraft:gold_block".to_string()), "{label}: saved: the diamond ({names:?})");
            let got = states(&ws, turned).get([0, 0, 0]).clone();
            assert_eq!(got.properties.get("facing").map(String::as_str), Some("east"), "{label}: saved: the stair ({got:?})");
            println!("{label}: fill, copy, paste, replace and turn all held");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A Bedrock world with Optimize on Save: saving with nothing else changed writes its
    /// database again (and says how big it is now), the world opens again with the same blocks,
    /// and the setting is used up.
    #[test]
    fn optimize_on_save_rewrites_a_bedrock_database() {
        let Some((tmp, worlds)) = three_worlds("optimize") else { return };
        let (_, path, ed) = worlds.into_iter().find(|w| w.2 == UiEdition::Bedrock).expect("a Bedrock world");
        let mut ws = Workspace::open(&path, ed, &crate::settings::Settings::default(), false, false).expect("opens");
        let showcase = ([16, 63, 0], [31, 75, 15]);
        let before = crate::gpu_world::read_box(&ws.path, ed.world(), &ws.map.edits.read().unwrap(), showcase.0, showcase.1);
        assert!(!ws.dirty());
        ws.optimize_on_save = true;
        assert!(ws.dirty(), "something to save");
        let msg = ws.save().expect("saves");
        assert!(msg.starts_with("Optimized the database"), "{msg}");
        assert!(!ws.dirty() && !ws.optimize_on_save, "used up");
        let ws = Workspace::open(&path, ed, &crate::settings::Settings::default(), false, false).expect("opens again");
        let after = crate::gpu_world::read_box(&ws.path, ed.world(), &ws.map.edits.read().unwrap(), showcase.0, showcase.1);
        assert_eq!(before, after, "the same world");
        println!("{msg}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// In each edition, the right-click menu's actions: a stair turned faces the next way and
    /// stays a stair; one waterlogged is (where the edition has waterlogging); a block made
    /// another and deleted; a mob turned is still there, and one made another kind is that kind.
    #[test]
    fn the_right_click_actions_work_in_every_edition() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        let Some((tmp, worlds)) = three_worlds("menu") else { return };
        let at = [20, 220, 4];
        for (label, path, ed) in worlds {
            let we = ed.world();
            let mut ws = Workspace::open(&path, ed, &crate::settings::Settings::default(), false, false).expect("opens");
            let one = (at, at);
            let state = |ws: &Workspace| states(ws, one).get([0, 0, 0]).clone();
            ws.run_edit(EditTool::SetBlock(at, crate::edit::states::default_state("minecraft:oak_stairs").with("facing", "north"))).unwrap();
            ws.run_edit(EditTool::BlockProps(at, vec![("facing".into(), "east".into())])).unwrap();
            let s = state(&ws);
            assert_eq!((s.name.as_str(), s.properties.get("facing").map(String::as_str)), ("minecraft:oak_stairs", Some("east")), "{label}: turned");
            if ed != UiEdition::Xbox360 {
                ws.run_edit(EditTool::BlockProps(at, vec![("waterlogged".into(), "true".into())])).unwrap();
                assert_eq!(state(&ws).properties.get("waterlogged").map(String::as_str), Some("true"), "{label}: waterlogged");
            }
            ws.run_edit(EditTool::SetBlock(at, mctk_java::BlockState::new("minecraft:gold_block"))).unwrap();
            assert_eq!(state(&ws).name, "minecraft:gold_block", "{label}: changed");
            // Moved a block over: gone from where it was, there where it went.
            let there = [at[0] + 1, at[1], at[2]];
            ws.run_edit(EditTool::MoveBlock(at, there)).unwrap();
            assert!(state(&ws).name.ends_with("air"), "{label}: moved away");
            assert_eq!(states(&ws, (there, there)).get([0, 0, 0]).name, "minecraft:gold_block", "{label}: moved to");
            ws.run_edit(EditTool::SetBlock(there, mctk_java::BlockState::air())).unwrap();
            assert!(states(&ws, (there, there)).get([0, 0, 0]).name.ends_with("air"), "{label}: deleted");
            // A box moved: filled, copied, then cleared from where it was as it is placed elsewhere.
            let from = ([at[0], at[1] + 8, at[2]], [at[0] + 1, at[1] + 9, at[2] + 1]);
            let to = [at[0] + 6, at[1] + 8, at[2]];
            ws.map.sel_box = Some(from);
            ws.run_edit(EditTool::Fill(vec![mctk_java::BlockState::new("minecraft:gold_block")])).unwrap();
            ws.copy_box(false).unwrap();
            ws.run_edit(EditTool::MoveBox { from, origin: to, place: Default::default(), with_air: true }).unwrap();
            let solid = |ws: &Workspace, b: ([i32; 3], [i32; 3])| crate::gpu_world::read_box(&ws.path, we, &ws.map.edits.read().unwrap(), b.0, b.1).0;
            // A group of stairs: one property set on all of them, then the whole group turned (each
            // stair turned with it: east is south after a quarter turn clockwise).
            let sb = ([at[0], at[1] + 12, at[2]], [at[0] + 1, at[1] + 12, at[2]]);
            ws.map.sel_box = Some(sb);
            ws.run_edit(EditTool::Fill(vec![crate::edit::states::default_state("minecraft:oak_stairs").with("facing", "north")])).unwrap();
            ws.run_edit(EditTool::BoxProps { bounds: sb, props: vec![("facing".into(), "east".into())] }).unwrap();
            let facing = |ws: &Workspace, b: ([i32; 3], [i32; 3]), x: i32| states(ws, b).get([x, 0, 0]).properties.get("facing").cloned();
            assert_eq!((facing(&ws, sb, 0), facing(&ws, sb, 1)), (Some("east".into()), Some("east".into())), "{label}: every stair set");
            ws.run_edit(EditTool::TurnBox { from: sb, turns: 1, stroke: false }).unwrap();
            let turned = super::tools::turned_box(sb.0, sb.1, 1);
            let c = states(&ws, turned);
            assert!(c.get([0, 0, 0]).properties.get("facing").map(String::as_str) == Some("south") && c.get([0, 0, 1]).properties.get("facing").map(String::as_str) == Some("south"), "{label}: the group turned with its stairs");
            assert_eq!(solid(&ws, from), 0, "{label}: the box left where it was");
            assert_eq!(solid(&ws, (to, [to[0] + 1, to[1] + 1, to[2] + 1])), 8, "{label}: and is where it was put");

            let mobs = |ws: &Workspace| crate::gpu_world::entity_places(&ws.path, we, &ws.map.edits.read().unwrap(), 2, 0);
            let before = mobs(&ws);
            let find = |ws: &Workspace, q: &str| {
                let (d, t, s) = (AtomicUsize::new(0), AtomicUsize::new(0), AtomicBool::new(false));
                crate::gpu_world::search(&ws.path, we, "minecraft:overworld", &ws.map.edits, [40.0, f32::NAN, 8.0], 24.0, q, false, true, &d, &t, &s).unwrap()
            };
            let zombie = find(&ws, "minecraft:zombie");
            let p = zombie.first().expect("the zoo's zombie").pos;
            // Moved three blocks over, then turned to face a direction (free, not a quarter turn).
            let q = [p[0] + 3.0, p[1], p[2]];
            ws.run_edit(EditTool::MoveEntity(p, q)).unwrap();
            assert_eq!(mobs(&ws).len(), before.len(), "{label}: moved, not lost");
            let moved = find(&ws, "minecraft:zombie");
            assert!((moved.first().expect("still a zombie").pos[0] - q[0]).abs() < 0.01, "{label}: the zombie is where it was put");
            ws.run_edit(EditTool::SetYaw { at: q, yaw: 123.0, stroke: false }).unwrap();
            ws.run_edit(EditTool::SetYaw { at: q, yaw: 200.0, stroke: true }).unwrap();
            assert_eq!(mobs(&ws).len(), before.len(), "{label}: turned freely, not lost");
            if ed == UiEdition::Java {
                ws.run_edit(EditTool::Ride(q, "minecraft:chicken".into())).unwrap();
                assert_eq!(mobs(&ws).len(), before.len(), "{label}: a rider is part of its mount");
            }
            // A lingering potion cloud: placed in a Java world, and said not to be in the others.
            let said = ws.run_edit(EditTool::PlaceCloud { at: [q[0], q[1] + 1.0, q[2]], cloud: Default::default() }).unwrap();
            assert_eq!(said.starts_with("Placed a potion cloud"), ed == UiEdition::Java, "{label}: {said}");
            if ed == UiEdition::Java {
                // A mob and an item put down by hand: each is another entity in the world.
                let n = mobs(&ws).len();
                let said = ws.run_edit(EditTool::PlaceMob { at: [q[0] + 2.5, q[1], q[2]], id: "minecraft:pig".into() }).unwrap();
                assert!(said.starts_with("Placed"), "{label}: {said}");
                let said = ws.run_edit(EditTool::PlaceEntity { cell: [(q[0] + 3.5).floor() as i32, q[1].floor() as i32, q[2].floor() as i32], id: "minecraft:armor_stand".into(), facing: None }).unwrap();
                assert!(said.starts_with("Placed"), "{label}: {said}");
                assert!(mobs(&ws).len() > n, "{label}: the new ones are in the world");
                // An armor stand gets a helmet, an item frame its item.
                let stand = [(q[0] + 3.5).floor() + 0.5, q[1].floor(), (q[2]).floor() + 0.5];
                let said = ws.run_edit(EditTool::Change { at: stand, path: Vec::new(), change: crate::workspace::EntityChange::Equip(crate::mob_edit::Slot::Head, Some("minecraft:iron_helmet".into())) }).unwrap();
                assert!(said.starts_with("Changed"), "{label}: {said}");
                // The cloud changed: its settings are edited where it stands.
                let changed = crate::cloud::Cloud { radius: 6.0, ..Default::default() };
                let said = ws.run_edit(EditTool::UpdateCloud { at: [q[0], q[1] + 1.0, q[2]], cloud: changed, stroke: false }).unwrap();
                assert!(said.starts_with("Changed the potion cloud"), "{label}: {said}");
                // A mob put on another as its rider leaves the world's list to ride.
                let all = mobs(&ws);
                let others: Vec<[f64; 3]> = all.iter().copied().filter(|m| (m[0] - q[0]).abs() > 0.01 || (m[2] - q[2]).abs() > 0.01).collect();
                if let Some(rider) = others.first().copied() {
                    let said = ws.run_edit(EditTool::MountMob { mount: q, path: Vec::new(), rider }).unwrap();
                    assert!(said.starts_with("Put it on as a rider"), "{label}: {said}");
                    assert!(mobs(&ws).len() < all.len(), "{label}: the rider is saved with its mount");
                    // Unmounted, the riders stand in the world again.
                    let before = mobs(&ws).len();
                    let said = ws.run_edit(EditTool::Unmount { mount: q, path: Vec::new(), seat: None, delete: false }).unwrap();
                    assert!(said.starts_with("Took it off"), "{label}: {said}");
                    assert!(mobs(&ws).len() > before, "{label}: the riders are back in the world");
                }
            }
            let p = q;
            if matches!(ed, UiEdition::Java | UiEdition::Bedrock) {
                let creepers = find(&ws, "minecraft:creeper").len();
                ws.run_edit(EditTool::RetypeEntity(p, "minecraft:creeper".into())).unwrap();
                assert_eq!(find(&ws, "minecraft:creeper").len(), creepers + 1, "{label}: the zombie is a creeper");
                assert!(find(&ws, "minecraft:zombie").is_empty(), "{label}: and no zombie is left");
            }
            println!("{label}: the right-click actions held");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// In each edition: Search's Find gets the gold just filled in (not yet saved) within its
    /// range and none beyond, and the zoo's zombies (by name, mobs only) near them but not from
    /// far away.
    #[test]
    fn find_gets_blocks_and_mobs_within_range_in_every_edition() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        let Some((tmp, worlds)) = three_worlds("find") else { return };
        let a = ([20, 200, 4], [23, 203, 7]);
        for (label, path, ed) in worlds {
            let we = ed.world();
            let mut ws = Workspace::open(&path, ed, &crate::settings::Settings::default(), false, false).expect("opens");
            ws.map.sel_box = Some(a);
            ws.run_edit(EditTool::Fill(vec![mctk_java::BlockState::new("minecraft:gold_block")])).unwrap();
            let find = |center: [f32; 3], radius: f32, query: &str, blocks: bool, mobs: bool| {
                let (done, total, stop) = (AtomicUsize::new(0), AtomicUsize::new(0), AtomicBool::new(false));
                let found = crate::gpu_world::search(&ws.path, we, "minecraft:overworld", &ws.map.edits, center, radius, query, blocks, mobs, &done, &total, &stop).expect("searches");
                assert_eq!(done.into_inner(), total.into_inner(), "{label}: every chunk in range read");
                found
            };
            let gold = find([21.5, 201.5, 5.5], 16.0, "Gold Block", true, false);
            assert_eq!(gold.len(), 64, "{label}: all the gold, by its name with a space");
            assert!(gold.iter().all(|f| f.entity.is_none() && f.pos[1] >= 200.0), "{label}: only the gold filled");
            assert!(gold.windows(2).all(|w| w[0].dist <= w[1].dist), "{label}: nearest first");
            let near = find([21.5, 201.5, 5.5], 1.0, "gold_block", true, true);
            assert_eq!(near.len(), 7, "{label}: within one block, the middle one and its 6 sides");
            let zombies = find([40.0, f32::NAN, 8.0], 32.0, "zombie", false, true);
            let mut names: Vec<&str> = zombies.iter().map(|f| f.name.as_str()).collect();
            names.sort_unstable();
            assert_eq!(zombies.len(), 2, "{label}: the zombie and the zombie villager ({names:?})");
            assert!(zombies.iter().all(|f| f.entity.is_some()), "{label}: mobs, to select");
            assert!(find([400.0, f32::NAN, 400.0], 32.0, "zombie", true, true).is_empty(), "{label}: none from far away");
            println!("{label}: found {} gold and {names:?}", gold.len());
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// In each edition (the test world as made, and made a Bedrock world and a console save by
    /// the converters): a box of blocks cleared, a zoo's entities removed and a chunk deleted show
    /// at once in what the 3D view reads; undo brings them back and redo takes them again; after
    /// saving, the world opened afresh has them as edited.
    #[test]
    fn edits_show_at_once_undo_and_save_in_every_edition() {
        let Ok(src) = std::env::var("MCTK_SNAPSHOT_WORLD") else { return };
        let src = std::path::PathBuf::from(src);
        let tmp = std::env::temp_dir().join(format!("mctk-edits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // The Java copy: the files the world needs.
        let java = tmp.join("java");
        for d in ["region", "entities"] {
            std::fs::create_dir_all(java.join(d)).unwrap();
            for f in std::fs::read_dir(src.join(d)).unwrap().flatten() {
                std::fs::copy(f.path(), java.join(d).join(f.file_name())).unwrap();
            }
        }
        std::fs::copy(src.join("level.dat"), java.join("level.dat")).unwrap();
        let bedrock = tmp.join("bedrock");
        mctk_convert::java_to_bedrock(&src, &bedrock).expect("to Bedrock");
        let console = tmp.join("savegame.dat");
        mctk_convert::java_to_console(&src, &console, mctk_console::Platform::Xbox360, &Default::default(), &Default::default(), &Default::default()).expect("to console");

        // The showcase platform's chunk, the zoo's chunk, and a chunk to delete.
        let platform = ([16, 0, 0], [31, 255, 15]);
        let zoo = ([32, -64, 0], [47, 319, 15]);
        let doomed = (0, 1);
        for (label, path, ed) in [("Java", java.as_path(), UiEdition::Java), ("Bedrock", bedrock.as_path(), UiEdition::Bedrock), ("console", console.as_path(), UiEdition::Xbox360)] {
            let we = ed.world();
            let mut ws = Workspace::open(path, ed, &crate::settings::Settings::default(), false, false).expect("opens");
            let read = |ws: &Workspace, b: ([i32; 3], [i32; 3])| crate::gpu_world::read_box(&ws.path, we, &ws.map.edits.read().unwrap(), b.0, b.1);
            let doomed_box = ([doomed.0 * 16, 0, doomed.1 * 16], [doomed.0 * 16 + 15, 0, doomed.1 * 16 + 15]);
            let (solid, _, _) = read(&ws, platform);
            let (_, mobs, _) = read(&ws, zoo);
            assert!(read(&ws, doomed_box).2 == 1, "{label}: the chunk to delete is there");
            assert!(solid > 100 && mobs >= 10, "{label}: blocks ({solid}) and mobs ({mobs}) to start with");

            ws.map.sel_box = Some(platform);
            ws.run_edit(EditTool::DeleteBlocks).expect("clears");
            assert_eq!(read(&ws, platform).0, 0, "{label}: the box is empty at once");
            ws.map.sel_box = Some(zoo);
            ws.run_edit(EditTool::DeleteEntities).expect("removes");
            assert_eq!(read(&ws, zoo).1, 0, "{label}: the zoo is empty at once");
            ws.map.selected = vec![doomed];
            ws.run_edit(EditTool::DeleteChunks).expect("deletes");
            assert_eq!(read(&ws, doomed_box).2, 0, "{label}: the chunk is gone at once");
            assert!(ws.dirty(), "{label}: unsaved");

            // Undo all three (in the World Explorer), then redo them.
            ws.show_explorer = true;
            for _ in 0..3 {
                ws.undo_any().unwrap();
            }
            assert_eq!(read(&ws, platform).0, solid, "{label}: undo brings the blocks back");
            assert_eq!(read(&ws, zoo).1, mobs, "{label}: and the mobs");
            assert_eq!(read(&ws, doomed_box).2, 1, "{label}: and the chunk");
            for _ in 0..3 {
                ws.redo_any().unwrap();
            }
            assert_eq!(read(&ws, platform).0, 0, "{label}: redo clears them again");

            ws.save().expect("saves");
            assert!(!ws.has_edits(), "{label}: nothing left unsaved");
            let ws = Workspace::open(path, ed, &crate::settings::Settings::default(), false, false).expect("opens again");
            assert_eq!(read(&ws, platform).0, 0, "{label}: saved: the box is empty");
            assert_eq!(read(&ws, zoo).1, 0, "{label}: saved: no mobs");
            assert_eq!(read(&ws, doomed_box).2, 0, "{label}: saved: the chunk is gone");
            // The console games keep light as saved: the emptied chunk was lit again (full sky
            // light all the way down).
            if ed == UiEdition::Xbox360 {
                let s = ws.console.as_ref().unwrap();
                let entry = s.archive.regions().into_iter().find(|(d, x, z, _)| *d == 0 && *x == 0 && *z == 0).unwrap().3;
                let r = mctk_console::ConsoleRegion::parse(entry.data.clone()).unwrap();
                let root = r.raw_chunk(1, 0).unwrap().unwrap().nbt().unwrap();
                let Some(mctk_nbt::Tag::ByteArray(sky)) = root.tag.get("Level").and_then(|l| l.get("SkyLight")) else { panic!("no SkyLight") };
                let dark = sky.iter().filter(|b| **b as u8 != 0xff).count();
                assert_eq!(dark, 0, "console: the emptied chunk is all sky-lit ({dark} bytes not)");
            }
            println!("{label}: cleared {solid} blocks, removed {mobs} entities, deleted a chunk");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
