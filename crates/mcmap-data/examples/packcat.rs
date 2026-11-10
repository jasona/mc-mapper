//! Print one pack file: `cargo run -p mcmap-data --example packcat -- <pack> <path>`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let p = mcmap_data::pack::RawPack::read(std::path::Path::new(&a[1])).unwrap();
    for k in &a[2..] {
        println!("== {k}\n{}", p.files.get(k).map(String::as_str).unwrap_or("(missing)"));
    }
}
