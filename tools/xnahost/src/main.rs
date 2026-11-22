//! 32-bit host: runs XnaNative's XMem decompressor over a savegame.dat and compares with a reference.
//! xnahost <XnaNative.dll> <savegame.dat> [reference-decompressed-file]
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;

extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
}

type Create = unsafe extern "system" fn(u32, *const u32, u32, *mut *mut c_void) -> i32;
type Decompress = unsafe extern "system" fn(*mut c_void, *mut u8, *mut u32, *const u8, *mut u32) -> i32;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let wide: Vec<u16> = std::ffi::OsStr::new(&a[1]).encode_wide().chain(Some(0)).collect();
    let base = unsafe { LoadLibraryW(wide.as_ptr()) } as usize;
    assert!(base != 0, "LoadLibrary failed");
    let at = |rva: usize| base + rva - 0x1000_0000 + 0; // RVAs in the doc are relative to image base 0x10000000
    let create: Create = unsafe { std::mem::transmute(at(0x1000_0000 + 0x197a1d)) };
    let decompress: Decompress = unsafe { std::mem::transmute(at(0x1000_0000 + 0x1979a1)) };
    if a[2].ends_with(".chunks") {
        chunks(&a[2], create, decompress);
        return;
    }
    let file = std::fs::read(&a[2]).unwrap();
    let word = u32::from_be_bytes(file[0..4].try_into().unwrap()) as usize;
    let out_len = u32::from_be_bytes(file[8..12].try_into().unwrap()) as usize;
    let stream = &file[12..4 + word];
    println!("stream {} bytes, expect {} decompressed", stream.len(), out_len);
    let params = [1u32, 0x20000, 0x80000];
    let mut ctx: *mut c_void = std::ptr::null_mut();
    let r = unsafe { create(1, params.as_ptr(), 1, &mut ctx) };
    println!("create -> {r:#x} ctx {ctx:?}");
    let mut dest = vec![0u8; out_len + 65536];
    let mut dlen = out_len as u32;
    let mut slen = stream.len() as u32;
    let r = unsafe { decompress(ctx, dest.as_mut_ptr(), &mut dlen, stream.as_ptr(), &mut slen) };
    println!("decompress -> {r:#x}, produced {dlen}, consumed {slen}");
    dest.truncate(dlen as usize);
    if let Some(p) = a.get(3) {
        let reference = std::fs::read(p).unwrap();
        println!("matches reference: {}", reference == dest);
    }
    std::fs::write(format!("{}.xna.out", a[2]), &dest).unwrap();
}

fn chunks(path: &str, create: Create, decompress: Decompress) {
    let d = std::fs::read(path).unwrap();
    let (mut pos, mut ok, mut bad) = (0usize, 0usize, 0usize);
    while pos < d.len() {
        let sl = u32::from_le_bytes(d[pos..pos + 4].try_into().unwrap()) as usize;
        let il = u32::from_le_bytes(d[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let stream = &d[pos + 8..pos + 8 + sl];
        let inner = &d[pos + 8 + sl..pos + 8 + sl + il];
        pos += 8 + sl + il;
        let params = [1u32, 0x20000, 0x80000];
        let mut ctx: *mut c_void = std::ptr::null_mut();
        unsafe { create(1, params.as_ptr(), 1, &mut ctx) };
        let mut dest = vec![0u8; il + 65536];
        let (mut dlen, mut slen) = (il as u32, sl as u32);
        let r = unsafe { decompress(ctx, dest.as_mut_ptr(), &mut dlen, stream.as_ptr(), &mut slen) };
        if r == 0 && dest[..dlen as usize] == *inner { ok += 1 } else { bad += 1; if bad < 4 { println!("chunk at {pos}: r={r:#x} produced {dlen} of {il}") } }
    }
    println!("per-chunk streams: {ok} identical to lzxd, {bad} different");
}
