fn main() {
    // Provide memory.x to the linker.
    println!("cargo:rustc-link-search={}", std::env::current_dir().unwrap().display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");
}
