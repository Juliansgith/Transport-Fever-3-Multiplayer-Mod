//! Give Explorer, shortcuts and Windows Apps the same logo as the window.
//! Generate the ICO from the existing logo, so there is only one source asset.

use std::{env, fs::File, path::PathBuf};

use image::{
    ExtendedColorType,
    codecs::ico::{IcoEncoder, IcoFrame},
    imageops::FilterType,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=images/logo.png");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS")? != "windows" {
        tpf3mp_buildinfo::emit(tpf3mp_buildinfo::Artifact::Program, "Transport Fever 3 Multiplayer", "TPF3-MP.exe");
        return Ok(());
    }
    let logo = image::open("images/logo.png")?;
    let mut frames = Vec::new();
    for size in [16, 24, 32, 48, 64, 128, 256] {
        let pixels = logo
            .resize_exact(size, size, FilterType::Lanczos3)
            .to_rgba8();
        frames.push(IcoFrame::as_png(
            &pixels,
            size,
            size,
            ExtendedColorType::Rgba8,
        )?);
    }
    let path = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR missing")?).join("tpf3mp.ico");
    IcoEncoder::new(File::create(&path)?).encode_images(&frames)?;
    tpf3mp_buildinfo::emit_with_icon(
        tpf3mp_buildinfo::Artifact::Program,
        "Transport Fever 3 Multiplayer",
        "TPF3-MP.exe",
        Some(&path),
    );
    Ok(())
}
