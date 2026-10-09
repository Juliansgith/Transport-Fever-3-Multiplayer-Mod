//! Static proof for the public Steam build 40420 profile.
//!
//! The executable is optional in CI, but when `TPF3MP_TF3_BUILD40420_EXE` is
//! supplied it must be this exact input: an identity mismatch is a test failure.

#![allow(clippy::duplicate_mod, clippy::unwrap_used)]

use std::path::PathBuf;

use tpf3mp_hookcore::pe::PeHeaders;
use tpf3mp_hookcore::profile::{BuildIdentity, Profile};

#[allow(dead_code)]
mod build_data {
    pub const COMPILED_PROFILE_NAME: &str = "test candidate bundle";
    pub const COMPILED_PROFILE_TOML: &str = "";
}

#[allow(dead_code)]
mod guiplayer {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Reg {
        Rax,
        Rbx,
        Rdx,
        R8,
        R14,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Kind {
        Value,
        Owner,
        IconOwner,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct Site {
        pub name: &'static str,
        pub expected: &'static [u8],
        pub reg: Reg,
        pub kind: Kind,
        pub what: &'static str,
    }
}

#[allow(dead_code)]
#[path = "../../../profiles/tf3_build40420_steam_windows/modules.rs"]
mod modules;

#[allow(dead_code, unused_imports)]
#[path = "../../../profiles/tf3_build40420_steam_windows/native.rs"]
mod candidate_native;

const PROFILE: &str = include_str!("../../../profiles/tf3_build40420_steam_windows/hooks.toml");
const EXE_ENV: &str = "TPF3MP_TF3_BUILD40420_EXE";
const SHA256: &str = "74861ac43b041aebc5179154345b3cf1ec83154c8e6cc58e0d9e02ff5fa602e4";
const SIZE: u64 = 69_755_832;
const PE_TIMESTAMP: u32 = 0x6ac5_0427;

/// All 173 profile targets, including optional targets, at their RVAs in the
/// public build 25754343 executable (image base 0x140000000).
const TARGETS: &[(&str, u64)] = &[
    ("GameSim::Step", 0x1593d0),
    ("CGame::Step", 0x11f3f0),
    ("CGame::Sync", 0x11f690),
    ("CGame::Step/Sync call", 0x11f446),
    ("CGame::Sync/ComputeFrameTime call", 0x11f9cc),
    ("CGame::ComputeFrameTime", 0x11d290),
    ("CRenderer::NewUpdate", 0x2ffc60),
    (
        "CRenderer::NewUpdate/ModelInstanceList index call",
        0x3003b2,
    ),
    ("CRenderer::NewUpdate/Lambda7 dispatch call", 0x301b3c),
    (
        "CRenderer::NewUpdate/ModelInstanceList index function",
        0xa4c80,
    ),
    ("RenderModelInstance::_Do_call", 0x2f5d90),
    ("RenderModelInstance/GetInstance call", 0x2f617e),
    ("ModelInstanceList::GetInstance", 0x2fdb80),
    ("RenderModelInstance::transformator slot call", 0x2f640d),
    ("RoadVehicleTransformator::vf3", 0xc8ff70),
    ("RoadVehicleTransformator::path helper call", 0xc90391),
    ("RoadVehicleTransformator::user transforms call", 0xc91371),
    (
        "RoadVehicleTransformator::vf3/CGameTime interpolation call",
        0xc9040f,
    ),
    ("CGameTime interpolation/current getter call", 0xc456e7),
    ("CGameTime interpolation/previous getter call", 0xc456f7),
    ("GameState::Replicate", 0x255e60),
    ("ecs::Engine::RemoveEntity", 0x2bbc700),
    ("CGameTime::GetSpeed", 0x2a95e0),
    ("GameSim::Step/GetSpeed call", 0x15942e),
    ("UI::CMenuUI::StartSavegame", 0x6a2740),
    ("UI::CMenuUI::CreatePage", 0x6a2da0),
    ("CommandList::Add::lambda", 0x9d3a40),
    ("CommandList::Add", 0x9d4040),
    ("WorldBuildProposal apply", 0x9e2900),
    ("ModuleBuilder::MousePressed/Add call", 0x542eb5),
    ("StreetTerminalBuilder::MousePressed/Add call", 0x5948d8),
    ("StreetTerminalBuilder::MousePressed/busy set", 0x5946f5),
    ("ProposalAction::DoApply/Add call", 0x548f75),
    ("luaB_print", 0x2fd4660),
    ("lua_checkstack", 0x2fc4fa0),
    ("lua_createtable", 0x2fc51d0),
    ("lua_gettop", 0x2fc5660),
    ("lua_next", 0x2fc59d0),
    ("lua_pushboolean", 0x2fc5b20),
    ("lua_pushcclosure", 0x2fc5b40),
    ("lua_pushlstring", 0x2fc5c90),
    ("lua_pushnil", 0x2fc5cf0),
    ("lua_pushnumber", 0x2fc5d10),
    ("lua_pushvalue", 0x2fc5e00),
    ("lua_rawget", 0x2fc5ee0),
    ("lua_rawgeti", 0x2fc5f20),
    ("lua_rawset", 0x2fc6000),
    ("lua_settop", 0x2fc6440),
    ("lua_toboolean", 0x2fc6600),
    ("lua_tolstring", 0x2fc6680),
    ("lua_tonumberx", 0x2fc6720),
    ("lua_touserdata", 0x2fc68a0),
    ("lua_type", 0x2fc68e0),
    ("UI::CMenuUI::DoStep", 0x69fe90),
    ("UI::CMenuUI::DoStep/m_game test", 0x69fef0),
    ("UI::CMenuUI::DoStep/m_loadGameResult read", 0x6a09b4),
    ("lua_pcallk", 0x2fc5a10),
    ("luaL_ref", 0x2fbba00),
    ("lua_loadfile", 0x2fa96a0),
    ("lua_cached_loadfile", 0x2fafa80),
    ("lua_load", 0x2fc58c0),
    ("RegisterAppUsertypes", 0xdc6430),
    ("ecs::LandVehicleMoveSystem::Update2/shuffle", 0xac2e20),
    ("ecs::LandVehicleMoveSystem::Update2/records", 0xac3022),
    ("GameSim::Step/paused GameTime advance", 0x159452),
    ("CGameTime::Advance", 0xbad6b0),
    ("CGameTime::Advance/tick", 0xbad739),
    ("CGameTime::GetTickCount", 0x2a9600),
    ("CGameTime::GetUpdateCount", 0x2a96c0),
    (
        "ecs::SimEntityAtTerminalSystem::Update/vehicles at stop",
        0xb0f18c,
    ),
    (
        "ecs::TransportVehicleSystem::GetVehiclesAtLineStop",
        0xb86dc0,
    ),
    ("ecs::TransportVehicleSystem::Update2/visit", 0xb8c57b),
    ("FindNextFreeTerminal/candidate sort", 0xb85ce0),
    ("ecs::LineSystem::GetData/return", 0xad3314),
    ("transport::EdgeReservationManager::Reserve", 0x255fa30),
    (
        "transport::EdgeReservationManager::Reserve_simple",
        0x255f8b0,
    ),
    ("transport::EdgeUseManager::Add", 0x2562090),
    ("transport::EdgeUseManager::AddRange", 0x25603c0),
    ("ecs::Engine::Update", 0x2bbdb20),
    ("game_script_util::Update/lambda_1::_Do_call", 0xf46940),
    ("game_script_util::PostUpdate/lambda_1::_Do_call", 0xf45ea0),
    (
        "game_script_util::HandleEvent/lambda_1/lambda_1::operator()",
        0xf42c60,
    ),
    ("TownDevelopAt::Apply", 0x9e0590),
    ("StreetField::At", 0x2b7a1b0),
    ("StreetField::At/cache found", 0x2b7a26a),
    ("TownUpdateSize::Apply", 0x9e12b0),
    ("TownUpdateSize::Apply/develop", 0x9e142a),
    ("TownUpdateSize::Apply/return", 0x9e1477),
    ("TownDeveloper::Develop", 0x8de0b0),
    ("CommandApply::One", 0x9e33b0),
    ("CommandApply::One/return", 0x9e3702),
    ("StreetDeveloper::TryCandidate", 0x969140),
    ("StreetDeveloper::TryCandidate/return", 0x9698fc),
    ("StreetDeveloper::Reject", 0x964b40),
    ("StreetDeveloper::BuildStreet/errors", 0x96720d),
    ("BaseNodeConfig/field offsets", 0x176b6a7),
    ("StreetProposal/node configuration offsets", 0x22c70af),
    ("BaseNodeConfig/crosswalk set layout", 0xa4ae3d),
    ("lua_getfield", 0x2fc54e0),
    ("probe: GUI GameState getter", 0x6aa5b0),
    ("probe: engine GameState getter", 0x120010),
    ("probe: ProposalStreetGraph::GetPlayerOwnedPtr", 0xa48200),
    ("probe: street_util IsOwnedByOtherPlayer", 0x6101d0),
    ("destination_util::GetTargetsByLandUse/candidates", 0x8e5b55),
    (
        "ecs::SimEntityAtBuildingSystem::Update2/leave batches",
        0xb06e72,
    ),
    ("ecs::PersonMoveSystem::Update2/arrival batch", 0xaee05d),
    ("ecs::SimEntityNeedsPathSystem::Update/list", 0xb19043),
    (
        "ecs::SimEntityNeedsPathSystem::EntityToBeRemoved/data getter call",
        0xb18f51,
    ),
    ("ecs::Engine::EndModification/free-id append", 0x2bba0e1),
    ("UI::StreetBuilder::Step", 0x585160),
    ("UI::StreetBuilder ctor/player store", 0x569b14),
    ("UI::TrackModifier::Step", 0x5cb380),
    ("UI::TrackModifier ctor/player store", 0x5b3618),
    ("UI::Bulldozer::Step", 0x4d5700),
    ("UI::Bulldozer ctor/filter", 0x4c3ff2),
    ("UI::ConstructionBuilder::Step", 0x51c1b0),
    ("UI::ConstructionBuilder ctor/player store", 0x50a7f8),
    ("UI::StreetTerminalBuilder::Step", 0x595320),
    ("UI::StreetTerminalBuilder ctor/player store", 0x58fb63),
    ("UI::ModuleBuilder::Step", 0x544ee0),
    ("UI::ModuleBuilder ctor/player store", 0x53f7c1),
    ("UI::Bulldozer ctor/player store", 0x4c3e01),
    ("probe: StreetBulldozerAction edge test", 0x5f1d30),
    ("probe: bulldozer owner test", 0x5f70e0),
    ("UI::Bulldozer ctor/owner list", 0x4c3e95),
    ("UI::Bulldozer set owner list", 0x4d55e0),
    (
        "view: HudIconManager::PreemptiveOctreeTraversal/player",
        0x67b95b,
    ),
    ("view: StationViewer::vf4/player", 0x83c218),
    ("view: CSelector pick/player", 0x83b8e9),
    ("view: ViewCreator::vf1/player", 0x86918a),
    ("view: CatchmentAreaHelper/player 1", 0x878695),
    ("view: CatchmentAreaHelper/player 2", 0x878fcc),
    ("view: CatchmentAreaHelper/player 3", 0x879150),
    ("view: CatchmentAreaHelper/player 4", 0x879193),
    ("view: CatchmentAreaHelper/owner test", 0x879bc9),
    ("view: LayerManagerColorMap/player 1", 0x87d8e0),
    ("view: LayerManagerColorMap/player 2", 0x87d9b9),
    ("view: LayerManagerColorMap/player 3", 0x87da8f),
    ("view: LayerManager colour lambda/player", 0x88511b),
    ("view: LayerManager colour/owner test", 0x887d22),
    ("view: react RendererComponentDelegate/player", 0x29f9f1a),
    ("view: react RailroadCrossingComp/player", 0x28a17e6),
    ("view: HudIconManager icon pass/owner", 0x674a46),
    ("view: getPlayer binding/push", 0x24f0a22),
    ("probe: LineViewer route data test", 0x7f21a7),
    ("probe: LineViewer GetEdgeGeometries", 0x7f2e60),
    ("view: LineViewer lines of the player/call", 0x7f5cd2),
    ("UI::RendererFactory::Create", 0x828380),
    ("UI::CRendererComponent::AddRenderable", 0x6ae720),
    ("UI::CRendererComponent::RemoveRenderable", 0x6afba0),
    ("UI::BuilderRenderer::Clear", 0x7bc480),
    ("UI::BuilderRenderer::vf0", 0x7ba890),
    ("builder_renderer_util::AddToRenderer", 0x5e1f20),
    ("CreateProposalData", 0xa212d0),
    ("makeProposalData/CreateProposalData call", 0x2515a2a),
    ("UI::CGameUI::~CGameUI", 0x6508c0),
    ("UI::CMenuUI::StartGame/CGameUI store", 0x6a4d10),
    ("UI::CGameUI::CreateUI/RendererFactory field", 0x65c02c),
    ("UI::CGameUI::CreateUI/mainView store", 0x65b3dc),
    ("ProposalViewer/ModelData read", 0x2aa7186),
    ("BuilderRenderer::EndHeightMod/upload flag", 0x7bda5a),
    ("UI::BuilderRenderer::EndHeightMod", 0x7bd9d0),
    ("terrain::ViewTerrain::ApplyBlocks", 0x3968e0),
    ("UI::StreetBuilder::ResetProposal", 0x575650),
    ("view: findBestDepot depot owner test", 0x268d26b),
    ("view: findBestDepot owner test", 0x268d6b5),
    ("simperf: EmissionGridSystem::Update", 0xaaa700),
    ("simperf: EmissionEmitterSystem::Update2", 0xaa6570),
    ("simperf: TownSystem::Update2", 0xb627f0),
    ("simperf: UpdateParcelCollision", 0x932b00),
    ("simperf: UpdateParcelCollision call", 0x260009a),
    (
        "fast-component-index: Engine::GetComponentDataIndex",
        0xa4b50,
    ),
    ("emission::EmissionGridSystem::Update", 0xaaa700),
];

#[test]
fn profile_pins_public_steam_build_40420_and_all_target_names() {
    let profile = Profile::from_toml(PROFILE).unwrap();
    assert_eq!(
        profile.name,
        "Transport Fever 3 Build 40420 (Steam, Windows x64)"
    );
    assert_eq!(profile.build.sha256, SHA256);
    assert_eq!(profile.build.size, Some(SIZE));
    assert_eq!(profile.build.pe_timestamp, Some(PE_TIMESTAMP));
    assert_eq!(profile.image_base, Some(0x0001_4000_0000));
    assert_eq!(profile.targets.len(), 173);
    assert_eq!(TARGETS.len(), 173);
    for &(name, _) in TARGETS {
        assert!(
            profile.targets.iter().any(|target| target.name == name),
            "missing profile target {name}"
        );
    }
}

fn target_rva(name: &str) -> u64 {
    TARGETS
        .iter()
        .find_map(|(target, rva)| (*target == name).then_some(*rva))
        .unwrap_or_else(|| panic!("no 40420 RVA recorded for {name}"))
}

fn rva_bytes<'a>(image: &'a [u8], pe: &PeHeaders, rva: u64, len: usize) -> &'a [u8] {
    let rva = u32::try_from(rva).expect("an RVA fits u32");
    let end = rva
        .checked_add(u32::try_from(len).expect("length fits u32"))
        .unwrap();
    let section = pe
        .sections
        .iter()
        .find(|section| {
            rva >= section.virtual_address
                && end
                    <= section
                        .virtual_address
                        .saturating_add(section.size_of_raw_data)
        })
        .unwrap_or_else(|| panic!("RVA {rva:#x} is not in a raw PE section"));
    let start = usize::try_from(rva - section.virtual_address).unwrap();
    section.raw(image).unwrap().get(start..start + len).unwrap()
}

fn rva_u64(image: &[u8], pe: &PeHeaders, rva: u64) -> u64 {
    u64::from_le_bytes(rva_bytes(image, pe, rva, 8).try_into().unwrap())
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn assert_native_data(image: &[u8], pe: &PeHeaders, text: &[u8], text_rva: u64) {
    for site in &candidate_native::guiplayer::SITES {
        let address = target_rva(site.name);
        let start = usize::try_from(address - text_rva).unwrap();
        assert_eq!(
            text.get(start..start + site.expected.len()).unwrap(),
            site.expected,
            "GUI player splice bytes changed at {} ({address:#x})",
            site.name
        );
    }

    let base = pe.image_base;
    let (edge_vtable, edge_vf0, node_vtable, node_vf0) = (
        candidate_native::network::EDGE_VTABLE as u64,
        0x1769cd0,
        candidate_native::network::NODE_CONFIG_VTABLE as u64,
        0x1769dd0,
    );
    assert_eq!(
        rva_u64(image, pe, edge_vtable),
        base + edge_vf0,
        "BaseEdge UserdataValue vtable"
    );
    assert_eq!(
        rva_u64(image, pe, node_vtable),
        base + node_vf0,
        "BaseNodeConfig UserdataValue vtable"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x177aa2d, 5),
        &[0xba, 0x28, 0x01, 0x00, 0x00],
        "BaseEdge UserdataValue allocation size"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x177aa82, 4),
        &[0x48, 0x8d, 0x5e, 0x10],
        "BaseEdge inline component payload address"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x177aa9d, 4),
        &[0x48, 0x89, 0x5e, 0x08],
        "BaseEdge UserdataValue payload pointer"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x17577db, 5),
        &[0xba, 0x88, 0x00, 0x00, 0x00],
        "BaseNodeConfig UserdataValue allocation size"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x1757832, 4),
        &[0x4c, 0x8d, 0x47, 0x10],
        "BaseNodeConfig inline component payload address"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x17578f0, 4),
        &[0x4c, 0x89, 0x47, 0x08],
        "BaseNodeConfig UserdataValue payload pointer"
    );
    assert_eq!(
        candidate_native::network::LANE_VECTOR,
        0x70,
        "BaseEdge laneConfigs field offset"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x176b381, 7),
        &[0xc7, 0x45, 0xb0, 0x70, 0x00, 0x00, 0x00],
        "BaseEdge laneConfigs registered field offset"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x176b465, 0x1b),
        &[
            0x48, 0x8d, 0x45, 0xb0, 0x48, 0x89, 0x84, 0x24, 0xb0, 0x00, 0x00, 0x00, 0x48, 0x8d,
            0x05, 0x10, 0xbe, 0xfa, 0x01, 0x48, 0x89, 0x84, 0x24, 0xa8, 0x00, 0x00, 0x00,
        ],
        "BaseEdge laneConfigs registration maps to the 0x70 member"
    );
    assert_eq!(
        candidate_native::network::EDGE_SIZE + candidate_native::network::PAYLOAD,
        0x128
    );
    assert_eq!(
        candidate_native::network::NODE_CONFIG_SIZE + candidate_native::network::PAYLOAD,
        0x88
    );

    for (name, table, slot, slot_rva) in [
        (
            candidate_native::simperf::EMISSION_GRID,
            0x3705e38,
            11,
            candidate_native::simperf::EMISSION_GRID_SLOT,
        ),
        (
            candidate_native::simperf::EMISSION_EMITTERS,
            0x37057b0,
            12,
            candidate_native::simperf::EMISSION_EMITTERS_SLOT,
        ),
        (
            candidate_native::simperf::TOWNS,
            0x3711ed8,
            12,
            candidate_native::simperf::TOWNS_SLOT,
        ),
    ] {
        let function = target_rva(name);
        assert_eq!(slot_rva, table + slot * 8, "{name} vtable slot RVA");
        assert_eq!(
            rva_u64(image, pe, slot_rva),
            base + function,
            "{name} vtable slot must call its 40420 target"
        );
    }

    let update = target_rva(candidate_native::emission::UPDATE);
    for code in candidate_native::emission::CODE {
        let address = i64::try_from(update).unwrap() + code.offset;
        let bytes = rva_bytes(image, pe, u64::try_from(address).unwrap(), code.len);
        assert_eq!(
            fnv1a(bytes),
            code.fnv1a,
            "emission code hash: {}",
            code.what
        );
    }

    for (site, opcode, expected_offset) in [
        (
            candidate_native::menu::MENU_GAME_TARGET,
            candidate_native::menu::MENU_GAME_OPCODE,
            0x6b0,
        ),
        (
            candidate_native::menu::MENU_LOAD_TARGET,
            candidate_native::menu::MENU_LOAD_OPCODE,
            0x1bd0,
        ),
    ] {
        let address = target_rva(site);
        let instruction = rva_bytes(image, pe, address, 7);
        assert_eq!(&instruction[..3], opcode, "menu field opcode: {site}");
        assert_eq!(
            u32::from_le_bytes(instruction[3..7].try_into().unwrap()),
            expected_offset,
            "menu field offset: {site}"
        );
    }
    let menu_vtable = 0x36d0ff0;
    assert_eq!(
        rva_u64(image, pe, menu_vtable + 52 * 8),
        base + target_rva(candidate_native::menu::MENU_STEP_TARGET),
        "CMenuUI::DoStep remains vtable slot 52 in the 60-slot 40420 class"
    );
}

#[test]
fn supplied_40420_executable_has_the_exact_identity_and_runtime_resolves_all_173_sites() {
    let Some(exe) = std::env::var_os(EXE_ENV).map(PathBuf::from) else {
        eprintln!("skipping executable resolution: set {EXE_ENV} to the archived 40420 executable");
        return;
    };
    let profile = Profile::from_toml(PROFILE).unwrap();
    let identity = BuildIdentity::of_file(&exe)
        .unwrap_or_else(|error| panic!("read {}: {error}", exe.display()));
    assert_eq!(
        identity.sha256, SHA256,
        "{EXE_ENV} must point at the public Steam 40420 executable"
    );
    assert_eq!(
        identity.size,
        Some(SIZE),
        "{EXE_ENV} has the wrong file size"
    );
    assert_eq!(
        identity.pe_timestamp,
        Some(PE_TIMESTAMP),
        "{EXE_ENV} has the wrong PE timestamp"
    );
    profile
        .verify_identity(&identity)
        .expect("the supplied executable must match the pinned profile");

    let image =
        std::fs::read(&exe).unwrap_or_else(|error| panic!("read {}: {error}", exe.display()));
    let pe = PeHeaders::parse(&image).expect("a PE32+ executable");
    assert_eq!(pe.image_base, 0x0001_4000_0000);
    let text = pe
        .section(".text")
        .expect("the executable has a .text section");
    let text_bytes = text.raw(&image).expect(".text raw bytes");
    assert_eq!(profile.targets.len(), TARGETS.len());
    let text_rva = u64::from(text.virtual_address);
    for spec in &profile.targets {
        let address = target_rva(&spec.name);
        let target_index = usize::try_from(address - text_rva).unwrap();
        let match_index =
            usize::try_from(i64::try_from(target_index).unwrap() - spec.offset).unwrap();
        assert!(
            spec.pattern().matches_at(text_bytes, match_index),
            "{} signature does not match at its recorded RVA {address:#x}",
            spec.name
        );
        assert_eq!(
            i64::try_from(match_index).unwrap() + spec.offset,
            i64::try_from(target_index).unwrap(),
            "{} signature offset must resolve to its recorded RVA",
            spec.name
        );
        assert_eq!(
            text_bytes
                .get(target_index..target_index + spec.prologue.len())
                .unwrap(),
            spec.prologue,
            "{} prologue at {address:#x}",
            spec.name
        );
    }
    // Hook startup scans the mapped .text virtual-size span and rejects any
    // ambiguous optional target. Rebuild that exact contiguous view from the
    // PE section's raw bytes (zero-fill the loader's virtual tail), then use
    // the same resolver rather than only checking each signature at its
    // expected address.
    let virtual_size = usize::try_from(text.virtual_size).unwrap();
    let mut loaded_text = vec![0; virtual_size];
    let loaded_bytes = text_bytes.len().min(loaded_text.len());
    loaded_text[..loaded_bytes].copy_from_slice(&text_bytes[..loaded_bytes]);
    let resolved = tpf3mp_hookcore::profile::resolve(&profile, &loaded_text, text_rva)
        .expect("runtime's profile resolver must uniquely resolve the mapped .text");
    assert_eq!(
        resolved
            .get("RoadVehicleTransformator::vf3")
            .expect("road vf3 target")
            .address,
        target_rva("RoadVehicleTransformator::vf3"),
    );
    for &(name, rva) in TARGETS {
        assert_eq!(target_rva(name), rva, "{name} expected RVA changed");
    }
    assert_native_data(&image, &pe, text_bytes, text_rva);
}

#[test]
fn supplied_40420_executable_uniquely_resolves_native_time_probe_callsites() {
    let Some(exe) = std::env::var_os(EXE_ENV).map(PathBuf::from) else {
        eprintln!("skipping executable resolution: set {EXE_ENV} to the archived 40420 executable");
        return;
    };
    let profile = Profile::from_toml(PROFILE).unwrap();
    let identity = BuildIdentity::of_file(&exe)
        .unwrap_or_else(|error| panic!("read {}: {error}", exe.display()));
    profile
        .verify_identity(&identity)
        .expect("the supplied executable must match the pinned Steam 40420 profile");
    let image =
        std::fs::read(&exe).unwrap_or_else(|error| panic!("read {}: {error}", exe.display()));
    let pe = PeHeaders::parse(&image).expect("a PE32+ executable");
    let text = pe.section(".text").expect("the executable has .text");
    let text_bytes = text.raw(&image).expect(".text raw bytes");
    let text_rva = u64::from(text.virtual_address);
    let mut loaded_text = vec![0; usize::try_from(text.virtual_size).unwrap()];
    let copied = text_bytes.len().min(loaded_text.len());
    loaded_text[..copied].copy_from_slice(&text_bytes[..copied]);

    let names = [
        "RoadVehicleTransformator::vf3/CGameTime interpolation call",
        "CGameTime interpolation/current getter call",
        "CGameTime interpolation/previous getter call",
    ];
    let mut focused = profile.clone();
    focused
        .targets
        .retain(|target| names.contains(&target.name.as_str()));
    assert_eq!(focused.targets.len(), names.len());
    let resolved = tpf3mp_hookcore::profile::resolve(&focused, &loaded_text, text_rva)
        .expect("all three exact 40420 callsites must resolve uniquely in mapped .text");
    for (name, rva) in [
        (names[0], 0xc9040f),
        (names[1], 0xc456e7),
        (names[2], 0xc456f7),
    ] {
        assert_eq!(resolved.get(name).unwrap().address, rva, "{name}");
    }
}

#[test]
fn supplied_40420_executable_uniquely_resolves_road_history_writer_targets() {
    let Some(exe) = std::env::var_os(EXE_ENV).map(PathBuf::from) else {
        eprintln!("skipping executable resolution: set {EXE_ENV} to the archived 40420 executable");
        return;
    };
    let profile = Profile::from_toml(PROFILE).unwrap();
    let identity = BuildIdentity::of_file(&exe)
        .unwrap_or_else(|error| panic!("read {}: {error}", exe.display()));
    profile
        .verify_identity(&identity)
        .expect("the supplied executable must match the pinned Steam 40420 profile");
    let image =
        std::fs::read(&exe).unwrap_or_else(|error| panic!("read {}: {error}", exe.display()));
    let pe = PeHeaders::parse(&image).expect("a PE32+ executable");
    let text = pe.section(".text").expect("the executable has .text");
    let text_bytes = text.raw(&image).expect(".text raw bytes");
    let text_rva = u64::from(text.virtual_address);
    let mut loaded_text = vec![0; usize::try_from(text.virtual_size).unwrap()];
    let copied = text_bytes.len().min(loaded_text.len());
    loaded_text[..copied].copy_from_slice(&text_bytes[..copied]);

    let names = ["GameState::Replicate", "ecs::Engine::RemoveEntity"];
    let mut focused = profile.clone();
    focused
        .targets
        .retain(|target| names.contains(&target.name.as_str()));
    assert_eq!(focused.targets.len(), names.len());
    let resolved = tpf3mp_hookcore::profile::resolve(&focused, &loaded_text, text_rva)
        .expect("both exact 40420 writer hooks must resolve uniquely in mapped .text");
    for (name, rva) in [(names[0], 0x255e60), (names[1], 0x2bbc700)] {
        assert_eq!(resolved.get(name).unwrap().address, rva, "{name}");
    }
}
