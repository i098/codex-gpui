// rust-embed reads assets/ at compile time; rebuild when an asset is added or changed.
fn main() {
    println!("cargo:rerun-if-changed=../../assets");
}
