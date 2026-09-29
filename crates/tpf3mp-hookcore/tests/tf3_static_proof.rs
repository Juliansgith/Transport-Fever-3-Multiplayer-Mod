//! Static proof of the Transport Fever 3 release profile.
//!
//! The profile always parses and names its build; when the executable is on
//! disk, every target resolves uniquely at the address found on release day,
//! and a modified copy is refused. The executable is READ-ONLY: this test only
//! reads it, and skips when it is absent (in CI). Set `TPF3MP_TF3_EXE` to
//! point it at the game elsewhere.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use tpf3mp_hookcore::pe::PeHeaders;
use tpf3mp_hookcore::profile::{self, BuildIdentity, Profile};

const DEFAULT_EXE: &str = r"F:\SteamLibrary\steamapps\common\Transport Fever 3\TransportFever3.exe";
const PROFILE: &str = include_str!("../../../profiles/tf3_build40408_steam_windows.toml");

/// Addresses found on release day (RVAs, image base 0x140000000).
const TARGETS: &[(&str, u64)] = &[
    ("GameSim::Step", 0x159390),
    ("CGame::Step", 0x11f3b0),
    ("CGameTime::GetSpeed", 0x2a95a0),
    ("GameSim::Step/GetSpeed call", 0x1593ee),
    ("UI::CMenuUI::StartSavegame", 0x6a2880),
    ("UI::CMenuUI::CreatePage", 0x6a2ee0),
    ("CommandList::Add::lambda", 0x9d23c0),
];

#[test]
fn the_release_profile_parses_and_pins_build_40408() {
    let profile = Profile::from_toml(PROFILE).unwrap();
    assert_eq!(
        profile.build.sha256,
        "de1daad3a13f3b7e9f79903361bb43769cf4f15e59271a263aefe1f075f23ef2"
    );
    assert_eq!(profile.build.size, Some(69_711_288));
    assert_eq!(profile.targets.len(), TARGETS.len());
}

#[test]
fn every_target_resolves_uniquely_in_the_installed_game() {
    let exe = std::env::var_os("TPF3MP_TF3_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_EXE));
    if !exe.exists() {
        eprintln!(
            "skipping: {} is not present (expected in CI)",
            exe.display()
        );
        return;
    }
    let profile = Profile::from_toml(PROFILE).unwrap();
    let identity = BuildIdentity::of_file(&exe).unwrap();
    if profile.verify_identity(&identity).is_err() {
        eprintln!("skipping: {} is another build", exe.display());
        return;
    }
    let image = std::fs::read(&exe).unwrap();
    let pe = PeHeaders::parse(&image).unwrap();
    let text = pe.section(".text").expect("a .text section");
    let text_bytes = text.raw(&image).expect(".text raw bytes");
    let base = u64::from(text.virtual_address);

    let resolved = profile::resolve(&profile, text_bytes, base).unwrap();
    assert!(resolved.absent_optional.is_empty());
    for &(name, rva) in TARGETS {
        let target = resolved
            .get(name)
            .unwrap_or_else(|| panic!("{name} did not resolve"));
        assert_eq!(target.address, rva, "{name} resolved to the wrong RVA");
    }

    // A required target's bytes changed: resolution fails closed.
    let mut tampered = text_bytes.to_vec();
    let at = usize::try_from(0x159390 - base).unwrap();
    tampered[at] ^= 0xff;
    assert!(profile::resolve(&profile, &tampered, base).is_err());
}
