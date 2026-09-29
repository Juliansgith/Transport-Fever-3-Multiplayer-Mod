//! Static proof of the resolver against the real Transport Fever 3 binary.
//!
//! Resolves the TF3 Steam build-40408 profile against the executable on disk
//! and checks that every target is found at its known RVA, uniquely, and that
//! a modified copy is refused. The executable is READ-ONLY: this test only
//! reads it. It is skipped when the file is absent (for example in CI, and on
//! any machine without the game).
//!
//! Scanning the on-disk file is a development convenience, valid here because
//! this build's `.text` is not encrypted (the SteamStub `.bind` section leaves
//! the code readable on disk), which the exact prologue matches confirm. The
//! production resolver scans the running process's mapped, unpacked module
//! image instead - the same [`resolve`] call, a different byte source (see
//! `docs/HOOKS.md`). The targets and RVAs come from the release-day recon
//! (`investigation/TPF3_RECON_2026-09-29.md`); they are proven to resolve, not
//! yet measured in-game.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use tpf3mp_hookcore::pe::PeHeaders;
use tpf3mp_hookcore::profile::{self, BuildIdentity, Profile, Refusal};

const EXE: &str =
    r"C:\Program Files (x86)\Steam\steamapps\common\Transport Fever 3\TransportFever3.exe";
const PROFILE: &str = include_str!("data/tpf3_steam_40408.toml");

/// Known function RVAs (image base 0x140000000), from the recon.
const TARGETS: &[(&str, u64)] = &[
    ("GameSim::Step", 0x159390),
    ("CGame::RunGameSimLoop", 0x11e210),
    ("CGame::Sync", 0x11f650),
    ("GameState::Replicate", 0x255de0),
    ("CGameTime::GetSpeed", 0x2a95a0),
    ("CommandList::Add", 0x9d29c0),
    ("CommandList::Swap", 0x9d2d20),
    ("CommandApply::Dispatch", 0x9d7350),
    ("CommandApply::One", 0x9e1c10),
    ("SetupCommandInterface", 0xe38ae0),
    ("Player::Create", 0xcceee0),
    ("EntitySetPlayer::Apply", 0x9db080),
];

#[test]
fn resolves_every_tpf3_steam_target_uniquely_and_refuses_tampering() {
    let exe = Path::new(EXE);
    if !exe.exists() {
        eprintln!("skipping: {EXE} is not present (this is expected in CI)");
        return;
    }
    let image = std::fs::read(exe).unwrap();
    let profile = Profile::from_toml(PROFILE).unwrap();

    // The profile is for exactly this build.
    let identity = BuildIdentity::of_file(exe).unwrap();
    profile
        .verify_identity(&identity)
        .expect("the on-disk executable matches the pinned build identity");

    // A different build is refused up front.
    let stranger = BuildIdentity {
        sha256: "00".repeat(32),
        size: identity.size,
        pe_timestamp: identity.pe_timestamp,
    };
    assert!(matches!(
        profile.verify_identity(&stranger),
        Err(Refusal::UnknownBuild { .. })
    ));

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
    let index = (0x159390u64 - base) as usize; // GameSim::Step
    tampered[index] ^= 0xFF;
    let refusal = profile::resolve(&profile, &tampered, base).unwrap_err();
    assert!(
        matches!(refusal, Refusal::Missing { ref target } if target == "GameSim::Step"),
        "expected a Missing refusal, got {refusal:?}"
    );
}
