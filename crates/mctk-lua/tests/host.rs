use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mctk_lua::{Extension, RunOptions, run};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mctk-lua-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn extension(dir: &Path, script: &str, editions: &str) -> Extension {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        format!(
            r#"{{"name": "Test", "editions": [{editions}], "extension_types": ["WORLD_TOOL"]}}"#
        ),
    )
    .unwrap();
    std::fs::write(dir.join("main.lua"), script).unwrap();
    Extension::load(dir).unwrap()
}

fn eval(script: &str, options: &RunOptions) -> Result<Option<String>, String> {
    let dir = temp(&format!("ext{}", script.len()));
    let ext = extension(&dir.join("ext"), script, "");
    run(&ext, options)
        .map(|o| o.result)
        .map_err(|e| e.to_string())
}

#[test]
fn sandbox_has_no_ambient_file_or_process_access() {
    let r = eval(
        "function run() return tostring(io) .. tostring(os) .. tostring(package) .. tostring(debug) .. tostring(dofile) .. tostring(loadfile) end",
        &RunOptions::default(),
    );
    assert_eq!(r.unwrap().as_deref(), Some("nilnilnilnilnilnil"));
    // Precompiled chunks can break out of the Lua sandbox; only text loads.
    let r = eval(
        "function run() local f, err = load(string.dump(function() return 1 end)); return tostring(f) end",
        &RunOptions::default(),
    );
    assert_eq!(r.unwrap().as_deref(), Some("nil"));
    assert_eq!(
        eval(
            "function run() return load('return 2 + 3')() end",
            &RunOptions::default()
        )
        .unwrap()
        .as_deref(),
        Some("5")
    );
}

#[test]
fn paths_are_confined_to_the_run_folders() {
    let out = temp("confined-out");
    let options = RunOptions {
        output: Some(out.clone()),
        ..Default::default()
    };
    let outside = std::env::temp_dir().join("mctk-lua-outside.txt");
    std::fs::write(&outside, "secret").unwrap();
    let read_outside = eval(
        &format!(
            "function run() return Files.read_text([[{}]]) end",
            outside.display()
        ),
        &options,
    );
    assert!(read_outside.unwrap_err().contains("outside"));
    let dotdot = eval(
        "function run() Files.write_text('../x.txt', 'no') end",
        &options,
    );
    assert!(dotdot.unwrap_err().contains(".."));
    let write_outside = eval(
        &format!(
            "function run() Files.write_text([[{}]], 'x') end",
            outside.display()
        ),
        &options,
    );
    assert!(write_outside.unwrap_err().contains("outside"));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "secret");
    let no_output = eval(
        "function run() Files.write_text('a.txt', 'x') end",
        &RunOptions::default(),
    );
    assert!(no_output.is_err());
    eval(
        "function run() Files.write_text('sub/a.txt', 'hello') end",
        &options,
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(out.join("sub/a.txt")).unwrap(),
        "hello"
    );
}

#[test]
fn runaway_scripts_stop_at_the_time_limit() {
    let options = RunOptions {
        time_limit: Duration::from_millis(300),
        ..Default::default()
    };
    let r = eval("function run() while true do end end", &options);
    assert!(r.unwrap_err().contains("time limit"));
}

#[test]
fn nbt_api_builds_writes_and_reads_typed_tags() {
    let out = temp("nbt-out");
    let options = RunOptions {
        output: Some(out.clone()),
        ..Default::default()
    };
    let script = r#"
        function run(ctx)
          local root = NBT.compound()
          root:set("name", "Steve")
          root:set("level", 7, TYPE.SHORT)
          root:set("health", 19.5, TYPE.FLOAT)
          root:set("blocks", {1, 2, 3}, TYPE.INT_ARRAY)
          local list = NBT.list(TYPE.COMPOUND)
          local item = NBT.compound()
          item:set("id", "minecraft:stone")
          list:push(item)
          list:push(item)
          root:set("items", list)
          root:get("items"):get(2):set("id", "minecraft:dirt")
          NBT.write("data.dat", root)
          local back, info = NBT.read(ctx.output .. "/data.dat")
          assert(info.compression == "gzip" and info.flavor == "java")
          assert(back:get("level") == 7 and back:get("health") == 19.5)
          assert(back:get("blocks"):get(3) == 3 and back:get("blocks"):len() == 3)
          assert(back:get("items"):get(2):get("id") == "minecraft:dirt")
          assert(back:get("items"):get(1):get("id") == "minecraft:stone")
          back:remove("name")
          return back:get("level"):type() == nil or #back:keys()
        end"#;
    // `back:get("level")` is a number, so `:type()` would fail: check the error path too.
    let r = eval(script, &options);
    assert!(r.is_err());
    let script = script.replace(r#"back:get("level"):type() == nil or "#, "");
    assert_eq!(eval(&script, &options).unwrap().as_deref(), Some("4"));
    let (_, root) = mctk_nbt::read_file(out.join("data.dat"), mctk_nbt::Flavor::Java).unwrap();
    assert_eq!(root.tag.get("level"), Some(&mctk_nbt::Tag::Short(7)));
    assert_eq!(root.tag.get("health"), Some(&mctk_nbt::Tag::Float(19.5)));
}

#[test]
fn json_round_trips() {
    let r = eval(
        r#"function run()
             local t = JSON.decode('{"a": [1, 2, {"b": true}], "c": "x"}')
             assert(t.a[3].b == true and t.c == "x")
             return JSON.encode(t.a)
           end"#,
        &RunOptions::default(),
    );
    assert_eq!(r.unwrap().as_deref(), Some(r#"[1,2,{"b":true}]"#));
    assert_eq!(
        eval(
            "function run() return UTF8ToPercentEncoding('a b/é') end",
            &RunOptions::default()
        )
        .unwrap()
        .as_deref(),
        Some("a%20b%2F%C3%A9")
    );
}

#[test]
fn install_list_and_remove() {
    let root = temp("install");
    let installed = root.join("extensions");
    let src = root.join("src");
    extension(&src, "Print('hi')", "\"JAVA\"");
    let ext = mctk_lua::install_folder(&src, &installed).unwrap();
    assert_eq!(ext.id, "test");
    // A broken manifest is listed as an error, not skipped silently.
    std::fs::create_dir_all(installed.join("broken")).unwrap();
    std::fs::write(installed.join("broken/manifest.json"), "{").unwrap();
    let listed = mctk_lua::list(&installed);
    assert_eq!(listed.len(), 2);
    assert!(listed[0].is_err() && listed[1].as_ref().unwrap().id == "test");
    assert_eq!(
        run(&ext, &RunOptions::default()).unwrap().log,
        vec!["hi".to_string()]
    );
    mctk_lua::remove(&installed, "test").unwrap();
    assert!(mctk_lua::remove(&installed, "../src").is_err());

    // Zip with one top-level folder, as GitHub produces.
    let zip_path = root.join("ext.zip");
    let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
    let opts = zip::write::SimpleFileOptions::default();
    z.start_file("repo-main/manifest.json", opts).unwrap();
    z.write_all(br#"{"name": "Zipped Tool"}"#).unwrap();
    z.start_file("repo-main/main.lua", opts).unwrap();
    z.write_all(b"function run() return 'zip ok' end").unwrap();
    z.finish().unwrap();
    let ext = mctk_lua::install_zip(&zip_path, &installed).unwrap();
    assert_eq!(ext.id, "zipped-tool");
    assert_eq!(
        run(&ext, &RunOptions::default()).unwrap().result.as_deref(),
        Some("zip ok")
    );

    // Entries that climb out of the target are refused.
    let evil = root.join("evil.zip");
    let mut z = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap());
    z.start_file("../escape.lua", opts).unwrap();
    z.write_all(b"x").unwrap();
    z.finish().unwrap();
    assert!(mctk_lua::install_zip(&evil, &installed).is_err());
    assert!(!installed.parent().unwrap().join("escape.lua").exists());
}

const WORLDS: &str = r"C:\Users\JustinPC\Desktop\Worlds";

const CENSUS_MANIFEST: &str = r#"{"name":"Block Census","description":"Counts surface blocks","author":"test","version":"1.0.0","editions":["JAVA","BEDROCK","CONSOLE"],"extension_types":["WORLD_TOOL"]}"#;
const CENSUS_LUA: &str = r#"
local LIMIT = 16
local function surface(world, dim, x, z, top, bottom)
  for y = top, bottom, -1 do
    local name = world:block(dim, x, y, z)
    if name ~= nil and name ~= "minecraft:air" and name ~= "legacy:0" then return name end
  end
  return nil
end
function run(ctx)
  assert(ctx.world, "choose a world to run the census on")
  local world = World.open(ctx.world)
  local dim = "minecraft:overworld"
  local top, bottom = 319, -64
  if world:edition() >= EDITION.XBOX360 then top, bottom = 255, 0 end
  local counts, columns, used = {}, 0, 0
  for _, c in ipairs(world:chunks(dim)) do
    if used >= LIMIT then break end
    if surface(world, dim, c.x * 16 + 8, c.z * 16 + 8, top, bottom) then
      used = used + 1
      for dx = 0, 15 do
        for dz = 0, 15 do
          local name = surface(world, dim, c.x * 16 + dx, c.z * 16 + dz, top, bottom)
          if name then
            counts[name] = (counts[name] or 0) + 1
            columns = columns + 1
          end
        end
      end
    end
  end
  local names = {}
  for name in pairs(counts) do names[#names + 1] = name end
  table.sort(names, function(a, b) return counts[a] > counts[b] end)
  for i = 1, math.min(5, #names) do Print(string.format("%-40s %d", names[i], counts[names[i]])) end
  if ctx.output then
    Files.write_text("census.json", JSON.encode({ world = world:name(), columns = columns, blocks = counts }, true))
  end
  return string.format("%d columns, %d kinds of surface block", columns, #names)
end
"#;
const RENAME_MANIFEST: &str = r#"{"name":"Rename World","description":"Renames a copy of level.dat","author":"test","version":"1.0.0","editions":["JAVA"],"extension_types":["WORLD_TOOL"]}"#;
const RENAME_LUA: &str = r#"
function run(ctx)
  assert(ctx.world and ctx.output, "needs a world and an output folder")
  local level = World.open(ctx.world):level()
  local data = level:get("Data")
  local old = data:get("LevelName")
  data:set("LevelName", old .. " (renamed)")
  NBT.write("level.dat", level)
  local check = NBT.read(ctx.output .. "/level.dat")
  Print("new name: " .. check:get("Data"):get("LevelName"))
  return old
end
"#;

/// Writes a small extension into a temporary folder and loads it.
fn sample_extension(name: &str, manifest: &str, lua: &str) -> Extension {
    let dir = temp(&format!("sample-ext-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"), manifest).unwrap();
    std::fs::write(dir.join("main.lua"), lua).unwrap();
    Extension::load(&dir).unwrap()
}

/// Example extensions against the sample worlds (skipped when the samples are absent).
#[test]
fn example_extensions_run_on_sample_worlds() {
    let census = sample_extension("census", CENSUS_MANIFEST, CENSUS_LUA);
    for (rel, tag) in [
        (r"Java\New World", "java"),
        (r"BedrockPC\My World.mcworld", "bedrock"),
        (r"PS3\NPUB31419--131226152509", "ps3"),
    ] {
        let world = Path::new(WORLDS).join(rel);
        if !world.exists() {
            continue;
        }
        let out = temp(&format!("census-{tag}"));
        let r = run(
            &census,
            &RunOptions {
                world: Some(world),
                output: Some(out.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let summary = r.result.unwrap();
        assert!(summary.contains(" columns"), "{tag}: {summary}");
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("census.json")).unwrap())
                .unwrap();
        assert!(json["columns"].as_u64().unwrap() > 0, "{tag}: {json}");
        println!("{tag}: {summary}; top: {:?}", r.log.first());
    }

    let rename = sample_extension("rename", RENAME_MANIFEST, RENAME_LUA);
    let java = Path::new(WORLDS).join(r"Java\New World");
    if java.exists() {
        let out = temp("rename");
        let r = run(
            &rename,
            &RunOptions {
                world: Some(java.clone()),
                output: Some(out.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.result.as_deref(), Some("New World"));
        let (_, root) = mctk_nbt::read_file(out.join("level.dat"), mctk_nbt::Flavor::Java).unwrap();
        assert_eq!(
            root.tag
                .get("Data")
                .unwrap()
                .get("LevelName")
                .unwrap()
                .as_str(),
            Some("New World (renamed)")
        );
        // The source world is untouched.
        let (_, src) = mctk_nbt::read_file(java.join("level.dat"), mctk_nbt::Flavor::Java).unwrap();
        assert_eq!(
            src.tag
                .get("Data")
                .unwrap()
                .get("LevelName")
                .unwrap()
                .as_str(),
            Some("New World")
        );
    }
    let bedrock = Path::new(WORLDS).join(r"BedrockPC\My World.mcworld");
    if bedrock.exists() {
        let e = run(
            &rename,
            &RunOptions {
                world: Some(bedrock),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(e.to_string().contains("does not support BEDROCK"));
    }
}
