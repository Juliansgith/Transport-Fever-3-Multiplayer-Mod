//! The profile `tools/re/make_profile.py` emits resolves with hookcore itself.
//!
//! `data/make_profile_fixture.pe` is a synthetic PE32+ built by
//! `tools/re/test_make_profile.py` (hand-assembled functions, no game code), and
//! `data/make_profile_fixture.toml` is the profile `make_profile.py` generated
//! for it; that test fails if either drifts from what the tool emits today.
//! Here the real loader and resolver take them: the identity matches, every
//! target resolves uniquely to its RVA with its prologue, and the twins the
//! signatures had to be extended past make a tampered copy refuse.

#![allow(clippy::unwrap_used)]

use tpf3mp_hookcore::pe::PeHeaders;
use tpf3mp_hookcore::profile::{self, BuildIdentity, Profile, Refusal};

const IMAGE: &[u8] = include_bytes!("data/make_profile_fixture.pe");
const PROFILE: &str = include_str!("data/make_profile_fixture.toml");

/// The functions' RVAs as `test_make_profile.py` lays them out.
const TARGETS: &[(&str, u64, bool)] = &[
    ("Alpha::Init", 0x1000, true),
    ("Beta::Speed", 0x1040, true),
    ("Beta::Rate", 0x1080, true),
    ("Gamma::Load", 0x10c0, true),
    ("Gamma::Store", 0x1100, false),
];

fn text() -> (Vec<u8>, u64) {
    let pe = PeHeaders::parse(IMAGE).unwrap();
    let text = pe.section(".text").unwrap();
    (
        text.raw(IMAGE).unwrap().to_vec(),
        u64::from(text.virtual_address),
    )
}

#[test]
fn generated_profile_parses_and_matches_the_build() {
    let profile = Profile::from_toml(PROFILE).unwrap();
    assert_eq!(profile.region.as_deref(), Some(".text"));
    assert_eq!(profile.image_base, Some(0x1_4000_0000));
    assert_eq!(profile.build.size, Some(IMAGE.len() as u64));
    profile
        .verify_identity(&BuildIdentity::of_bytes(IMAGE))
        .expect("the generated identity is the fixture's");
    for spec in &profile.targets {
        assert!(
            spec.prologue.len() >= 14,
            "{} steals fewer bytes than a far detour needs",
            spec.name
        );
    }
}

#[test]
fn generated_profile_resolves_every_target_to_its_rva() {
    let profile = Profile::from_toml(PROFILE).unwrap();
    let (text, base) = text();
    let resolved = profile::resolve(&profile, &text, base).unwrap();
    assert_eq!(resolved.targets.len(), TARGETS.len());
    assert!(resolved.absent_optional.is_empty());
    for &(name, rva, required) in TARGETS {
        let target = resolved
            .get(name)
            .unwrap_or_else(|| panic!("{name} did not resolve"));
        assert_eq!(target.address, rva, "{name} resolved to the wrong RVA");
        assert_eq!(target.required, required, "{name} required flag");
    }
}

#[test]
fn the_extension_past_the_twin_call_is_what_keeps_it_unique() {
    // Beta::Rate differs from Beta::Speed only in `mov eax,[rax+8]` after the
    // call; turn it into `[rax+4]` and Beta::Speed's signature matches twice.
    let profile = Profile::from_toml(PROFILE).unwrap();
    let (mut text, base) = text();
    let disp = (0x1080 - base) as usize + 16 + 5 + 2;
    assert_eq!(text[disp], 0x08);
    text[disp] = 0x04;
    assert_eq!(
        profile::resolve(&profile, &text, base),
        Err(Refusal::Ambiguous {
            target: "Beta::Speed".into(),
            count: 2
        })
    );
}
