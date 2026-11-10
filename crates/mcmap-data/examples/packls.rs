//! List a version pack's files: `cargo run -p mcmap-data --example packls -- <pack> [filter]`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let p = mcmap_data::pack::RawPack::read(std::path::Path::new(&a[1])).unwrap();
    let f = a.get(2).cloned().unwrap_or_default();
    for (k, v) in &p.files {
        if k.contains(&f) {
            println!("{k}  {}", v.len());
        }
    }
}
