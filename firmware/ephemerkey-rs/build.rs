use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // Put our memory.x (which reserves the store-journal pages) on the linker
    // search path. We supply it ourselves instead of using embassy's `memory-x`
    // feature precisely so we can shrink FLASH below the journal region.
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out.join("memory.x"), include_bytes!("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");

    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}
