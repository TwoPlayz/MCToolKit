# xnahost

Checks the Xbox 360 saves `mctk` writes against Microsoft's own LZX decoder.

`mctk-console` compresses Xbox 360 saves with its own LZX encoder (`lzx_enc.rs`) and reads them with the `lzxd`
crate. That only proves the two agree with each other. This tool loads the real `XnaNative.dll` that ships with
Universal Minecraft Tool (not included here; point it at your own copy) and runs *its* decompressor over
the same streams.

The DLL is 32-bit, so this is a separate 32-bit program, outside the workspace:

```
rustup target add i686-pc-windows-msvc
cargo build --release --target i686-pc-windows-msvc          # in tools/xnahost

# whole save: decompresses with XnaNative; with a reference file also compares byte for byte
cargo run -q --release -p mctk-console --example xmem_ref -- savegame.dat savegame.dat.ref
xnahost.exe "C:\path\to\XnaNative.dll" savegame.dat savegame.dat.ref

# every per-chunk stream
cargo run -q --release -p mctk-console --example xmem_chunks -- savegame.dat savegame.dat.chunks
xnahost.exe "C:\path\to\XnaNative.dll" savegame.dat.chunks
```

It calls two internal routines of build 4.0.30901.0 (md5 `cbffc669518ee511890f236fefffb4c1`) by offset, the
same ones UMT does: CreateDecompressionContext at `0x197a1d` and Decompress at `0x1979a1`. Another build of the
DLL has other offsets and will crash or misbehave. Result on 2026-11-21: UMT's own save and every stream
`mctk` wrote (whole saves and 10,900 chunk streams) decode identically.
