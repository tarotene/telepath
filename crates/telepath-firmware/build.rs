use std::{env, fs, path::PathBuf};

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy("linkme.x", out_dir.join("linkme.x")).expect("copy linkme.x to OUT_DIR");
    println!("cargo:rustc-link-search={}", out_dir.display());
    println!("cargo:rerun-if-changed=linkme.x");
    println!("cargo:rerun-if-changed=build.rs");
}
