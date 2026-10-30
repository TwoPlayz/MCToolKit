//! Prints the Java biome → Bedrock biome id table (for comparing with other converters).
fn main() {
    let m = mctk_mapping::mappings();
    for name in m.java_biome_names() {
        println!("{name}\t{}", m.java_biome_to_bedrock(name).map_or("-".to_string(), |i| i.to_string()));
    }
}
