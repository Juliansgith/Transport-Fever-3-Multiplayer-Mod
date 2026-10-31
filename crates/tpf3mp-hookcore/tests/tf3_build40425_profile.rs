//! Static proof for the public Steam build 40425 profile (Steam build ID
//! 26129055), the 40423 proof with the identity changed: 40425's code is
//! 40423's but for 17 bytes of version data, so every fact below holds at
//! the same address (investigation/UPDATE_40425_2026-10-24.md).
//!
//! The executable is optional in CI, but when `TPF3MP_TF3_BUILD40425_EXE` is
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
#[path = "../../../profiles/tf3_build40425_steam_windows/modules.rs"]
mod modules;

#[allow(dead_code, unused_imports)]
#[path = "../../../profiles/tf3_build40425_steam_windows/native.rs"]
mod candidate_native;

const PROFILE: &str = include_str!("../../../profiles/tf3_build40425_steam_windows/hooks.toml");
const EXE_ENV: &str = "TPF3MP_TF3_BUILD40425_EXE";
const SHA256: &str = "e9a6190f9ca8f7f34a46dfafac0f3b897e41ea6dc6498d6f5871aed6b9ca74cd";
const SIZE: u64 = 69_876_664;
const PE_TIMESTAMP: u32 = 0x6ad9_ce10;

/// All 170 profile targets, including optional targets, at their RVAs in the
/// public build 26129055 executable (image base 0x140000000).
const TARGETS: &[(&str, u64)] = &[
    ("save: PushCompressor level load", 0x32dec4),
    ("save: PushCompressor buffer size", 0x32e024),
    ("GameSim::Step", 0x1593e0),
    ("CGame::Step", 0x11f400),
    // The batch interval (crates/tpf3mp-hook/src/interval.rs).
    ("CGame::Sync", 0x11f6a0),
    ("CGame::Step/Sync call", 0x11f456),
    ("CGameTime::GetSpeed", 0x2a9600),
    ("GameSim::Step/GetSpeed call", 0x15943e),
    ("UI::CMenuUI::StartSavegame", 0x6a3540),
    ("UI::CMenuUI::CreatePage", 0x6a3ba0),
    ("CommandList::Add::lambda", 0x9d8b40),
    ("CommandList::Add", 0x9d9140),
    ("WorldBuildProposal apply", 0x9e7a00),
    ("ModuleBuilder::MousePressed/Add call", 0x543ce5),
    ("StreetTerminalBuilder::MousePressed/Add call", 0x5956e8),
    ("StreetTerminalBuilder::MousePressed/busy set", 0x595505),
    ("StreetBuilder::UpdateEngine/Add call", 0x589fbf),
    ("StreetBuilder::UpdateEngine/capture", 0x589d75),
    ("StreetBuilder::UpdateEngine callback/result scan", 0x56e779),
    ("StreetBuilder::UpdateEngine callback/snap", 0x56ea32),
    ("UI::StreetBuilder::StartAt", 0x571860),
    ("ecs::Engine::EntityExists", 0x4f7fa0),
    ("RailroadCrossingComp/type selection Add call", 0x28aad7c),
    ("StreetProposal/level crossing maps", 0x4c8b16),
    (
        "UpdateRailRoadCrossingProposal/crossing map insert",
        0x28ab7df,
    ),
    ("ProposalAction::DoApply/Add call", 0x549da5),
    ("luaB_print", 0x2fe99f0),
    ("lua_checkstack", 0x2fda330),
    ("lua_createtable", 0x2fda560),
    ("lua_gettop", 0x2fda9f0),
    ("lua_next", 0x2fdad60),
    ("lua_pushboolean", 0x2fdaeb0),
    ("lua_pushcclosure", 0x2fdaed0),
    ("lua_pushlstring", 0x2fdb020),
    ("lua_pushnil", 0x2fdb080),
    ("lua_pushnumber", 0x2fdb0a0),
    ("lua_pushvalue", 0x2fdb190),
    ("lua_rawget", 0x2fdb270),
    ("lua_rawgeti", 0x2fdb2b0),
    ("lua_rawset", 0x2fdb390),
    ("lua_settop", 0x2fdb7d0),
    ("lua_toboolean", 0x2fdb990),
    ("lua_tolstring", 0x2fdba10),
    ("lua_tonumberx", 0x2fdbab0),
    ("lua_touserdata", 0x2fdbc30),
    ("lua_type", 0x2fdbc70),
    ("UI::CMenuUI::DoStep", 0x6a0c20),
    ("UI::CMenuUI::DoStep/m_game test", 0x6a0c80),
    ("UI::CMenuUI::DoStep/m_loadGameResult read", 0x6a1743),
    ("lua_pcallk", 0x2fdada0),
    ("luaL_ref", 0x2fd0d90),
    ("lua_loadfile", 0x2fbea30),
    ("lua_cached_loadfile", 0x2fc4e10),
    ("lua_load", 0x2fdac50),
    ("RegisterAppUsertypes", 0xdcc770),
    ("ecs::LandVehicleMoveSystem::Update2/shuffle", 0xac7fb0),
    ("ecs::LandVehicleMoveSystem::Update2/records", 0xac81b2),
    ("GameSim::Step/paused GameTime advance", 0x159462),
    ("CGameTime::Advance", 0xbb28a0),
    ("CGameTime::Advance/tick", 0xbb2929),
    ("CGameTime::GetTickCount", 0x2a9620),
    ("CGameTime::GetUpdateCount", 0x2a96e0),
    (
        "ecs::SimEntityAtTerminalSystem::Update/vehicles at stop",
        0xb1433c,
    ),
    (
        "ecs::TransportVehicleSystem::GetVehiclesAtLineStop",
        0xb8bfa0,
    ),
    ("ecs::TransportVehicleSystem::Update2/visit", 0xb9175b),
    ("FindNextFreeTerminal/candidate sort", 0xb8aec0),
    ("ecs::LineSystem::GetData/return", 0xad84a4),
    ("transport::EdgeReservationManager::Reserve", 0x2566af0),
    (
        "transport::EdgeReservationManager::Reserve_simple",
        0x2566970,
    ),
    ("transport::EdgeUseManager::Add", 0x2569150),
    ("transport::EdgeUseManager::AddRange", 0x2567480),
    ("ecs::Engine::Update", 0x2bc5440),
    ("game_script_util::Update/lambda_1::_Do_call", 0xf4d790),
    ("game_script_util::PostUpdate/lambda_1::_Do_call", 0xf4ccf0),
    (
        "game_script_util::HandleEvent/lambda_1/lambda_1::operator()",
        0xf49ab0,
    ),
    ("TownDevelopAt::Apply", 0x9e5690),
    ("StreetField::At", 0x2b81850),
    ("StreetField::At/cache found", 0x2b8190a),
    ("TownUpdateSize::Apply", 0x9e63b0),
    ("TownUpdateSize::Apply/develop", 0x9e652a),
    ("TownUpdateSize::Apply/return", 0x9e6577),
    ("TownDeveloper::Develop", 0x8e31c0),
    ("CommandApply::One", 0x9e84b0),
    ("CommandApply::One/return", 0x9e8802),
    ("StreetDeveloper::TryCandidate", 0x96e240),
    ("StreetDeveloper::TryCandidate/return", 0x96e9fc),
    ("StreetDeveloper::Reject", 0x969c40),
    ("StreetDeveloper::BuildStreet/errors", 0x96c30d),
    ("BaseNodeConfig/field offsets", 0x1772577),
    ("StreetProposal/node configuration offsets", 0x22cdf7f),
    ("BaseNodeConfig/crosswalk set layout", 0xa5003d),
    ("lua_getfield", 0x2fda870),
    ("probe: GUI GameState getter", 0x6ab3b0),
    ("probe: engine GameState getter", 0x120020),
    ("probe: ProposalStreetGraph::GetPlayerOwnedPtr", 0xa4d400),
    ("probe: street_util IsOwnedByOtherPlayer", 0x6110b0),
    ("destination_util::GetTargetsByLandUse/candidates", 0x8eac65),
    (
        "ecs::SimEntityAtBuildingSystem::Update2/leave batches",
        0xb0c002,
    ),
    ("ecs::PersonMoveSystem::Update2/arrival batch", 0xaf31ed),
    ("ecs::SimEntityNeedsPathSystem::Update/list", 0xb1e1f3),
    (
        "ecs::SimEntityNeedsPathSystem::EntityToBeRemoved/data getter call",
        0xb1e101,
    ),
    ("ecs::Engine::EndModification/free-id append", 0x2bc1a01),
    ("UI::StreetBuilder::Step", 0x585f90),
    ("UI::StreetBuilder ctor/player store", 0x56a944),
    ("UI::TrackModifier::Step", 0x5cc190),
    ("UI::TrackModifier ctor/player store", 0x5b4428),
    ("UI::Bulldozer::Step", 0x4d6530),
    ("UI::Bulldozer ctor/filter", 0x4c4e22),
    ("UI::ConstructionBuilder::Step", 0x51cff0),
    ("UI::ConstructionBuilder ctor/player store", 0x50b638),
    ("UI::StreetTerminalBuilder::Step", 0x596130),
    ("UI::StreetTerminalBuilder ctor/player store", 0x590973),
    ("UI::ModuleBuilder::Step", 0x545d10),
    ("UI::ModuleBuilder ctor/player store", 0x5405f1),
    ("UI::Bulldozer ctor/player store", 0x4c4c31),
    ("probe: StreetBulldozerAction edge test", 0x5f2b30),
    ("probe: bulldozer owner test", 0x5f7fc0),
    ("UI::Bulldozer ctor/owner list", 0x4c4cc5),
    ("UI::Bulldozer set owner list", 0x4d6410),
    (
        "view: HudIconManager::PreemptiveOctreeTraversal/player",
        0x67c6eb,
    ),
    ("view: StationViewer::vf4/player", 0x841098),
    ("view: CSelector pick/player", 0x840769),
    ("view: ViewCreator::vf1/player", 0x86e29a),
    ("view: CatchmentAreaHelper/player 1", 0x87d7a5),
    ("view: CatchmentAreaHelper/player 2", 0x87e0dc),
    ("view: CatchmentAreaHelper/player 3", 0x87e260),
    ("view: CatchmentAreaHelper/player 4", 0x87e2a3),
    ("view: CatchmentAreaHelper/owner test", 0x87ecd9),
    ("view: LayerManagerColorMap/player 1", 0x8829f0),
    ("view: LayerManagerColorMap/player 2", 0x882ac9),
    ("view: LayerManagerColorMap/player 3", 0x882b9f),
    ("view: LayerManager colour lambda/player", 0x88a22b),
    ("view: LayerManager colour/owner test", 0x88ce32),
    ("view: react RendererComponentDelegate/player", 0x2a0142a),
    ("view: react RailroadCrossingComp/player", 0x28a8cf6),
    ("view: HudIconManager icon pass/owner", 0x6757d6),
    ("view: getPlayer binding/push", 0x24f78b2),
    ("probe: LineViewer route data test", 0x7f7127),
    ("probe: LineViewer GetEdgeGeometries", 0x7f7de0),
    ("view: LineViewer lines of the player/call", 0x7fac52),
    ("UI::RendererFactory::Create", 0x82d200),
    ("UI::CRendererComponent::AddRenderable", 0x6af520),
    ("UI::CRendererComponent::RemoveRenderable", 0x6b09c0),
    ("UI::BuilderRenderer::Clear", 0x7c1400),
    ("UI::BuilderRenderer::vf0", 0x7bf810),
    ("builder_renderer_util::AddToRenderer", 0x5e2d30),
    ("CreateProposalData", 0xa263d0),
    ("makeProposalData/CreateProposalData call", 0x251c96a),
    ("UI::CGameUI::~CGameUI", 0x6516b0),
    ("UI::CMenuUI::StartGame/CGameUI store", 0x6a5b13),
    ("UI::CGameUI::CreateUI/RendererFactory field", 0x65cdac),
    ("UI::CGameUI::CreateUI/mainView store", 0x65c15c),
    ("ProposalViewer/ModelData read", 0x2aae696),
    ("BuilderRenderer::EndHeightMod/upload flag", 0x7c29da),
    ("UI::BuilderRenderer::EndHeightMod", 0x7c2950),
    ("terrain::ViewTerrain::ApplyBlocks", 0x3976f0),
    ("UI::StreetBuilder::ResetProposal", 0x576480),
    ("StreetBuilder state swap/ResetProposal call", 0x56dc2f),
    ("view: findBestDepot depot owner test", 0x269434b),
    ("view: findBestDepot owner test", 0x2694795),
    ("simperf: EmissionGridSystem::Update", 0xaaf890),
    ("simperf: EmissionEmitterSystem::Update2", 0xaab700),
    ("simperf: TownSystem::Update2", 0xb679d0),
    ("simperf: UpdateParcelCollision", 0x937c10),
    ("simperf: UpdateParcelCollision call", 0x260716a),
    (
        "fast-component-index: Engine::GetComponentDataIndex",
        0xa4b60,
    ),
    ("emission::EmissionGridSystem::Update", 0xaaf890),
    ("industry schedule", 0x1513f0),
    ("fast-bitmap-span: BitmapRasterizable::Fill", 0x91a560),
    ("UI::CGameUI::GameStep/CGame::Step call", 0x65f26c),
];

#[test]
fn profile_pins_public_steam_build_40425_and_all_target_names() {
    let profile = Profile::from_toml(PROFILE).unwrap();
    assert_eq!(
        profile.name,
        "Transport Fever 3 Build 40425 (Steam, Windows x64)"
    );
    assert_eq!(profile.build.sha256, SHA256);
    assert_eq!(profile.build.size, Some(SIZE));
    assert_eq!(profile.build.pe_timestamp, Some(PE_TIMESTAMP));
    assert_eq!(profile.image_base, Some(0x0001_4000_0000));
    assert_eq!(profile.targets.len(), 170);
    assert_eq!(TARGETS.len(), 170);
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
        .unwrap_or_else(|| panic!("no 40425 RVA recorded for {name}"))
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

/// The world checks' native reader's data (`netread.rs`): each pool's
/// `CompVec<T>` vtable (its first slot that class's own `vf0`), and the
/// engine's code that reads what the reader reads, the same bytes as on
/// 40408 at their new places (the investigation's functions, matched by
/// identical bodies): the component sizes, the pools, dense vectors and
/// pages, the entity table, the component bits and a construction's place.
fn assert_netread_data(image: &[u8], pe: &PeHeaders) {
    use candidate_native::netread as layout;
    let base = pe.image_base;
    for (name, vtable, vf0) in [
        ("BaseEdge", layout::BASE_EDGE_POOL_VTABLE, 0x22a590),
        ("BaseNode", layout::BASE_NODE_POOL_VTABLE, 0x22a760),
        (
            "BaseNodeConfig",
            layout::BASE_NODE_CONFIG_POOL_VTABLE,
            0x22a800,
        ),
        ("Construction", layout::CONSTRUCTION_POOL_VTABLE, 0x22ad00),
        (
            "RailroadCrossing",
            layout::RAILROAD_CROSSING_POOL_VTABLE,
            0x22b780,
        ),
    ] {
        assert_eq!(
            rva_u64(image, pe, vtable as u64),
            base + vf0,
            "CompVec<{name}> vtable"
        );
    }
    let code: [(u64, &[u8], &str); 14] = [
        (
            0x9554b,
            &[0x48, 0x69, 0xc0, 0x18, 0x01, 0x00, 0x00],
            "imul rax, 0x118 (BaseEdge size)",
        ),
        (
            0x2813dc,
            &[0x48, 0x6b, 0xc0, 0x78],
            "imul rax, 0x78 (BaseNodeConfig size)",
        ),
        (
            0x2808ab,
            &[0x48, 0x69, 0xc0, 0x88, 0x02, 0x00, 0x00],
            "imul rax, 0x288 (Construction size)",
        ),
        (
            0x280748,
            &[0x48, 0x8b, 0x4b, 0x78],
            "mov rcx, [rbx+0x78] (the pools)",
        ),
        (
            0x280750,
            &[0x3d, 0x00, 0x00, 0x00, 0x40],
            "cmp eax, 0x40000000 (paged from)",
        ),
        (
            0x28076c,
            &[0x49, 0x8b, 0x91, 0x80, 0x00, 0x00, 0x00],
            "mov rdx, [r9+0x80] (pages)",
        ),
        (
            0x28079d,
            &[0x49, 0x8b, 0x41, 0x68],
            "mov rax, [r9+0x68] (dense)",
        ),
        (
            0xa4b9c,
            &[0x48, 0x8b, 0x81, 0x90, 0x00, 0x00, 0x00],
            "mov rax, [rcx+0x90] (entity table)",
        ),
        (
            0x2bc2b21,
            &[0x48, 0x8b, 0x86, 0xc0, 0x00, 0x00, 0x00],
            "mov rax, [rsi+0xc0] (component bits)",
        ),
        (
            0x54527a,
            &[0xc4, 0xc1, 0x7a, 0x10, 0xad, 0x88, 0x00, 0x00, 0x00],
            "vmovss xmm5, [r13+0x88] (transf x)",
        ),
        (
            0x545295,
            &[0xc4, 0x41, 0x7a, 0x10, 0xa5, 0x8c, 0x00, 0x00, 0x00],
            "vmovss xmm12, [r13+0x8c] (transf y)",
        ),
        // The RailroadCrossing store's copy (sub_17ee80): its 0x40 stride,
        // and its type index at +0x30.
        (
            0x17ef1d,
            &[0x48, 0xc1, 0xe3, 0x06],
            "shl rbx, 6 (RailroadCrossing size)",
        ),
        (
            0x17ef6a,
            &[0x8b, 0x46, 0x30, 0x89, 0x43, 0x30],
            "mov eax, [rsi+0x30]; mov [rbx+0x30], eax (a crossing's type index)",
        ),
        // Its Lua binding's field offsets (the binding at 0x188e91e): stateTime 0x38,
        // state 0x34, typeIndex 0x30, edges 0x18, then nodes (r15d, 0).
        (
            0x188e91e,
            &[
                0xc7, 0x45, 0x80, 0x38, 0x00, 0x00, 0x00, 0xc7, 0x45, 0x84, 0x34, 0x00, 0x00, 0x00,
                0xc7, 0x45, 0x88, 0x30, 0x00, 0x00, 0x00, 0xc7, 0x45, 0x8c, 0x18, 0x00, 0x00, 0x00,
                0x44, 0x89, 0x7d, 0x90,
            ],
            "RailroadCrossing's Lua field offsets",
        ),
    ];
    for (rva, bytes, what) in code {
        assert_eq!(
            rva_bytes(image, pe, rva, bytes.len()),
            bytes,
            "{what} at {rva:#x}"
        );
    }
    assert_eq!(
        (
            layout::POOLS,
            layout::ENTITIES,
            layout::BITS,
            layout::POOL_DENSE,
            layout::POOL_PAGES
        ),
        (0x78, 0x90, 0xc0, 0x68, 0x80)
    );
    assert_eq!(
        (
            layout::BASE_EDGE_SIZE,
            layout::BASE_NODE_CONFIG_SIZE,
            layout::CONSTRUCTION_SIZE
        ),
        (0x118, 0x78, 0x288)
    );
    assert_eq!(
        (layout::CONSTRUCTION_X, layout::CONSTRUCTION_Y),
        (0x88, 0x8c)
    );
    assert_eq!(
        (
            layout::RAILROAD_CROSSING_SIZE,
            layout::CROSSING_NODES,
            layout::CROSSING_TYPE_INDEX
        ),
        (0x40, 0x00, 0x30)
    );
}

fn assert_native_data(image: &[u8], pe: &PeHeaders, text: &[u8], text_rva: u64) {
    // Both compression and decompression read this constant. Rewriting the
    // compressor's instruction must leave the constant (and loader) alone.
    assert_eq!(rva_bytes(image, pe, 0x37e4ad8, 4), 3u32.to_le_bytes());
    assert_eq!(
        rva_bytes(image, pe, 0x32dec4, 6),
        candidate_native::savefast::LEVEL_LOAD_BYTES
    );
    assert_eq!(
        rva_bytes(image, pe, 0x32e024, 6),
        candidate_native::savefast::BUFFER_BYTES
    );
    assert_netread_data(image, pe);
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
        0x1770ba0,
        candidate_native::network::NODE_CONFIG_VTABLE as u64,
        0x1770ca0,
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
        rva_bytes(image, pe, 0x17818fd, 5),
        &[0xba, 0x28, 0x01, 0x00, 0x00],
        "BaseEdge UserdataValue allocation size"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x1781952, 4),
        &[0x48, 0x8d, 0x5e, 0x10],
        "BaseEdge inline component payload address"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x178196d, 4),
        &[0x48, 0x89, 0x5e, 0x08],
        "BaseEdge UserdataValue payload pointer"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x175e6ab, 5),
        &[0xba, 0x88, 0x00, 0x00, 0x00],
        "BaseNodeConfig UserdataValue allocation size"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x175e702, 4),
        &[0x4c, 0x8d, 0x47, 0x10],
        "BaseNodeConfig inline component payload address"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x175e7c0, 4),
        &[0x4c, 0x89, 0x47, 0x08],
        "BaseNodeConfig UserdataValue payload pointer"
    );
    assert_eq!(
        candidate_native::network::LANE_VECTOR,
        0x70,
        "BaseEdge laneConfigs field offset"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x1772251, 7),
        &[0xc7, 0x45, 0xb0, 0x70, 0x00, 0x00, 0x00],
        "BaseEdge laneConfigs registered field offset"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x1772335, 0x1b),
        &[
            0x48, 0x8d, 0x45, 0xb0, 0x48, 0x89, 0x84, 0x24, 0xb0, 0x00, 0x00, 0x00, 0x48, 0x8d,
            0x05, 0x60, 0xc2, 0xfb, 0x01, 0x48, 0x89, 0x84, 0x24, 0xa8, 0x00, 0x00, 0x00,
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
            0x371d158,
            11,
            candidate_native::simperf::EMISSION_GRID_SLOT,
        ),
        (
            candidate_native::simperf::EMISSION_EMITTERS,
            0x371cad0,
            12,
            candidate_native::simperf::EMISSION_EMITTERS_SLOT,
        ),
        (
            candidate_native::simperf::TOWNS,
            0x37291f8,
            12,
            candidate_native::simperf::TOWNS_SLOT,
        ),
    ] {
        let function = target_rva(name);
        assert_eq!(slot_rva, table + slot * 8, "{name} vtable slot RVA");
        assert_eq!(
            rva_u64(image, pe, slot_rva),
            base + function,
            "{name} vtable slot must call its 40425 target"
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
            0x8d8,
        ),
        (
            candidate_native::menu::MENU_LOAD_TARGET,
            candidate_native::menu::MENU_LOAD_OPCODE,
            0x1df8,
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
    let menu_vtable = 0x36e81b0;
    assert_eq!(
        rva_u64(image, pe, menu_vtable + 52 * 8),
        base + target_rva(candidate_native::menu::MENU_STEP_TARGET),
        "CMenuUI::DoStep remains vtable slot 52 in the 60-slot 40425 class"
    );
}

#[test]
fn supplied_40425_executable_has_the_exact_identity_and_all_170_sites() {
    let Some(exe) = std::env::var_os(EXE_ENV).map(PathBuf::from) else {
        eprintln!("skipping executable resolution: set {EXE_ENV} to the archived 40425 executable");
        return;
    };
    let profile = Profile::from_toml(PROFILE).unwrap();
    let identity = BuildIdentity::of_file(&exe)
        .unwrap_or_else(|error| panic!("read {}: {error}", exe.display()));
    assert_eq!(
        identity.sha256, SHA256,
        "{EXE_ENV} must point at the public Steam 40425 executable"
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
    for &(name, rva) in TARGETS {
        assert_eq!(target_rva(name), rva, "{name} expected RVA changed");
    }
    assert_native_data(&image, &pe, text_bytes, text_rva);
}
