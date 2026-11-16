fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.rc");
    #[cfg(windows)]
    let _ = embed_resource::compile("assets/icon.rc", embed_resource::NONE);
}
