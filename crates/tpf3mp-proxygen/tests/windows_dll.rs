//! End-to-end proxy generation against a real Windows system DLL. Windows-only;
//! on other platforms this test file compiles to nothing.

#![cfg(windows)]
#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use tpf3mp_proxygen::{parse_exports, write_proxy};

fn scratch(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "tpf3mp-proxygen-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

#[test]
fn generates_and_builds_a_forwarding_proxy() {
    // A small, always-present system DLL. Copy it so we never touch the original.
    let system_dll = PathBuf::from(r"C:\Windows\System32\version.dll");
    if !system_dll.exists() {
        eprintln!("skipping: {} not present", system_dll.display());
        return;
    }
    let root = scratch("build");
    fs::create_dir_all(&root).unwrap();

    let source_bytes = fs::read(&system_dll).unwrap();
    let exports = parse_exports(&source_bytes).unwrap();
    assert!(!exports.entries.is_empty(), "version.dll exports functions");
    let source_names: BTreeSet<String> = exports.named().map(|(n, _)| n.to_owned()).collect();
    assert!(source_names.contains("GetFileVersionInfoW"));

    let crate_dir = root.join("proxy");
    write_proxy(&crate_dir, &exports, "version", "version_real").unwrap();
    // The .def has a forwarder line per named export.
    let def = fs::read_to_string(crate_dir.join("version.def")).unwrap();
    for name in &source_names {
        assert!(
            def.contains(&format!("{name}=version_real.{name}")),
            "def missing forwarder for {name}"
        );
    }

    // Build the proxy in an isolated target dir. `env!("CARGO")` is the cargo
    // that is running this test.
    let target_dir = root.join("target");
    let build = Command::new(env!("CARGO"))
        .arg("build")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(crate_dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target_dir)
        .status();

    match build {
        Ok(status) if status.success() => {
            let dll = target_dir.join("debug").join("version.dll");
            assert!(dll.exists(), "the proxy DLL was built at {}", dll.display());
            let proxy_bytes = fs::read(&dll).unwrap();
            let proxy_exports = parse_exports(&proxy_bytes).unwrap();
            let proxy_names: BTreeSet<String> =
                proxy_exports.named().map(|(n, _)| n.to_owned()).collect();
            assert_eq!(
                proxy_names, source_names,
                "proxy exports match the original"
            );
            assert!(
                proxy_exports.entries.iter().all(|e| e.forwarder),
                "every proxy export is a forwarder"
            );
        }
        Ok(status) => panic!("proxy build failed with {status}"),
        Err(error) => eprintln!("skipping build check (could not run cargo: {error})"),
    }

    let _ = fs::remove_dir_all(&root);
}

/// A proxy built with `--load-hook` loads the hook from its own folder when
/// it is loaded, and still forwards to the renamed original.
#[test]
#[allow(unsafe_code)]
fn a_proxy_loads_the_hook_beside_it() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut core::ffi::c_void, flags: u32) -> isize;
        fn GetModuleHandleW(name: *const u16) -> isize;
    }
    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x8;
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let system = PathBuf::from(r"C:\Windows\System32");
    let (original, stand_in) = (system.join("version.dll"), system.join("msimg32.dll"));
    if !original.exists() || !stand_in.exists() {
        eprintln!("skipping: system DLLs not present");
        return;
    }
    let root = scratch("load");
    let crate_dir = root.join("proxy");
    let exports = parse_exports(&fs::read(&original).unwrap()).unwrap();
    // Unusual names, so nothing already loaded in this process answers.
    tpf3mp_proxygen::write_proxy_with(
        &crate_dir,
        &exports,
        "tpf3mpver",
        "tpf3mpver_real",
        Some("tpf3mp_marker_hook.dll"),
    )
    .unwrap();
    let target_dir = root.join("target");
    let build = Command::new(env!("CARGO"))
        .arg("build")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(crate_dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target_dir)
        .status();
    match build {
        Ok(status) if status.success() => {}
        Ok(status) => panic!("proxy build failed with {status}"),
        Err(error) => {
            eprintln!("skipping (could not run cargo: {error})");
            return;
        }
    }

    // A game folder: the proxy, the renamed original, and a stand-in hook.
    let game = root.join("game");
    fs::create_dir_all(&game).unwrap();
    let proxy = game.join("tpf3mpver.dll");
    fs::copy(target_dir.join("debug").join("tpf3mpver.dll"), &proxy).unwrap();
    fs::copy(&original, game.join("tpf3mpver_real.dll")).unwrap();
    fs::copy(&stand_in, game.join("tpf3mp_marker_hook.dll")).unwrap();

    let marker = wide("tpf3mp_marker_hook.dll");
    // SAFETY: NUL-terminated wide strings; the loaded modules stay loaded.
    unsafe {
        assert_eq!(GetModuleHandleW(marker.as_ptr()), 0, "not loaded yet");
        let path = wide(&proxy.to_string_lossy());
        let module = LoadLibraryExW(
            path.as_ptr(),
            std::ptr::null_mut(),
            LOAD_WITH_ALTERED_SEARCH_PATH,
        );
        assert_ne!(module, 0, "the proxy loads, its forwarders resolved");
        assert_ne!(
            GetModuleHandleW(marker.as_ptr()),
            0,
            "the proxy loaded the hook from its folder"
        );
    }
}
