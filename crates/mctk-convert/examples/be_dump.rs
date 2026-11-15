//! Prints every block entity of a Bedrock world.
//! cargo run --release -p mctk-convert --example be_dump -- <bedrock world>
use mctk_bedrock::{BedrockWorld, ChunkTag, Key};
use mctk_nbt::{Flavor, Tag};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bw = BedrockWorld::open(std::env::args().nth(1).ok_or("world")?)?;
    for (k, v) in &bw.db.entries {
        if let Key::Chunk(c) = Key::decode(k) {
            if c.tag == ChunkTag::BlockEntity {
                let mut s = v.as_slice();
                while !s.is_empty() {
                    let Ok(root) = mctk_nbt::read_root(&mut s, Flavor::Bedrock) else { break };
                    let _: &Tag = &root.tag;
                    println!("{:?}", root.tag);
                }
            }
        }
    }
    Ok(())
}