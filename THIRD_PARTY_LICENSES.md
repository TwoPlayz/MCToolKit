# Third-party licenses

MCToolKit itself is licensed under the **GNU LGPL v3 or later** (see [`LICENSE`](LICENSE) and
[`COPYING`](COPYING)). It builds on the work below, each under its own licence, all compatible
with the LGPL. The same information is shown in the app under **Settings → About & Licenses**.

## Code, data and assets

| Work | Used for | Licence |
|---|---|---|
| [Chunker](https://github.com/HiveGamesOSS/Chunker) (© 2024 Hive Games) | Ported to Rust in `mctk-chunker`: block entities, items, entities, item frames, text. Its licence stays with that crate: [`LICENSE-CHUNKER`](crates/mctk-chunker/LICENSE-CHUNKER) | MIT |
| [GeyserMC/mappings](https://github.com/GeyserMC/mappings) and [Geyser](https://github.com/GeyserMC/Geyser) (© GeyserMC) | Java ↔ Bedrock block, biome and item tables | MIT |
| [PrismarineJS/minecraft-data](https://github.com/PrismarineJS/minecraft-data) | Numeric block and item ids from before Java 1.13 | MIT |
| [pmmp/BedrockData](https://github.com/pmmp/BedrockData) | Bedrock 1.12 block conversion table | CC0 1.0 |
| [misode/mcmeta](https://github.com/misode/mcmeta), [Mojang/bedrock-samples](https://github.com/Mojang/bedrock-samples) | Facts about the game (block properties, entity identifiers); no files copied | n/a |
| [Lato](https://www.latofonts.com) (© Łukasz Dziedzic) | The app's typeface ([`OFL.txt`](crates/mctk-gui/assets/fonts/OFL.txt)) | SIL OFL 1.1 |
| [Universal Minecraft Tool](https://universalminecrafttool.com) extensions | The six built-in NBT editor extensions, unchanged; each folder ships its own `LICENSE` | BSD 2-Clause |
| [Font Awesome Free](https://fontawesome.com/license/free) | Many of the general icons | CC BY 4.0 |

Per-file provenance is in [`crates/mctk-mapping/data/NOTICE.md`](crates/mctk-mapping/data/NOTICE.md),
[`crates/mctk-gui/assets/extensions/NOTICE.md`](crates/mctk-gui/assets/extensions/NOTICE.md) and
[`crates/mctk-gui/assets/icons/NOTICE.md`](crates/mctk-gui/assets/icons/NOTICE.md).

Minecraft is a trademark of Mojang Synergies AB. MCToolKit is not affiliated with Mojang or
Microsoft.

## Rust dependencies

Every crate in MCToolKit's build (from `Cargo.lock`, excluding dev-only crates), with the
licence it is published under. Where a crate offers a choice (`OR`), MCToolKit uses the
permissive option. Regenerate with `cargo metadata` when dependencies change.

| Crate | Version | License |
|---|---|---|
| [ab_glyph](https://github.com/alexheretic/ab-glyph) | 0.2.32 | Apache-2.0 |
| [ab_glyph_rasterizer](https://github.com/alexheretic/ab-glyph) | 0.1.10 | Apache-2.0 |
| [accesskit](https://github.com/AccessKit/accesskit) | 0.17.1 | MIT OR Apache-2.0 |
| [accesskit_atspi_common](https://github.com/AccessKit/accesskit) | 0.10.1 | MIT OR Apache-2.0 |
| [accesskit_consumer](https://github.com/AccessKit/accesskit) | 0.26.0 | MIT OR Apache-2.0 |
| [accesskit_macos](https://github.com/AccessKit/accesskit) | 0.18.1 | MIT OR Apache-2.0 |
| [accesskit_unix](https://github.com/AccessKit/accesskit) | 0.13.1 | MIT OR Apache-2.0 |
| [accesskit_windows](https://github.com/AccessKit/accesskit) | 0.24.1 | MIT OR Apache-2.0 |
| [accesskit_winit](https://github.com/AccessKit/accesskit) | 0.23.1 | Apache-2.0 |
| [adler2](https://github.com/oyvindln/adler2) | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| [ahash](https://github.com/tkaitchuck/ahash) | 0.8.12 | MIT OR Apache-2.0 |
| [android_system_properties](https://github.com/nical/android_system_properties) | 0.1.6 | MIT OR Apache-2.0 |
| [android-activity](https://github.com/rust-mobile/android-activity) | 0.6.1 | MIT OR Apache-2.0 |
| [android-properties](https://github.com/miklelappo/android-properties) | 0.2.2 | MIT |
| [anstream](https://github.com/rust-cli/anstyle.git) | 1.0.0 | MIT OR Apache-2.0 |
| [anstyle](https://github.com/rust-cli/anstyle.git) | 1.0.14 | MIT OR Apache-2.0 |
| [anstyle-parse](https://github.com/rust-cli/anstyle.git) | 1.0.0 | MIT OR Apache-2.0 |
| [anstyle-query](https://github.com/rust-cli/anstyle.git) | 1.1.5 | MIT OR Apache-2.0 |
| [anstyle-wincon](https://github.com/rust-cli/anstyle.git) | 3.0.11 | MIT OR Apache-2.0 |
| [arbitrary](https://github.com/rust-fuzz/arbitrary/) | 1.5.0 | MIT OR Apache-2.0 |
| [arboard](https://github.com/1Password/arboard) | 3.7.0 | MIT OR Apache-2.0 |
| [arrayref](https://github.com/droundy/arrayref) | 0.3.9 | BSD-2-Clause |
| [arrayvec](https://github.com/bluss/arrayvec) | 0.7.8 | MIT OR Apache-2.0 |
| [ash](https://github.com/ash-rs/ash) | 0.38.0+1.3.281 | MIT OR Apache-2.0 |
| [ashpd](https://github.com/bilelmoussaoui/ashpd) | 0.11.1 | MIT |
| [as-raw-xcb-connection](https://github.com/psychon/as-raw-xcb-connection) | 1.0.1 | MIT OR Apache-2.0 |
| [async-broadcast](https://github.com/smol-rs/async-broadcast) | 0.7.2 | MIT OR Apache-2.0 |
| [async-channel](https://github.com/smol-rs/async-channel) | 2.5.0 | Apache-2.0 OR MIT |
| [async-executor](https://github.com/smol-rs/async-executor) | 1.14.0 | Apache-2.0 OR MIT |
| [async-fs](https://github.com/smol-rs/async-fs) | 2.2.0 | Apache-2.0 OR MIT |
| [async-io](https://github.com/smol-rs/async-io) | 2.6.0 | Apache-2.0 OR MIT |
| [async-lock](https://github.com/smol-rs/async-lock) | 3.4.2 | Apache-2.0 OR MIT |
| [async-net](https://github.com/smol-rs/async-net) | 2.0.0 | Apache-2.0 OR MIT |
| [async-process](https://github.com/smol-rs/async-process) | 2.5.0 | Apache-2.0 OR MIT |
| [async-recursion](https://github.com/dcchut/async-recursion) | 1.2.0 | MIT OR Apache-2.0 |
| [async-signal](https://github.com/smol-rs/async-signal) | 0.2.14 | Apache-2.0 OR MIT |
| [async-task](https://github.com/smol-rs/async-task) | 4.7.1 | Apache-2.0 OR MIT |
| [async-trait](https://github.com/dtolnay/async-trait) | 0.1.92 | MIT OR Apache-2.0 |
| [atomic-waker](https://github.com/smol-rs/atomic-waker) | 1.1.2 | Apache-2.0 OR MIT |
| [atspi](https://github.com/odilia-app/atspi) | 0.22.0 | Apache-2.0 OR MIT |
| [atspi-common](https://github.com/odilia-app/atspi) | 0.6.0 | Apache-2.0 OR MIT |
| [atspi-connection](https://github.com/odilia-app/atspi/) | 0.6.0 | Apache-2.0 OR MIT |
| [atspi-proxies](https://github.com/odilia-app/atspi) | 0.6.0 | Apache-2.0 OR MIT |
| [autocfg](https://github.com/cuviper/autocfg) | 1.5.1 | Apache-2.0 OR MIT |
| [base64](https://github.com/marshallpierce/rust-base64) | 0.22.1 | MIT OR Apache-2.0 |
| [bitflags](https://github.com/bitflags/bitflags) | 1.3.2 | MIT/Apache-2.0 |
| [bitflags](https://github.com/bitflags/bitflags) | 2.13.2 | MIT OR Apache-2.0 |
| [bit-set](https://github.com/contain-rs/bit-set) | 0.8.0 | Apache-2.0 OR MIT |
| [bit-vec](https://github.com/contain-rs/bit-vec) | 0.8.0 | Apache-2.0 OR MIT |
| [block](http://github.com/SSheldon/rust-block) | 0.1.6 | MIT |
| [block2](https://github.com/madsmtm/objc2) | 0.5.1 | MIT |
| [block2](https://github.com/madsmtm/objc2) | 0.6.2 | MIT |
| [block-buffer](https://github.com/RustCrypto/utils) | 0.10.4 | MIT OR Apache-2.0 |
| [blocking](https://github.com/smol-rs/blocking) | 1.7.0 | Apache-2.0 OR MIT |
| [bstr](https://github.com/BurntSushi/bstr) | 1.13.1 | MIT OR Apache-2.0 |
| [bumpalo](https://github.com/fitzgen/bumpalo) | 3.20.3 | MIT OR Apache-2.0 |
| [bytemuck](https://github.com/Lokathor/bytemuck) | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| [bytemuck_derive](https://github.com/Lokathor/bytemuck) | 1.12.1 | Zlib OR Apache-2.0 OR MIT |
| [byteorder-lite](https://github.com/image-rs/byteorder-lite) | 0.1.0 | Unlicense OR MIT |
| [bytes](https://github.com/tokio-rs/bytes) | 1.12.1 | MIT |
| [calloop](https://github.com/Smithay/calloop) | 0.13.0 | MIT |
| [calloop](https://github.com/Smithay/calloop) | 0.14.5 | MIT |
| [calloop-wayland-source](https://github.com/smithay/calloop-wayland-source) | 0.3.0 | MIT |
| [calloop-wayland-source](https://github.com/smithay/calloop-wayland-source) | 0.4.1 | MIT |
| [cc](https://github.com/rust-lang/cc-rs) | 1.10.0 | MIT OR Apache-2.0 |
| [cfg_aliases](https://github.com/katharostech/cfg_aliases) | 0.2.2 | MIT |
| [cfg-if](https://github.com/rust-lang/cfg-if) | 1.0.5 | MIT OR Apache-2.0 |
| [cgl](https://github.com/servo/cgl-rs) | 0.3.2 | MIT / Apache-2.0 |
| [clap](https://github.com/clap-rs/clap) | 4.6.7 | MIT OR Apache-2.0 |
| [clap_builder](https://github.com/clap-rs/clap) | 4.6.7 | MIT OR Apache-2.0 |
| [clap_derive](https://github.com/clap-rs/clap) | 4.6.7 | MIT OR Apache-2.0 |
| [clap_lex](https://github.com/clap-rs/clap) | 1.1.1 | MIT OR Apache-2.0 |
| [clipboard-win](https://github.com/DoumanAsh/clipboard-win) | 5.4.3 | BSL-1.0 |
| [codespan-reporting](https://github.com/brendanzab/codespan) | 0.11.1 | Apache-2.0 |
| [colorchoice](https://github.com/rust-cli/anstyle.git) | 1.0.5 | MIT OR Apache-2.0 |
| [combine](https://github.com/Marwes/combine) | 4.6.9 | MIT |
| [concurrent-queue](https://github.com/smol-rs/concurrent-queue) | 2.5.0 | Apache-2.0 OR MIT |
| [core-foundation](https://github.com/servo/core-foundation-rs) | 0.9.4 | MIT OR Apache-2.0 |
| [core-foundation-sys](https://github.com/servo/core-foundation-rs) | 0.8.7 | MIT OR Apache-2.0 |
| [core-graphics](https://github.com/servo/core-foundation-rs) | 0.23.2 | MIT OR Apache-2.0 |
| [core-graphics-types](https://github.com/servo/core-foundation-rs) | 0.1.3 | MIT OR Apache-2.0 |
| [cpufeatures](https://github.com/RustCrypto/utils) | 0.2.17 | MIT OR Apache-2.0 |
| [crc32fast](https://github.com/srijs/rust-crc32fast) | 1.5.3 | MIT OR Apache-2.0 |
| [crossbeam-utils](https://github.com/crossbeam-rs/crossbeam) | 0.8.23 | MIT OR Apache-2.0 |
| [crunchy](https://github.com/eira-fransham/crunchy) | 0.2.4 | MIT |
| [crypto-common](https://github.com/RustCrypto/traits) | 0.1.7 | MIT OR Apache-2.0 |
| [cursor-icon](https://github.com/rust-windowing/cursor-icon) | 1.2.0 | MIT OR Apache-2.0 OR Zlib |
| [derive_arbitrary](https://github.com/rust-fuzz/arbitrary) | 1.5.0 | MIT OR Apache-2.0 |
| [digest](https://github.com/RustCrypto/traits) | 0.10.7 | MIT OR Apache-2.0 |
| [dispatch](http://github.com/SSheldon/rust-dispatch) | 0.2.0 | MIT |
| [dispatch2](https://github.com/madsmtm/objc2) | 0.2.0 | Zlib OR Apache-2.0 OR MIT |
| [dispatch2](https://github.com/madsmtm/objc2) | 0.3.1 | Zlib OR Apache-2.0 OR MIT |
| [displaydoc](https://github.com/yaahc/displaydoc) | 0.2.7 | MIT OR Apache-2.0 |
| [dlib](https://github.com/elinorbgr/dlib) | 0.5.3 | MIT |
| [document-features](https://github.com/slint-ui/document-features) | 0.2.12 | MIT OR Apache-2.0 |
| [downcast-rs](https://github.com/marcianx/downcast-rs) | 1.2.1 | MIT/Apache-2.0 |
| [dpi](https://github.com/rust-windowing/winit) | 0.1.2 | Apache-2.0 AND MIT |
| [ecolor](https://github.com/emilk/egui) | 0.31.1 | MIT OR Apache-2.0 |
| [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) | 0.31.1 | MIT OR Apache-2.0 |
| [egui](https://github.com/emilk/egui) | 0.31.1 | MIT OR Apache-2.0 |
| [egui_glow](https://github.com/emilk/egui/tree/master/crates/egui_glow) | 0.31.1 | MIT OR Apache-2.0 |
| [egui-wgpu](https://github.com/emilk/egui/tree/master/crates/egui-wgpu) | 0.31.1 | MIT OR Apache-2.0 |
| [egui-winit](https://github.com/emilk/egui/tree/master/crates/egui-winit) | 0.31.1 | MIT OR Apache-2.0 |
| [either](https://github.com/rayon-rs/either) | 1.19.0 | MIT OR Apache-2.0 |
| [emath](https://github.com/emilk/egui/tree/master/crates/emath) | 0.31.1 | MIT OR Apache-2.0 |
| [embed-resource](https://github.com/nabijaczleweli/rust-embed-resource) | 3.0.12 | MIT |
| [endi](https://github.com/zeenix/endi) | 1.1.1 | MIT |
| [enumflags2](https://github.com/meithecatte/enumflags2) | 0.7.12 | MIT OR Apache-2.0 |
| [enumflags2_derive](https://github.com/meithecatte/enumflags2) | 0.7.12 | MIT OR Apache-2.0 |
| [env_home](https://github.com/notpeter/env-home) | 0.1.0 | MIT OR Apache-2.0 |
| [epaint](https://github.com/emilk/egui/tree/master/crates/epaint) | 0.31.1 | MIT OR Apache-2.0 |
| [epaint_default_fonts](https://github.com/emilk/egui/tree/master/crates/epaint_default_fonts) | 0.31.1 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 |
| [equivalent](https://github.com/indexmap-rs/equivalent) | 1.0.3 | Apache-2.0 OR MIT |
| [errno](https://github.com/lambda-fairy/rust-errno) | 0.3.14 | MIT OR Apache-2.0 |
| [error-code](https://github.com/DoumanAsh/error-code) | 3.4.0 | BSL-1.0 |
| [event-listener](https://github.com/smol-rs/event-listener) | 5.4.2 | Apache-2.0 OR MIT |
| [event-listener-strategy](https://github.com/smol-rs/event-listener-strategy) | 0.5.4 | Apache-2.0 OR MIT |
| [fastrand](https://github.com/smol-rs/fastrand) | 2.5.0 | Apache-2.0 OR MIT |
| [fax](https://github.com/pdf-rs/fax) | 0.2.7 | MIT |
| [fdeflate](https://github.com/image-rs/fdeflate) | 0.3.7 | MIT OR Apache-2.0 |
| [find-msvc-tools](https://github.com/rust-lang/cc-rs) | 0.1.15 | MIT OR Apache-2.0 |
| [flate2](https://github.com/rust-lang/flate2-rs) | 1.1.10 | MIT OR Apache-2.0 |
| [foldhash](https://github.com/orlp/foldhash) | 0.1.5 | Zlib |
| [foreign-types](https://github.com/sfackler/foreign-types) | 0.5.0 | MIT/Apache-2.0 |
| [foreign-types-macros](https://github.com/sfackler/foreign-types) | 0.2.4 | MIT/Apache-2.0 |
| [foreign-types-shared](https://github.com/sfackler/foreign-types) | 0.3.1 | MIT/Apache-2.0 |
| [form_urlencoded](https://github.com/servo/rust-url) | 1.2.2 | MIT OR Apache-2.0 |
| [futures-channel](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-core](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-io](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-lite](https://github.com/smol-rs/futures-lite) | 2.6.1 | Apache-2.0 OR MIT |
| [futures-macro](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-sink](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-task](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [futures-util](https://github.com/rust-lang/futures-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [generic-array](https://github.com/fizyk20/generic-array.git) | 0.14.7 | MIT |
| [gethostname](https://codeberg.org/swsnr/gethostname.rs.git) | 1.1.0 | Apache-2.0 |
| [getrandom](https://github.com/rust-random/getrandom) | 0.2.17 | MIT OR Apache-2.0 |
| [getrandom](https://github.com/rust-random/getrandom) | 0.3.4 | MIT OR Apache-2.0 |
| [getrandom](https://github.com/rust-random/getrandom) | 0.4.3 | MIT OR Apache-2.0 |
| [gl_generator](https://github.com/brendanzab/gl-rs/) | 0.14.0 | Apache-2.0 |
| [glow](https://github.com/grovesNL/glow) | 0.16.0 | MIT OR Apache-2.0 OR Zlib |
| [glutin](https://github.com/rust-windowing/glutin) | 0.32.3 | Apache-2.0 |
| [glutin_egl_sys](https://github.com/rust-windowing/glutin) | 0.7.1 | Apache-2.0 |
| [glutin_glx_sys](https://github.com/rust-windowing/glutin) | 0.6.1 | Apache-2.0 |
| [glutin_wgl_sys](https://github.com/rust-windowing/glutin) | 0.6.1 | Apache-2.0 |
| [glutin-winit](https://github.com/rust-windowing/glutin) | 0.5.0 | MIT |
| [gpu-alloc](https://github.com/zakarumych/gpu-alloc) | 0.6.2 | MIT OR Apache-2.0 |
| [gpu-alloc-types](https://github.com/zakarumych/gpu-alloc) | 0.3.1 | MIT OR Apache-2.0 |
| [gpu-descriptor](https://github.com/zakarumych/gpu-descriptor) | 0.3.2 | MIT OR Apache-2.0 |
| [gpu-descriptor-types](https://github.com/zakarumych/gpu-descriptor) | 0.2.0 | MIT OR Apache-2.0 |
| [half](https://github.com/VoidStarKat/half-rs) | 2.7.1 | MIT OR Apache-2.0 |
| [hashbrown](https://github.com/rust-lang/hashbrown) | 0.15.5 | MIT OR Apache-2.0 |
| [hashbrown](https://github.com/rust-lang/hashbrown) | 0.17.1 | MIT OR Apache-2.0 |
| [heck](https://github.com/withoutboats/heck) | 0.5.0 | MIT OR Apache-2.0 |
| [hermit-abi](https://github.com/hermit-os/hermit-rs) | 0.5.3 | MIT OR Apache-2.0 |
| [hex](https://github.com/KokaKiwi/rust-hex) | 0.4.3 | MIT OR Apache-2.0 |
| [hexf-parse](https://github.com/lifthrasiir/hexf) | 0.2.1 | CC0-1.0 |
| [icu_collections](https://github.com/unicode-org/icu4x) | 2.4.0 | Unicode-3.0 |
| [icu_locale_core](https://github.com/unicode-org/icu4x) | 2.4.0 | Unicode-3.0 |
| [icu_normalizer](https://github.com/unicode-org/icu4x) | 2.4.0 | Unicode-3.0 |
| [icu_normalizer_data](https://github.com/unicode-org/icu4x) | 2.4.0 | Unicode-3.0 |
| [icu_properties](https://github.com/unicode-org/icu4x) | 2.4.0 | Unicode-3.0 |
| [icu_properties_data](https://github.com/unicode-org/icu4x) | 2.4.0 | Unicode-3.0 |
| [icu_provider](https://github.com/unicode-org/icu4x) | 2.4.0 | Unicode-3.0 |
| [idna](https://github.com/servo/rust-url/) | 1.1.0 | MIT OR Apache-2.0 |
| [idna_adapter](https://github.com/hsivonen/idna_adapter) | 1.2.2 | Apache-2.0 OR MIT |
| [image](https://github.com/image-rs/image) | 0.25.10 | MIT OR Apache-2.0 |
| [immutable-chunkmap](https://github.com/estokes/immutable-chunkmap) | 2.1.5 | Apache-2.0 OR MIT |
| [indexmap](https://github.com/indexmap-rs/indexmap) | 2.14.2 | Apache-2.0 OR MIT |
| [is_terminal_polyfill](https://github.com/polyfill-rs/is_terminal_polyfill) | 1.70.2 | MIT OR Apache-2.0 |
| [itoa](https://github.com/dtolnay/itoa) | 1.0.18 | MIT OR Apache-2.0 |
| [jni](https://github.com/jni-rs/jni-rs) | 0.22.4 | MIT OR Apache-2.0 |
| [jni-macros](https://github.com/jni-rs/jni-rs) | 0.22.4 | MIT OR Apache-2.0 |
| [jni-sys](https://github.com/jni-rs/jni-sys) | 0.3.1 | MIT OR Apache-2.0 |
| [jni-sys](https://github.com/jni-rs/jni-sys) | 0.4.1 | MIT OR Apache-2.0 |
| [jni-sys-macros](https://github.com/jni-rs/jni-sys) | 0.4.1 | MIT OR Apache-2.0 |
| [jobserver](https://github.com/rust-lang/jobserver-rs) | 0.1.35 | MIT OR Apache-2.0 |
| [js-sys](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys) | 0.3.106 | MIT OR Apache-2.0 |
| [khronos_api](https://github.com/brendanzab/gl-rs/) | 3.1.0 | Apache-2.0 |
| [khronos-egl](https://github.com/timothee-haudebourg/khronos-egl) | 6.0.0 | MIT/Apache-2.0 |
| [libc](https://github.com/rust-lang/libc) | 0.2.191 | MIT OR Apache-2.0 |
| [libloading](https://github.com/nagisa/rust_libloading/) | 0.8.9 | ISC |
| [libredox](https://gitlab.redox-os.org/redox-os/libredox.git) | 0.1.25 | MIT |
| [linux-raw-sys](https://github.com/sunfishcode/linux-raw-sys) | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| [linux-raw-sys](https://github.com/sunfishcode/linux-raw-sys) | 0.4.15 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| [litemap](https://github.com/unicode-org/icu4x) | 0.8.3 | Unicode-3.0 |
| [litrs](https://github.com/LukasKalbertodt/litrs) | 1.0.1 | MIT OR Apache-2.0 |
| [lock_api](https://github.com/Amanieu/parking_lot) | 0.4.14 | MIT OR Apache-2.0 |
| [log](https://github.com/rust-lang/log) | 0.4.34 | MIT OR Apache-2.0 |
| [luajit-src](https://github.com/khvzak/luajit-src-rs) | 210.5.12+a4f56a4 | MIT |
| [lua-src](https://github.com/khvzak/lua-src-rs) | 547.0.0 | MIT |
| [lzxd](https://github.com/Lonami/lzxd) | 0.2.7 | MIT OR Apache-2.0 |
| [malloc_buf](https://github.com/SSheldon/malloc_buf) | 0.0.6 | MIT |
| [memchr](https://github.com/BurntSushi/memchr) | 2.8.3 | Unlicense OR MIT |
| [memmap2](https://github.com/RazrFalcon/memmap2-rs) | 0.9.12 | MIT OR Apache-2.0 |
| [memoffset](https://github.com/Gilnaa/memoffset) | 0.9.1 | MIT |
| [metal](https://github.com/gfx-rs/metal-rs) | 0.31.0 | MIT OR Apache-2.0 |
| [miniz_oxide](https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide) | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| [miniz_oxide](https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide) | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| [mlua](https://github.com/mlua-rs/mlua) | 0.10.5 | MIT |
| [mlua-sys](https://github.com/mlua-rs/mlua) | 0.6.8 | MIT |
| [moxcms](https://github.com/awxkee/moxcms.git) | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| [naga](https://github.com/gfx-rs/wgpu/tree/trunk/naga) | 24.0.0 | MIT OR Apache-2.0 |
| [ndk](https://github.com/rust-mobile/ndk) | 0.9.0 | MIT OR Apache-2.0 |
| [ndk-context](https://github.com/rust-windowing/android-ndk-rs) | 0.1.1 | MIT OR Apache-2.0 |
| [ndk-sys](https://github.com/rust-mobile/ndk) | 0.5.0+25.2.9519653 | MIT OR Apache-2.0 |
| [ndk-sys](https://github.com/rust-mobile/ndk) | 0.6.0+11769913 | MIT OR Apache-2.0 |
| [nix](https://github.com/nix-rust/nix) | 0.29.0 | MIT |
| [nohash-hasher](https://github.com/paritytech/nohash-hasher) | 0.2.0 | Apache-2.0 OR MIT |
| [num_enum](https://github.com/illicitonion/num_enum) | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 |
| [num_enum_derive](https://github.com/illicitonion/num_enum) | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 |
| [num-traits](https://github.com/rust-num/num-traits) | 0.2.19 | MIT OR Apache-2.0 |
| [objc](http://github.com/SSheldon/rust-objc) | 0.2.7 | MIT |
| [objc2](https://github.com/madsmtm/objc2) | 0.5.3 | MIT |
| [objc2](https://github.com/madsmtm/objc2) | 0.6.5 | MIT |
| [objc2-app-kit](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-app-kit](https://github.com/madsmtm/objc2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| [objc2-cloud-kit](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-contacts](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-core-data](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-core-foundation](https://github.com/madsmtm/objc2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| [objc2-core-graphics](https://github.com/madsmtm/objc2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| [objc2-core-image](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-core-location](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-encode](https://github.com/madsmtm/objc2) | 4.1.0 | MIT |
| [objc2-foundation](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-foundation](https://github.com/madsmtm/objc2) | 0.3.2 | MIT |
| [objc2-io-surface](https://github.com/madsmtm/objc2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| [objc2-link-presentation](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-metal](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-quartz-core](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-symbols](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-ui-kit](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-uniform-type-identifiers](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc2-user-notifications](https://github.com/madsmtm/objc2) | 0.2.2 | MIT |
| [objc-sys](https://github.com/madsmtm/objc2) | 0.3.5 | MIT |
| [once_cell](https://github.com/matklad/once_cell) | 1.21.4 | MIT OR Apache-2.0 |
| [once_cell_polyfill](https://github.com/polyfill-rs/once_cell_polyfill) | 1.70.2 | MIT OR Apache-2.0 |
| [orbclient](https://gitlab.redox-os.org/redox-os/orbclient) | 0.3.55 | MIT |
| [ordered-float](https://github.com/reem/rust-ordered-float) | 4.6.0 | MIT |
| [ordered-stream](https://github.com/danieldg/ordered-stream) | 0.2.0 | MIT OR Apache-2.0 |
| [owned_ttf_parser](https://github.com/alexheretic/owned-ttf-parser) | 0.25.1 | Apache-2.0 |
| [parking](https://github.com/smol-rs/parking) | 2.2.1 | Apache-2.0 OR MIT |
| [parking_lot](https://github.com/Amanieu/parking_lot) | 0.12.5 | MIT OR Apache-2.0 |
| [parking_lot_core](https://github.com/Amanieu/parking_lot) | 0.9.12 | MIT OR Apache-2.0 |
| [paste](https://github.com/dtolnay/paste) | 1.0.15 | MIT OR Apache-2.0 |
| [percent-encoding](https://github.com/servo/rust-url/) | 2.3.2 | MIT OR Apache-2.0 |
| [pin-project](https://github.com/taiki-e/pin-project) | 1.1.13 | Apache-2.0 OR MIT |
| [pin-project-internal](https://github.com/taiki-e/pin-project) | 1.1.13 | Apache-2.0 OR MIT |
| [pin-project-lite](https://github.com/taiki-e/pin-project-lite) | 0.2.17 | Apache-2.0 OR MIT |
| [piper](https://github.com/smol-rs/piper) | 0.2.5 | MIT OR Apache-2.0 |
| [pkg-config](https://github.com/rust-lang/pkg-config-rs) | 0.3.34 | MIT OR Apache-2.0 |
| [plain](https://github.com/randomites/plain) | 0.2.3 | MIT/Apache-2.0 |
| [png](https://github.com/image-rs/image-png) | 0.17.16 | MIT OR Apache-2.0 |
| [png](https://github.com/image-rs/image-png) | 0.18.1 | MIT OR Apache-2.0 |
| [polling](https://github.com/smol-rs/polling) | 3.11.0 | Apache-2.0 OR MIT |
| [pollster](https://github.com/zesterer/pollster) | 0.4.0 | Apache-2.0/MIT |
| [potential_utf](https://github.com/unicode-org/icu4x) | 0.1.6 | Unicode-3.0 |
| [ppv-lite86](https://github.com/cryptocorrosion/cryptocorrosion) | 0.2.21 | MIT OR Apache-2.0 |
| [proc-macro2](https://github.com/dtolnay/proc-macro2) | 1.0.107 | MIT OR Apache-2.0 |
| [proc-macro-crate](https://github.com/bkchr/proc-macro-crate) | 3.5.0 | MIT OR Apache-2.0 |
| [profiling](https://github.com/aclysma/profiling) | 1.0.18 | MIT OR Apache-2.0 |
| [pxfm](https://github.com/awxkee/pxfm) | 0.1.30 | BSD-3-Clause OR Apache-2.0 |
| [quick-error](http://github.com/tailhook/quick-error) | 2.0.1 | MIT/Apache-2.0 |
| [quick-xml](https://github.com/tafia/quick-xml) | 0.30.0 | MIT |
| [quick-xml](https://github.com/tafia/quick-xml) | 0.41.0 | MIT |
| [quote](https://github.com/dtolnay/quote) | 1.0.47 | MIT OR Apache-2.0 |
| [rand](https://github.com/rust-random/rand) | 0.8.8 | MIT OR Apache-2.0 |
| [rand](https://github.com/rust-random/rand) | 0.9.5 | MIT OR Apache-2.0 |
| [rand_chacha](https://github.com/rust-random/rand) | 0.3.1 | MIT OR Apache-2.0 |
| [rand_chacha](https://github.com/rust-random/rand) | 0.9.0 | MIT OR Apache-2.0 |
| [rand_core](https://github.com/rust-random/rand) | 0.6.4 | MIT OR Apache-2.0 |
| [rand_core](https://github.com/rust-random/rand) | 0.9.5 | MIT OR Apache-2.0 |
| [raw-window-handle](https://github.com/rust-windowing/raw-window-handle) | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| [redox_syscall](https://gitlab.redox-os.org/redox-os/syscall) | 0.4.1 | MIT |
| [redox_syscall](https://gitlab.redox-os.org/redox-os/syscall) | 0.5.18 | MIT |
| [redox_syscall](https://gitlab.redox-os.org/redox-os/kernel) | 0.9.5 | MIT |
| [r-efi](https://github.com/r-efi/r-efi) | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| [r-efi](https://github.com/r-efi/r-efi) | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| [renderdoc-sys](https://github.com/ebkalderon/renderdoc-rs) | 1.1.0 | MIT OR Apache-2.0 |
| [rfd](https://github.com/PolyMeilex/rfd) | 0.15.3 | MIT |
| [ring](https://github.com/briansmith/ring) | 0.17.14 | Apache-2.0 AND ISC |
| [rustc_version](https://github.com/djc/rustc-version-rs) | 0.4.1 | MIT OR Apache-2.0 |
| [rustc-hash](https://github.com/rust-lang-nursery/rustc-hash) | 1.1.0 | Apache-2.0/MIT |
| [rustc-hash](https://github.com/rust-lang/rustc-hash) | 2.1.3 | Apache-2.0 OR MIT |
| [rustix](https://github.com/bytecodealliance/rustix) | 0.38.44 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| [rustix](https://github.com/bytecodealliance/rustix) | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| [rustls](https://github.com/rustls/rustls) | 0.23.46 | Apache-2.0 OR ISC OR MIT |
| [rustls-pki-types](https://github.com/rustls/pki-types) | 1.15.1 | MIT OR Apache-2.0 |
| [rustls-webpki](https://github.com/rustls/webpki) | 0.103.16 | ISC |
| [rustversion](https://github.com/dtolnay/rustversion) | 1.0.23 | MIT OR Apache-2.0 |
| [same-file](https://github.com/BurntSushi/same-file) | 1.0.6 | Unlicense/MIT |
| [scoped-tls](https://github.com/alexcrichton/scoped-tls) | 1.0.1 | MIT/Apache-2.0 |
| [scopeguard](https://github.com/bluss/scopeguard) | 1.2.0 | MIT OR Apache-2.0 |
| [sctk-adwaita](https://github.com/PolyMeilex/sctk-adwaita) | 0.10.1 | MIT |
| [semver](https://github.com/dtolnay/semver) | 1.0.28 | MIT OR Apache-2.0 |
| [serde](https://github.com/serde-rs/serde) | 1.0.229 | MIT OR Apache-2.0 |
| [serde_core](https://github.com/serde-rs/serde) | 1.0.229 | MIT OR Apache-2.0 |
| [serde_derive](https://github.com/serde-rs/serde) | 1.0.229 | MIT OR Apache-2.0 |
| [serde_json](https://github.com/serde-rs/json) | 1.0.154 | MIT OR Apache-2.0 |
| [serde_repr](https://github.com/dtolnay/serde-repr) | 0.1.21 | MIT OR Apache-2.0 |
| [serde_spanned](https://github.com/toml-rs/toml) | 1.1.2 | MIT OR Apache-2.0 |
| [sha1](https://github.com/RustCrypto/hashes) | 0.10.7 | MIT OR Apache-2.0 |
| [shlex](https://github.com/comex/rust-shlex) | 2.0.1 | MIT OR Apache-2.0 |
| [signal-hook-registry](https://github.com/vorner/signal-hook) | 1.4.8 | MIT OR Apache-2.0 |
| [simd_cesu8](https://github.com/seancroach/simd_cesu8) | 1.2.0 | Apache-2.0 OR MIT |
| [simd-adler32](https://github.com/mcountryman/simd-adler32) | 0.3.10 | MIT |
| [simdutf8](https://github.com/rusticstuff/simdutf8) | 0.1.5 | MIT OR Apache-2.0 |
| [slab](https://github.com/tokio-rs/slab) | 0.4.12 | MIT |
| [slotmap](https://github.com/orlp/slotmap) | 1.1.1 | Zlib |
| [smallvec](https://github.com/servo/rust-smallvec) | 1.16.4 | MIT OR Apache-2.0 |
| [smithay-client-toolkit](https://github.com/smithay/client-toolkit) | 0.19.2 | MIT |
| [smithay-client-toolkit](https://github.com/smithay/client-toolkit) | 0.20.0 | MIT |
| [smithay-clipboard](https://github.com/smithay/smithay-clipboard) | 0.7.3 | MIT |
| [smol_str](https://github.com/rust-analyzer/smol_str) | 0.2.2 | MIT OR Apache-2.0 |
| [spirv](https://github.com/gfx-rs/rspirv) | 0.3.0+sdk-1.3.268.0 | Apache-2.0 |
| [stable_deref_trait](https://github.com/storyyeller/stable_deref_trait) | 1.2.1 | MIT OR Apache-2.0 |
| [static_assertions](https://github.com/nvzqz/static-assertions-rs) | 1.1.0 | MIT OR Apache-2.0 |
| [strict-num](https://github.com/RazrFalcon/strict-num) | 0.1.1 | MIT |
| [strsim](https://github.com/rapidfuzz/strsim-rs) | 0.11.1 | MIT |
| [strum](https://github.com/Peternator7/strum) | 0.26.3 | MIT |
| [strum_macros](https://github.com/Peternator7/strum) | 0.26.4 | MIT |
| [subtle](https://github.com/dalek-cryptography/subtle) | 2.6.1 | BSD-3-Clause |
| [syn](https://github.com/dtolnay/syn) | 2.0.119 | MIT OR Apache-2.0 |
| [syn](https://github.com/dtolnay/syn) | 3.0.9 | MIT OR Apache-2.0 |
| [synstructure](https://github.com/mystor/synstructure) | 0.14.0 | MIT |
| [tempfile](https://github.com/Stebalien/tempfile) | 3.27.0 | MIT OR Apache-2.0 |
| [termcolor](https://github.com/BurntSushi/termcolor) | 1.4.1 | Unlicense OR MIT |
| [thiserror](https://github.com/dtolnay/thiserror) | 1.0.69 | MIT OR Apache-2.0 |
| [thiserror](https://github.com/dtolnay/thiserror) | 2.0.21 | MIT OR Apache-2.0 |
| [thiserror-impl](https://github.com/dtolnay/thiserror) | 1.0.69 | MIT OR Apache-2.0 |
| [thiserror-impl](https://github.com/dtolnay/thiserror) | 2.0.21 | MIT OR Apache-2.0 |
| [tiff](https://github.com/image-rs/image-tiff) | 0.11.3 | MIT |
| [tiny-skia](https://github.com/RazrFalcon/tiny-skia) | 0.11.4 | BSD-3-Clause |
| [tiny-skia-path](https://github.com/RazrFalcon/tiny-skia/tree/master/path) | 0.11.4 | BSD-3-Clause |
| [tinystr](https://github.com/unicode-org/icu4x) | 0.8.5 | Unicode-3.0 |
| [tokio](https://github.com/tokio-rs/tokio) | 1.54.1 | MIT |
| [toml](https://github.com/toml-rs/toml) | 1.1.8+spec-1.1.0 | MIT OR Apache-2.0 |
| [toml_datetime](https://github.com/toml-rs/toml) | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 |
| [toml_edit](https://github.com/toml-rs/toml) | 0.25.17+spec-1.1.0 | MIT OR Apache-2.0 |
| [toml_parser](https://github.com/toml-rs/toml) | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 |
| [toml_writer](https://github.com/toml-rs/toml) | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 |
| [tracing](https://github.com/tokio-rs/tracing) | 0.1.44 | MIT |
| [tracing-attributes](https://github.com/tokio-rs/tracing) | 0.1.31 | MIT |
| [tracing-core](https://github.com/tokio-rs/tracing) | 0.1.36 | MIT |
| [ttf-parser](https://github.com/harfbuzz/ttf-parser) | 0.25.1 | MIT OR Apache-2.0 |
| [type-map](https://github.com/kardeiz/type-map) | 0.5.1 | MIT/Apache-2.0 |
| [typenum](https://github.com/paholg/typenum) | 1.20.1 | MIT OR Apache-2.0 |
| [uds_windows](https://github.com/haraldh/rust_uds_windows) | 1.2.1 | MIT |
| [unicode-ident](https://github.com/dtolnay/unicode-ident) | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| [unicode-segmentation](https://github.com/unicode-rs/unicode-segmentation) | 1.13.3 | MIT OR Apache-2.0 |
| [unicode-width](https://github.com/unicode-rs/unicode-width) | 0.1.14 | MIT OR Apache-2.0 |
| [unicode-xid](https://github.com/unicode-rs/unicode-xid) | 0.2.6 | MIT OR Apache-2.0 |
| [untrusted](https://github.com/briansmith/untrusted) | 0.9.0 | ISC |
| [ureq](https://github.com/algesten/ureq) | 2.12.1 | MIT OR Apache-2.0 |
| [url](https://github.com/servo/rust-url) | 2.5.8 | MIT OR Apache-2.0 |
| [urlencoding](https://github.com/kornelski/rust_urlencoding) | 2.1.3 | MIT |
| [utf8_iter](https://github.com/hsivonen/utf8_iter) | 1.0.4 | Apache-2.0 OR MIT |
| [utf8parse](https://github.com/alacritty/vte) | 0.2.2 | Apache-2.0 OR MIT |
| [uuid](https://github.com/uuid-rs/uuid) | 1.29.0 | Apache-2.0 OR MIT |
| [version_check](https://github.com/SergioBenitez/version_check) | 0.9.5 | MIT/Apache-2.0 |
| [vswhom](https://github.com/nabijaczleweli/vswhom.rs) | 0.1.0 | MIT |
| [vswhom-sys](https://github.com/nabijaczleweli/vswhom-sys.rs) | 0.1.3 | MIT |
| [walkdir](https://github.com/BurntSushi/walkdir) | 2.5.0 | Unlicense/MIT |
| [wasi](https://github.com/bytecodealliance/wasi) | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| [wasip2](https://github.com/bytecodealliance/wasi-rs) | 1.0.4+wasi-0.2.12 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| [wasm-bindgen](https://github.com/wasm-bindgen/wasm-bindgen) | 0.2.129 | MIT OR Apache-2.0 |
| [wasm-bindgen-futures](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/futures) | 0.4.79 | MIT OR Apache-2.0 |
| [wasm-bindgen-macro](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro) | 0.2.129 | MIT OR Apache-2.0 |
| [wasm-bindgen-macro-support](https://github.com/wasm-bindgen/wasm-bindgen/tree/main/crates/macro-support) | 0.2.129 | MIT OR Apache-2.0 |
| [wasm-bindgen-shared](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared) | 0.2.129 | MIT OR Apache-2.0 |
| [wayland-backend](https://github.com/smithay/wayland-rs) | 0.3.17 | MIT |
| [wayland-client](https://github.com/smithay/wayland-rs) | 0.31.15 | MIT |
| [wayland-csd-frame](https://github.com/rust-windowing/wayland-csd-frame) | 0.3.0 | MIT |
| [wayland-cursor](https://github.com/smithay/wayland-rs) | 0.31.14 | MIT |
| [wayland-protocols](https://github.com/smithay/wayland-rs) | 0.32.13 | MIT |
| [wayland-protocols-experimental](https://github.com/smithay/wayland-rs) | 20250721.0.1 | MIT |
| [wayland-protocols-misc](https://github.com/smithay/wayland-rs) | 0.3.12 | MIT |
| [wayland-protocols-plasma](https://github.com/smithay/wayland-rs) | 0.3.12 | MIT |
| [wayland-protocols-wlr](https://github.com/smithay/wayland-rs) | 0.3.12 | MIT |
| [wayland-scanner](https://github.com/smithay/wayland-rs) | 0.31.11 | MIT |
| [wayland-sys](https://github.com/smithay/wayland-rs) | 0.31.11 | MIT |
| [webbrowser](https://github.com/amodm/webbrowser-rs) | 1.2.5 | MIT OR Apache-2.0 |
| [webpki-roots](https://github.com/rustls/webpki-roots) | 0.26.11 | CDLA-Permissive-2.0 |
| [webpki-roots](https://github.com/rustls/webpki-roots) | 1.0.11 | CDLA-Permissive-2.0 |
| [web-sys](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys) | 0.3.106 | MIT OR Apache-2.0 |
| [web-time](https://github.com/daxpedda/web-time) | 1.1.0 | MIT OR Apache-2.0 |
| [weezl](https://github.com/image-rs/weezl) | 0.1.12 | MIT OR Apache-2.0 |
| [wgpu](https://github.com/gfx-rs/wgpu) | 24.0.5 | MIT OR Apache-2.0 |
| [wgpu-core](https://github.com/gfx-rs/wgpu) | 24.0.5 | MIT OR Apache-2.0 |
| [wgpu-hal](https://github.com/gfx-rs/wgpu) | 24.0.4 | MIT OR Apache-2.0 |
| [wgpu-types](https://github.com/gfx-rs/wgpu) | 24.0.0 | MIT OR Apache-2.0 |
| [which](https://github.com/harryfei/which-rs.git) | 7.0.3 | MIT |
| [winapi](https://github.com/retep998/winapi-rs) | 0.3.9 | MIT/Apache-2.0 |
| [winapi-i686-pc-windows-gnu](https://github.com/retep998/winapi-rs) | 0.4.0 | MIT/Apache-2.0 |
| [winapi-util](https://github.com/BurntSushi/winapi-util) | 0.1.11 | Unlicense OR MIT |
| [winapi-x86_64-pc-windows-gnu](https://github.com/retep998/winapi-rs) | 0.4.0 | MIT/Apache-2.0 |
| [windows](https://github.com/microsoft/windows-rs) | 0.58.0 | MIT OR Apache-2.0 |
| [windows_aarch64_gnullvm](https://github.com/microsoft/windows-rs) | 0.52.6 | MIT OR Apache-2.0 |
| [windows_aarch64_msvc](https://github.com/microsoft/windows-rs) | 0.52.6 | MIT OR Apache-2.0 |
| [windows_i686_gnu](https://github.com/microsoft/windows-rs) | 0.52.6 | MIT OR Apache-2.0 |
| [windows_i686_gnullvm](https://github.com/microsoft/windows-rs) | 0.52.6 | MIT OR Apache-2.0 |
| [windows_i686_msvc](https://github.com/microsoft/windows-rs) | 0.52.6 | MIT OR Apache-2.0 |
| [windows_x86_64_gnu](https://github.com/microsoft/windows-rs) | 0.52.6 | MIT OR Apache-2.0 |
| [windows_x86_64_gnullvm](https://github.com/microsoft/windows-rs) | 0.52.6 | MIT OR Apache-2.0 |
| [windows_x86_64_msvc](https://github.com/microsoft/windows-rs) | 0.52.6 | MIT OR Apache-2.0 |
| [windows-core](https://github.com/microsoft/windows-rs) | 0.58.0 | MIT OR Apache-2.0 |
| [windows-implement](https://github.com/microsoft/windows-rs) | 0.58.0 | MIT OR Apache-2.0 |
| [windows-interface](https://github.com/microsoft/windows-rs) | 0.58.0 | MIT OR Apache-2.0 |
| [windows-link](https://github.com/microsoft/windows-rs) | 0.2.1 | MIT OR Apache-2.0 |
| [windows-result](https://github.com/microsoft/windows-rs) | 0.2.0 | MIT OR Apache-2.0 |
| [windows-strings](https://github.com/microsoft/windows-rs) | 0.1.0 | MIT OR Apache-2.0 |
| [windows-sys](https://github.com/microsoft/windows-rs) | 0.52.0 | MIT OR Apache-2.0 |
| [windows-sys](https://github.com/microsoft/windows-rs) | 0.59.0 | MIT OR Apache-2.0 |
| [windows-sys](https://github.com/microsoft/windows-rs) | 0.61.2 | MIT OR Apache-2.0 |
| [windows-targets](https://github.com/microsoft/windows-rs) | 0.52.6 | MIT OR Apache-2.0 |
| [winit](https://github.com/rust-windowing/winit) | 0.30.13 | Apache-2.0 |
| [winnow](https://github.com/winnow-rs/winnow) | 1.0.4 | MIT |
| [winreg](https://github.com/gentoo90/winreg-rs) | 0.56.0 | MIT |
| [winsafe](https://github.com/rodrigocfd/winsafe) | 0.0.19 | MIT |
| [wit-bindgen](https://github.com/bytecodealliance/wit-bindgen) | 0.57.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| [writeable](https://github.com/unicode-org/icu4x) | 0.6.4 | Unicode-3.0 |
| [x11-dl](https://github.com/AltF02/x11-rs.git) | 2.21.0 | MIT |
| [x11rb](https://github.com/psychon/x11rb) | 0.13.2 | MIT OR Apache-2.0 |
| [x11rb-protocol](https://github.com/psychon/x11rb) | 0.13.2 | MIT OR Apache-2.0 |
| [xcursor](https://github.com/esposm03/xcursor-rs) | 0.3.11 | MIT |
| [xdg-home](https://github.com/zeenix/xdg-home) | 1.3.0 | MIT |
| [xkbcommon-dl](https://github.com/rust-windowing/xkbcommon-dl) | 0.4.2 | MIT |
| [xkeysym](https://github.com/notgull/xkeysym) | 0.2.1 | MIT OR Apache-2.0 OR Zlib |
| [xml-rs](https://github.com/kornelski/xml-rs) | 0.8.29 | MIT |
| [yoke](https://github.com/unicode-org/icu4x) | 0.8.4 | Unicode-3.0 |
| [yoke-derive](https://github.com/unicode-org/icu4x) | 0.8.4 | Unicode-3.0 |
| [zbus](https://github.com/dbus2/zbus/) | 4.4.0 | MIT |
| [zbus](https://github.com/z-galaxy/zbus/) | 5.19.0 | MIT |
| [zbus_macros](https://github.com/dbus2/zbus/) | 4.4.0 | MIT |
| [zbus_macros](https://github.com/z-galaxy/zbus/) | 5.19.0 | MIT |
| [zbus_names](https://github.com/dbus2/zbus/) | 3.0.0 | MIT |
| [zbus_names](https://github.com/z-galaxy/zbus/) | 4.3.5 | MIT |
| [zbus_xml](https://github.com/dbus2/zbus/) | 4.0.0 | MIT |
| [zbus-lockstep](https://github.com/luukvanderduim/zbus-lockstep) | 0.4.4 | MIT |
| [zbus-lockstep-macros](https://github.com/luukvanderduim/zbus-lockstep) | 0.4.4 | MIT |
| [zcheapstr](https://github.com/z-galaxy/zcheapstr/) | 1.1.0 | MIT |
| [zerocopy](https://github.com/google/zerocopy) | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| [zerocopy-derive](https://github.com/google/zerocopy) | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| [zerofrom](https://github.com/unicode-org/icu4x) | 0.1.8 | Unicode-3.0 |
| [zerofrom-derive](https://github.com/unicode-org/icu4x) | 0.1.8 | Unicode-3.0 |
| [zeroize](https://github.com/RustCrypto/utils) | 1.9.1 | Apache-2.0 OR MIT |
| [zerotrie](https://github.com/unicode-org/icu4x) | 0.2.5 | Unicode-3.0 |
| [zerovec](https://github.com/unicode-org/icu4x) | 0.11.9 | Unicode-3.0 |
| [zerovec-derive](https://github.com/unicode-org/icu4x) | 0.11.7 | Unicode-3.0 |
| [zip](https://github.com/zip-rs/zip2.git) | 2.6.1 | MIT |
| [zlib-rs](https://github.com/trifectatechfoundation/zlib-rs) | 0.6.8 | Zlib |
| [zmij](https://github.com/dtolnay/zmij) | 1.0.23 | MIT |
| [zopfli](https://github.com/zopfli-rs/zopfli) | 0.8.4 | Apache-2.0 |
| [zune-core](https://github.com/etemesi254/zune-image) | 0.5.3 | MIT OR Apache-2.0 OR Zlib |
| [zune-jpeg](https://github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg) | 0.5.15 | MIT OR Apache-2.0 OR Zlib |
| [zvariant](https://github.com/dbus2/zbus/) | 4.2.0 | MIT |
| [zvariant](https://github.com/z-galaxy/zbus/) | 5.15.1 | MIT |
| [zvariant_derive](https://github.com/dbus2/zbus/) | 4.2.0 | MIT |
| [zvariant_derive](https://github.com/z-galaxy/zbus/) | 5.15.1 | MIT |
| [zvariant_utils](https://github.com/dbus2/zbus/) | 2.1.0 | MIT |
| [zvariant_utils](https://github.com/z-galaxy/zbus/) | 4.2.0 | MIT |
