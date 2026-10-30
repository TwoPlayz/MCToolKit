# Writing an extension

An extension is a folder with a `manifest.json` and a Lua 5.4 script (`main.lua` unless the
manifest sets `entry`).

## Adding one

- **In the app:** open the Extensions tab and press **Import from file**. Pick a `.zip` (such as
  a GitHub "Download ZIP" archive) or the extension's `manifest.json`. Imported extensions get a
  trash button to remove them; the built-in ones cannot be removed, only switched off.
- **From the command line:** `mctoolkit ext install <folder|zip>`, then
  `mctoolkit ext run <id> --world <world> --output <folder>`.

The extension's id is its folder name, made from the manifest `name` in lower case (spaces
become `-`). A name that matches a built-in extension is refused.

## Manifest

```json
{
  "name": "My Extension",
  "description": "What it does",
  "author": "you",
  "version": "1.0.0",
  "link": "https://github.com/you/your-extension",
  "editions": ["JAVA", "BEDROCK", "CONSOLE"],
  "extension_types": ["WORLD_TOOL"],
  "tags": ["statistics"]
}
```

`extension_types` is `WORLD_TOOL` (run on a world) or `NBT_EDITOR_STYLE` (styles the NBT
editor's tree). A `thumbnail.png` and a `LICENSE` file in the folder are shown on the card and
in More Info.

## A first script

The script may define `run(ctx)`; `ctx.world`, `ctx.output` and `ctx.edition` describe the run,
and the return value is shown as the result.

```lua
-- Prints the name of a world and the first Overworld chunks it has.
function run(ctx)
  assert(ctx.world, "choose a world first")
  local world = World.open(ctx.world)
  for i, c in ipairs(world:chunks("minecraft:overworld")) do
    if i > 3 then break end
    Print(string.format("chunk %d, %d", c.x, c.z))
  end
  return world:name()
end
```

## Sandbox

Scripts cannot use Lua's `io`, `os`, `package` or `debug` libraries, `dofile`, `loadfile` or
precompiled chunks. They read files only inside the extension folder, the chosen world and
the output folder, and write only inside the output folder. Worlds are opened read-only.
Each run stops after two minutes or 512 MiB of memory.

## API

| Name | Use |
|---|---|
| `BASE_PATH` | the extension's folder |
| `Print(...)` / `print(...)` | add a line to the run log |
| `UTF8ToPercentEncoding(s)` | percent-encode text for URLs |
| `EDITION`, `TYPE`, `JSON_TYPE`, `EXTENSION_TYPE` | enums (`EDITION.JAVA` = 0 … `EDITION.WIIU` = 4; `TYPE.BYTE` = 1 … `TYPE.LONG_ARRAY` = 12) |
| `NBT.read(path)` | returns a tag handle and `{flavor, compression}` |
| `NBT.write(path, tag, opts)` | `opts`: `flavor` (`"java"`/`"bedrock"`), `compression` (`"gzip"`/`"zlib"`/`"none"`), `header` (Bedrock `level.dat`) |
| `NBT.compound()`, `NBT.list(TYPE.x)` | new containers |
| tag methods | `type()`, `get(k)`, `set(k, v[, TYPE.x])`, `push(v[, TYPE.x])`, `remove(k)`, `keys()`, `len()`, `value()`, `copy()`, `tostring(tag)`; lists are 1-based |
| `World.open(path)` | `edition()`, `name()`, `dimensions()`, `chunks(dim)`, `block(dim, x, y, z)` → name, properties; `level()` → copy of `level.dat` |
| `Database.open(path)` | Bedrock LevelDB: `count()`, `keys([prefix])`, `get(key)` |
| `Files` | `read_text`, `write_text`, `exists`, `list` |
| `JSON` | `decode(text)`, `encode(value[, pretty])` |

Relative paths resolve against the output folder when writing and against `BASE_PATH` when
reading.

Extensions written for other tools' Lua APIs will not run unchanged: only the globals and
enums above exist here.
