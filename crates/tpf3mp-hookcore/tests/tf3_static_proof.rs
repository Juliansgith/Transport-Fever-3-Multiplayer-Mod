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
    ("CommandList::Add", 0x9d29c0),
    ("WorldBuildProposal apply", 0x9e1160),
    ("luaB_print", 0x2fccd10),
    ("lua_checkstack", 0x2fbd650),
    ("lua_createtable", 0x2fbd880),
    ("lua_gettop", 0x2fbdd10),
    ("lua_next", 0x2fbe080),
    ("lua_pushboolean", 0x2fbe1d0),
    ("lua_pushcclosure", 0x2fbe1f0),
    ("lua_pushlstring", 0x2fbe340),
    ("lua_pushnil", 0x2fbe3a0),
    ("lua_pushnumber", 0x2fbe3c0),
    ("lua_pushvalue", 0x2fbe4b0),
    ("lua_rawget", 0x2fbe590),
    ("lua_rawgeti", 0x2fbe5d0),
    ("lua_rawset", 0x2fbe6b0),
    ("lua_settop", 0x2fbeaf0),
    ("lua_toboolean", 0x2fbecb0),
    ("lua_tolstring", 0x2fbed30),
    ("lua_tonumberx", 0x2fbedd0),
    ("lua_type", 0x2fbef90),
    // The main menu's load (crates/tpf3mp-hook/src/menu.rs).
    ("UI::CMenuUI::DoStep", 0x6a0160),
    ("lua_pcallk", 0x2fbe0c0),
    ("luaL_ref", 0x2fb40b0),
    ("lua_load", 0x2fbdf70),
    ("RegisterAppUsertypes", 0xdc5fa0),
    // The seeds and the order fixes (crates/tpf3mp-hook/src/seeds.rs, order.rs).
    ("ecs::LandVehicleMoveSystem::Update2/shuffle", 0xac1b70),
    ("ecs::LandVehicleMoveSystem::Update2/records", 0xac1d72),
    (
        "ecs::SimEntityAtTerminalSystem::Update/vehicles at stop",
        0xb0e35c,
    ),
    (
        "ecs::TransportVehicleSystem::GetVehiclesAtLineStop",
        0xb86510,
    ),
    ("transport::EdgeReservationManager::Reserve", 0x255c2e0),
    (
        "transport::EdgeReservationManager::Reserve_simple",
        0x255c160,
    ),
    ("transport::EdgeUseManager::Add", 0x255e940),
    ("transport::EdgeUseManager::AddRange", 0x255cc70),
    ("ecs::Engine::Update", 0x2bb8a50),
    ("CGame::CGame::lambda_3", 0x11c6c0),
    ("TownDevelopAt::Apply", 0x9dedf0),
    ("lua_getfield", 0x2fbdb90),
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
