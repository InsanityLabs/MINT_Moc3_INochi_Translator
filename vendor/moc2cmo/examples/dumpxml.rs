use moc2cmo::{Decompiler, Texture};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let moc = std::fs::read(&a[1]).unwrap();
    let mut d = Decompiler::new();
    d.set_textures(a[3..].iter().map(|p| Texture::from_png(std::fs::read(p).unwrap()).unwrap()));
    let p = d.decompile_project(&moc).unwrap();
    std::fs::write(&a[2], p.main_xml()).unwrap();
}
