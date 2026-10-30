//! Prints the Bedrock block for a Java state: `one <name> [k=v ...]`.
use mctk_java::BlockState;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut s = BlockState::new(a[0].clone());
    for kv in &a[1..] {
        let (k, v) = kv.split_once('=').unwrap();
        s = s.with(k, v);
    }
    let m = mctk_mapping::mappings();
    let b = m.java_to_bedrock(&s).unwrap();
    println!("{} {:?}", b.name, b.states);
}
