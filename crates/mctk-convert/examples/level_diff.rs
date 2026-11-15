//! Prints the level.dat entries in which two Bedrock worlds differ.
//!
//! cargo run --release -p mctk-convert --example level_diff -- <world a> <world b>
use mctk_bedrock::LevelDat;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    let read = |p: &str| -> Result<LevelDat, Box<dyn std::error::Error>> { Ok(LevelDat::parse(&std::fs::read(format!("{p}/level.dat"))?)?) };
    let (x, y) = (read(&a[1])?, read(&a[2])?);
    println!("storage version {} / {}", x.storage_version, y.storage_version);
    let (cx, cy) = (x.root.tag.as_compound().unwrap(), y.root.tag.as_compound().unwrap());
    for (k, v) in cx {
        match cy.get(k) {
            None => println!("only in a: {k} = {}", v.to_string().chars().take(100).collect::<String>()),
            Some(w) if w != v => println!("differs: {k}\n   a = {}\n   b = {}", v.to_string().chars().take(100).collect::<String>(), w.to_string().chars().take(100).collect::<String>()),
            _ => {}
        }
    }
    for (k, v) in cy {
        if !cx.contains_key(k) {
            println!("only in b: {k} = {}", v.to_string().chars().take(100).collect::<String>());
        }
    }
    Ok(())
}
