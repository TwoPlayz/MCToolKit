# MCToolKit

A Minecraft world toolkit written in Rust: convert worlds between Java, Bedrock and Legacy
Console editions, prune chunks, edit NBT, browse a 2D map, and extend it with Lua scripts.

> **Early and untested.** World conversion has been checked against sample worlds on disk, but
> converted worlds have **not** been opened in Minecraft yet. Keep a backup of your world and
> expect missing or broken parts. Found a problem? Join the community on Discord:
> <https://discord.gg/y5mECmtban>

## What it does

| Tool | What you can do |
|---|---|
| **Converter** | Java 1.13+ (best: 26.3) ↔ Bedrock 1.26.60, and Legacy Console (Xbox 360, PS3) → Java. Also reads older Java worlds (before 1.13, with numeric block ids) and older Bedrock worlds (from 1.2.13) when converting. |
| **Pruner** | Remove chunks (or keep only some) by Box, Circle, Split, Polygon or Time layers, working on a copy and writing back with a backup. |
| **NBT Editor** | Open worlds, players, maps and single NBT files; typed tree with undo; chunk locator. |
| **Extensions** | Lua 5.4 scripts that style the NBT editor or run as world tools. Import your own from a `.zip` or folder (see [`extensions/README.md`](extensions/README.md)). |

### What the converter carries over

Blocks (including waterlogging), biomes, height maps, `level.dat`, block entities (chests and
other containers, signs, spawners, banners, beds, skulls, lecterns, brewing stands and the rest),
mobs and paintings, item frames, and the local player. Items convert with enchantments, names,
lore, books, potions and container contents.

Not converted: entity details beyond type, position, motion, rotation, health, age, name and a
few mob-specific fields (villager trades, owners, equipment, variants), dropped items, and
falling blocks. Entities such as chest minecarts keep their entity but not their contents. The
report at the end of a conversion lists what was left out.

How it was checked, on sample worlds only:

- Java 26.3 → Bedrock: all 258 million blocks of the sample match their mapping; Bedrock → Java
  matches all 2.3 million blocks of a Bedrock 1.26.60 world; a round trip returns 99.94 %
  identical (the rest is flowing liquid).
- Legacy Console → Java → Bedrock: 89 million blocks across a PS3 and an Xbox 360 save match, and
  all 80 containers and 388 item stacks in them match.
- Pruner: each shape selection and its inverse keep exactly the expected chunks, byte for byte.
- Older Bedrock worlds (`{name, val}` blocks and numeric ids) are covered by unit tests only.

Console limits: a modified PS3 `GAMEDATA` needs `PARAM.PFD` re-signing and a modified Xbox 360
`savegame.dat` needs re-packing into its STFS container before a console accepts them. Wii U
saves are untested (no sample).

## Build and run

You need a recent stable Rust toolchain (edition 2024).

```
cargo run --release -p mctk-gui             # the desktop app
cargo run --release -p mctk-cli -- --help   # the `mctoolkit` command-line tool
cargo test --workspace
```

Command-line examples:

```
mctoolkit convert java-to-bedrock "New World" out-bedrock
mctoolkit convert bedrock-to-java "My World.mcworld" out-java
mctoolkit convert console-to-java savegame.dat out-java
mctoolkit prune java world pruned --circle 0,0,500 --time ">=1200"
mctoolkit world info "path/to/world"
mctoolkit nbt dump level.dat
mctoolkit ext install my-extension.zip
mctoolkit ext run my-extension --world "New World" --output out
```

Settings and imported extensions are stored under `%APPDATA%\MCToolKit` (`~/.config/MCToolKit`
elsewhere).

## Crates

| Crate | Purpose |
|---|---|
| `mctk-nbt` | NBT codec (big/little endian), compression detection, SNBT |
| `mctk-anvil` | Java Anvil region files |
| `mctk-java` | Java world model: `level.dat`, dimensions, chunks of every era, block states |
| `mctk-leveldb` | Pure-Rust LevelDB reader and writer (Mojang's zlib/deflate blocks) |
| `mctk-bedrock` | Bedrock worlds: folders and `.mcworld`, chunk keys, subchunks, biomes |
| `mctk-console` | Legacy Console saves: Xbox 360 and PS3 archives, chunk formats |
| `mctk-mapping` | Block, biome, item and entity tables, including the pre-1.13 id tables |
| `mctk-chunker` | A Rust port of parts of Chunker (block entities, items, entities, item frames, text) |
| `mctk-convert` | Edition conversion on a multi-threaded staged pipeline |
| `mctk-prune` | The pruner |
| `mctk-world` | Edition detection and chunk listing |
| `mctk-lua` | Sandboxed Lua extension host |
| `mctk-cli` | The `mctoolkit` command-line tool |
| `mctk-gui` | The desktop app |

## Credits and data sources

MCToolKit builds on public data and open-source work; each source and its licence is listed in:

- [`crates/mctk-mapping/data/NOTICE.md`](crates/mctk-mapping/data/NOTICE.md): block, item and
  entity tables (GeyserMC, misode/mcmeta, Mojang bedrock-samples facts, PrismarineJS
  minecraft-data, pmmp/BedrockData).
- [`crates/mctk-chunker/LICENSE-CHUNKER`](crates/mctk-chunker/LICENSE-CHUNKER): Chunker by Hive
  Games (MIT), ported to Rust in `mctk-chunker`.
- [`crates/mctk-gui/assets/extensions/NOTICE.md`](crates/mctk-gui/assets/extensions/NOTICE.md)
  and [`assets/icons/NOTICE.md`](crates/mctk-gui/assets/icons/NOTICE.md): the built-in
  extensions and icons.

Minecraft is a trademark of Mojang Synergies AB. MCToolKit is not affiliated with Mojang or
Microsoft.

## Licence

MCToolKit is free software under the **GNU Lesser General Public License, version 3 or (at your
option) any later version** (`LGPL-3.0-or-later`). The licence texts are in
[`COPYING.LESSER`](COPYING.LESSER) and [`COPYING`](COPYING) (the GNU GPL v3, which the LGPL
builds on). In short: you can use, modify and share it; if you distribute a modified version of
the MCToolKit code, its source must be available under the same licence.

Third-party code and data keep their own licences, all compatible with the LGPL (MIT, BSD 2-Clause,
CC0, SIL OFL, CC BY 4.0); see the notices linked above. `mctk-chunker` stays under its MIT
licence because it is a port of Chunker.
