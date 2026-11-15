//! Builds a Java [`Registry`] from the data generator's reports
//! (`java -DbundlerMainClass=net.minecraft.data.Main -jar server.jar --reports`).

use indexmap::IndexMap;
use serde_json::Value;

use crate::{BlockDef, Edition, Error, Registry, bad};

/// `blocks_json` is `reports/blocks.json`, `registries_json` is `reports/registries.json`
/// and `biomes` the names under `data/minecraft/worldgen/biome` (biomes are data-driven, so
/// they are not in the registry report).
pub fn from_reports(
    version: &str,
    blocks_json: &str,
    registries_json: &str,
    biomes: Vec<String>,
) -> Result<Registry, Error> {
    let mut reg = Registry::new(Edition::Java, version);

    let blocks: Value = serde_json::from_str(blocks_json).map_err(|e| bad(e.to_string()))?;
    let blocks = blocks.as_object().ok_or_else(|| bad("blocks.json is not an object"))?;
    for (name, block) in blocks {
        let mut def = BlockDef {
            properties: IndexMap::new(),
            defaults: IndexMap::new(),
        };
        if let Some(props) = block.get("properties").and_then(Value::as_object) {
            for (k, vs) in props {
                let values = vs
                    .as_array()
                    .ok_or_else(|| bad(format!("{name}.{k} is not a list")))?
                    .iter()
                    .map(|v| v.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| bad(format!("{name}.{k} has a non-string value")))?;
                def.properties.insert(k.clone(), values);
            }
        }
        let states = block
            .get("states")
            .and_then(Value::as_array)
            .ok_or_else(|| bad(format!("{name} has no states")))?;
        if let Some(default) = states
            .iter()
            .find(|s| s.get("default").and_then(Value::as_bool) == Some(true))
        {
            // Keep declaration order of the properties, not the report's per-state order.
            for k in def.properties.keys() {
                if let Some(v) = default
                    .get("properties")
                    .and_then(|p| p.get(k))
                    .and_then(Value::as_str)
                {
                    def.defaults.insert(k.clone(), v.to_string());
                }
            }
        }
        reg.blocks.insert(name.clone(), def);
    }

    let regs: Value = serde_json::from_str(registries_json).map_err(|e| bad(e.to_string()))?;
    let entries = |key: &str| -> Vec<String> {
        regs.get(key)
            .and_then(|r| r.get("entries"))
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    };
    reg.items = entries("minecraft:item");
    reg.entities = entries("minecraft:entity_type");
    reg.block_entities = entries("minecraft:block_entity_type");
    reg.biomes = if biomes.is_empty() { entries("minecraft:biome") } else { biomes };
    // Up to 1.18 the structures are a registry; from 1.19 they are data files (see `mctk-gen`).
    reg.structures = entries("minecraft:worldgen/structure_feature");
    Ok(reg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BlockCheck;

    #[test]
    fn imports_blocks_defaults_and_registries() {
        let blocks = r#"{
          "minecraft:oak_log": {
            "properties": {"axis": ["x","y","z"]},
            "states": [
              {"id": 139, "properties": {"axis": "x"}},
              {"id": 140, "default": true, "properties": {"axis": "y"}},
              {"id": 141, "properties": {"axis": "z"}}
            ]
          },
          "minecraft:air": {"states": [{"default": true, "id": 0}]}
        }"#;
        let regs = r#"{
          "minecraft:item": {"entries": {"minecraft:stone": {"protocol_id": 1}}},
          "minecraft:entity_type": {"entries": {"minecraft:pig": {"protocol_id": 2}}},
          "minecraft:block_entity_type": {"entries": {"minecraft:chest": {"protocol_id": 3}}}
        }"#;
        let r = from_reports("t", blocks, regs, vec!["minecraft:plains".into()]).unwrap();
        assert_eq!(r.blocks.len(), 2);
        assert_eq!(r.default_state("minecraft:oak_log").unwrap()["axis"], "y");
        assert_eq!(r.check_block("minecraft:air", []), BlockCheck::Exact);
        assert!(r.has_item("minecraft:stone") && r.has_entity("minecraft:pig"));
        assert_eq!(r.block_entities, ["minecraft:chest"]);
        assert_eq!(r.biomes, ["minecraft:plains"]);
    }
}
