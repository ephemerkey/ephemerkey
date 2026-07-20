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

    // `oled` = the generator drives the SSD1306. The 512-byte framebuffer does
    // not co-fit with the experimental hw-aes scratch in the 40 KB SRAM, so the
    // bench hw-aes build omits the display (bench AES bring-up needs no OLED).
    println!("cargo:rustc-check-cfg=cfg(oled)");
    let display = env::var_os("CARGO_FEATURE_DISPLAY").is_some();
    let hw_aes = env::var_os("CARGO_FEATURE_HW_AES").is_some();
    if display && !hw_aes {
        println!("cargo:rustc-cfg=oled");
    }
}
