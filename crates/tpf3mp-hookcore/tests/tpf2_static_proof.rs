//! Static proof of the resolver against a real game binary.
//!
//! The profile always parses and refuses another build; when the executable
//! for exactly that build is on disk, every target resolves uniquely at its
//! known RVA, and a modified copy is refused. The executable is READ-ONLY:
//! this test only reads it, and skips when it is absent (in CI) or is another
//! build. Set `TPF3MP_TPF2_EXE` to point it at the game elsewhere.
//!
//! Scanning the on-disk file is a development convenience. It is valid here
//! because build 35924's `.text` is not encrypted (the SteamStub `.bind`
//! section leaves the code readable on disk), which this test's exact prologue
//! matches confirm. The production resolver scans the running process's mapped,
//! unpacked module image instead - the same [`resolve`] call, a different byte
//! source (see `docs/HOOKS.md`).

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use tpf3mp_hookcore::pe::PeHeaders;
use tpf3mp_hookcore::profile::{self, BuildIdentity, Profile, Refusal};

const DEFAULT_EXE: &str = r"E:\SteamLibrary\steamapps\common\Transport Fever 2\TransportFever2.exe";
const PROFILE: &str = include_str!("data/tpf2_build35924.toml");

/// Known function RVAs (image base 0x140000000).
const TARGETS: &[(&str, u64)] = &[
    ("GameSim::Step", 0x15aa00),
    ("CGame::Step", 0x118e90),
    ("CGameTime::GetSpeed", 0x2877a0),
    ("UI::CMenuUI::StartSavegame", 0x6785c0),
    ("UI::CMenuUI::CreatePage", 0x663370),
];

#[test]
fn the_pinned_profile_parses_and_refuses_another_build() {
    let profile = Profile::from_toml(PROFILE).unwrap();
    assert_eq!(profile.targets.len(), TARGETS.len());

    // A different build is refused up front, whatever it is.
    let stranger = BuildIdentity {
        sha256: "00".repeat(32),
        size: Some(72_843_280),
        pe_timestamp: Some(0x675A_BCC6),
    };
    assert!(matches!(
        profile.verify_identity(&stranger),
        Err(Refusal::UnknownBuild { .. })
    ));
}

#[test]
fn resolves_every_tpf2_target_uniquely_and_refuses_tampering() {
    let exe = std::env::var_os("TPF3MP_TPF2_EXE")
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
    // A Transport Fever 2 of another build proves nothing about this profile:
    // its signatures describe build 35924 and would not be expected to match.
    // The profile is read-only data, so skip rather than fail.
    if profile.verify_identity(&identity).is_err() {
        eprintln!("skipping: {} is another build", exe.display());
        return;
    }

    let image = std::fs::read(&exe).unwrap();

    // Resolve over the on-disk .text. `region_base` is the section's RVA, so a
    // resolved address is the function's RVA.
    let pe = PeHeaders::parse(&image).unwrap();
    let text = pe.section(".text").expect("a .text section");
    let text_bytes = text.raw(&image).expect(".text raw bytes");
    let base = u64::from(text.virtual_address);

    let resolved = profile::resolve(&profile, text_bytes, base).unwrap();
    assert_eq!(resolved.targets.len(), TARGETS.len());
    assert!(resolved.absent_optional.is_empty());
    for &(name, rva) in TARGETS {
        let target = resolved
            .get(name)
            .unwrap_or_else(|| panic!("{name} did not resolve"));
        assert_eq!(target.address, rva, "{name} resolved to the wrong RVA");
    }

    // Every signature matched exactly once: corrupting one target's bytes makes
    // that required target vanish, and resolution fails closed.
    let mut tampered = text_bytes.to_vec();
    let index = usize::try_from(0x15aa00u64 - base).unwrap(); // GameSim::Step
    tampered[index] ^= 0xFF;
    let refusal = profile::resolve(&profile, &tampered, base).unwrap_err();
    assert!(
        matches!(refusal, Refusal::Missing { ref target } if target == "GameSim::Step"),
        "expected a Missing refusal, got {refusal:?}"
    );
}
