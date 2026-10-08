//! Builds in the commit, the build time and, on Windows, the version
//! resource (`tpf3mp-buildinfo`).

#[path = "../tpf3mp-hookcore/src/bundle.rs"]
mod bundle;

fn main() {
    let manifest =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let profiles = manifest.join("../../profiles");
    println!(
        "cargo:rerun-if-changed={}",
        profiles.join("native-build.txt").display()
    );
    let selected = bundle::Bundle::selected(&profiles).expect("valid selected native bundle");
    println!("cargo:rerun-if-changed={}", selected.directory.display());
    println!("cargo:rustc-check-cfg=cfg(tpf3mp_native_savefast)");
    let native_source = std::fs::read_to_string(selected.directory.join("native.rs"))
        .expect("read selected native module");
    if native_source
        .lines()
        .any(|line| line.trim() == "pub mod savefast;")
    {
        assert!(
            selected.directory.join("savefast.rs").is_file(),
            "selected native module declares savefast without its data file"
        );
        println!("cargo:rustc-cfg=tpf3mp_native_savefast");
    }
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("output dir"));
    std::fs::write(
        output.join("native_bundle.rs"),
        selected.rust_module().expect("native module paths"),
    )
    .expect("write selected native module");
    tpf3mp_buildinfo::emit(
        tpf3mp_buildinfo::Artifact::Library,
        "TPF3-MP game hook",
        "tpf3mp_hook.dll",
    );
}
