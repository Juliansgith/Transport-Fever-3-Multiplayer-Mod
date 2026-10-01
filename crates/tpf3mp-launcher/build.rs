//! Builds in the commit, the build time and, on Windows, the version
//! resource (`tpf3mp-buildinfo`), with the window's logo as the program's
//! icon, so Explorer, shortcuts and Windows Apps show the same logo as the
//! window. The ICO is made from the existing logo, so there is only one
//! source asset.

use std::{env, fs::File, path::PathBuf};

use image::{
    ExtendedColorType,
    codecs::ico::{IcoEncoder, IcoFrame},
    imageops::FilterType,
};

fn main() {
    println!("cargo:rerun-if-changed=images/logo.png");
    println!("cargo:rerun-if-changed=build.rs");
    let icon = if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        match icon() {
            Ok(path) => Some(path),
            Err(error) => panic!("making the launcher's icon failed: {error}"),
        }
    } else {
        None
    };
    tpf3mp_buildinfo::emit_with_icon(
        tpf3mp_buildinfo::Artifact::Program,
        "TPF3-MP launcher",
        "tpf3mp-launcher.exe",
        icon.as_deref(),
    );
}

/// The logo at every size Windows asks for, as one ICO in OUT_DIR.
fn icon() -> Result<PathBuf, Box<dyn std::error::Error>> {
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
    Ok(path)
}
