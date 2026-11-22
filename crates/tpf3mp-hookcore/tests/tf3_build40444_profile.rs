//! Static proof for the public Steam build 40444 profile (Steam build ID
//! 26643808, the update of 2026-11-20). 40425's targets carried over: 159
//! still matched their signatures, eleven were found again
//! (investigation/UPDATE_40444_2026-11-21.md), and every fact the 40425
//! proof asserts is re-read here from the 40444 executable at its new place,
//! with the facts this update changed or added.
//!
//! The executable is optional in CI, but when `TPF3MP_TF3_BUILD40444_EXE` is
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
#[path = "../../../profiles/tf3_build40444_steam_windows/modules.rs"]
mod modules;

#[allow(dead_code, unused_imports)]
#[path = "../../../profiles/tf3_build40444_steam_windows/native.rs"]
mod candidate_native;

/// The industry scheduler's data, which the hook's build script includes
/// apart from `native.rs`.
#[allow(dead_code)]
#[path = "../../../profiles/tf3_build40444_steam_windows/industry_schedule.rs"]
mod industry_schedule;

const PROFILE: &str = include_str!("../../../profiles/tf3_build40444_steam_windows/hooks.toml");
const EXE_ENV: &str = "TPF3MP_TF3_BUILD40444_EXE";
const SHA256: &str = "51a7cafa6b57bee804f1f513a9c58aa7a01a1d4ba3048c46e1fb4daa3d039c49";
const SIZE: u64 = 70_134_624;
const PE_TIMESTAMP: u32 = 0x6af6_fd97;

/// All 170 profile targets, including optional targets, at their RVAs in the
/// public build 26643808 executable (image base 0x140000000).
const TARGETS: &[(&str, u64)] = &[
    ("save: PushCompressor level load", 0x32e224),
    ("save: PushCompressor buffer size", 0x32e384),
    ("GameSim::Step", 0x1597c0),
    ("CGame::Step", 0x11f7e0),
    // The batch interval (crates/tpf3mp-hook/src/interval.rs).
    ("CGame::Sync", 0x11fa80),
    ("CGame::Step/Sync call", 0x11f836),
    ("CGameTime::GetSpeed", 0x2a9920),
    ("GameSim::Step/GetSpeed call", 0x15981e),
    ("UI::CMenuUI::StartSavegame", 0x6a47f0),
    ("UI::CMenuUI::CreatePage", 0x6a4e60),
    ("CommandList::Add::lambda", 0x9da9a0),
    ("CommandList::Add", 0x9dafa0),
    ("WorldBuildProposal apply", 0x9e9850),
    ("ModuleBuilder::MousePressed/Add call", 0x544345),
    ("StreetTerminalBuilder::MousePressed/Add call", 0x595c28),
    ("StreetTerminalBuilder::MousePressed/busy set", 0x595a45),
    ("StreetBuilder::UpdateEngine/Add call", 0x58a4ff),
    ("StreetBuilder::UpdateEngine/capture", 0x58a2b5),
    ("StreetBuilder::UpdateEngine callback/result scan", 0x56ec41),
    ("StreetBuilder::UpdateEngine callback/snap", 0x56eeea),
    ("UI::StreetBuilder::StartAt", 0x571da0),
    ("ecs::Engine::EntityExists", 0x4f8530),
    ("RailroadCrossingComp/type selection Add call", 0x28c723c),
    ("StreetProposal/level crossing maps", 0x4c8fd6),
    (
        "UpdateRailRoadCrossingProposal/crossing map insert",
        0x28c7c9f,
    ),
    ("ProposalAction::DoApply/Add call", 0x54a405),
    ("luaB_print", 0x304a540),
    ("lua_checkstack", 0x303ad90),
    ("lua_createtable", 0x303afc0),
    ("lua_gettop", 0x303b450),
    ("lua_next", 0x303b7c0),
    ("lua_pushboolean", 0x303b910),
    ("lua_pushcclosure", 0x303b930),
    ("lua_pushlstring", 0x303ba80),
    ("lua_pushnil", 0x303bae0),
    ("lua_pushnumber", 0x303bb00),
    ("lua_pushvalue", 0x303bbf0),
    ("lua_rawget", 0x303bcd0),
    ("lua_rawgeti", 0x303bd10),
    ("lua_rawset", 0x303bdf0),
    ("lua_settop", 0x303c230),
    ("lua_toboolean", 0x303c3f0),
    ("lua_tolstring", 0x303c470),
    ("lua_tonumberx", 0x303c510),
    ("lua_touserdata", 0x303c690),
    ("lua_type", 0x303c6d0),
    ("UI::CMenuUI::DoStep", 0x6a1f20),
    ("UI::CMenuUI::DoStep/m_game test", 0x6a1f80),
    ("UI::CMenuUI::DoStep/m_loadGameResult read", 0x6a2908),
    ("lua_pcallk", 0x303b800),
    ("luaL_ref", 0x3031570),
    ("lua_loadfile", 0x301ec40),
    ("lua_cached_loadfile", 0x3025090),
    ("lua_load", 0x303b6b0),
    ("RegisterAppUsertypes", 0xdd0b40),
    ("ecs::LandVehicleMoveSystem::Update2/shuffle", 0xacb770),
    ("ecs::LandVehicleMoveSystem::Update2/records", 0xacb972),
    ("GameSim::Step/paused GameTime advance", 0x159842),
    ("CGameTime::Advance", 0xbb6140),
    ("CGameTime::Advance/tick", 0xbb61c9),
    ("CGameTime::GetTickCount", 0x2a9940),
    ("CGameTime::GetUpdateCount", 0x2a9a00),
    (
        "ecs::SimEntityAtTerminalSystem::Update/vehicles at stop",
        0xb17afc,
    ),
    (
        "ecs::TransportVehicleSystem::GetVehiclesAtLineStop",
        0xb8f760,
    ),
    ("ecs::TransportVehicleSystem::Update2/visit", 0xb94f6b),
    ("FindNextFreeTerminal/candidate sort", 0xb8e680),
    ("ecs::LineSystem::GetData/return", 0xadbc64),
    ("transport::EdgeReservationManager::Reserve", 0x257e920),
    (
        "transport::EdgeReservationManager::Reserve_simple",
        0x257e7a0,
    ),
    ("transport::EdgeUseManager::Add", 0x2580f80),
    ("transport::EdgeUseManager::AddRange", 0x257f2b0),
    ("ecs::Engine::Update", 0x2be6490),
    ("game_script_util::Update/lambda_1::_Do_call", 0xf534f0),
    ("game_script_util::PostUpdate/lambda_1::_Do_call", 0xf52a50),
    (
        "game_script_util::HandleEvent/lambda_1/lambda_1::operator()",
        0xf4f810,
    ),
    ("TownDevelopAt::Apply", 0x9e74e0),
    ("StreetField::At", 0x2ba27d0),
    ("StreetField::At/cache found", 0x2ba288a),
    ("TownUpdateSize::Apply", 0x9e8200),
    ("TownUpdateSize::Apply/develop", 0x9e837a),
    ("TownUpdateSize::Apply/return", 0x9e83c7),
    ("TownDeveloper::Develop", 0x8e4f90),
    ("CommandApply::One", 0x9ea300),
    ("CommandApply::One/return", 0x9ea652),
    ("StreetDeveloper::TryCandidate", 0x9700a0),
    ("StreetDeveloper::TryCandidate/return", 0x97085c),
    ("StreetDeveloper::Reject", 0x96baa0),
    ("StreetDeveloper::BuildStreet/errors", 0x96e16d),
    ("BaseNodeConfig/field offsets", 0x177f707),
    ("StreetProposal/node configuration offsets", 0x22e000f),
    ("BaseNodeConfig/crosswalk set layout", 0xa5222d),
    ("lua_getfield", 0x303b2d0),
    ("probe: GUI GameState getter", 0x6ac770),
    ("probe: engine GameState getter", 0x120400),
    ("probe: ProposalStreetGraph::GetPlayerOwnedPtr", 0xa4f5f0),
    ("probe: street_util IsOwnedByOtherPlayer", 0x6114b0),
    ("destination_util::GetTargetsByLandUse/candidates", 0x8eca35),
    (
        "ecs::SimEntityAtBuildingSystem::Update2/leave batches",
        0xb0f7c2,
    ),
    ("ecs::PersonMoveSystem::Update2/arrival batch", 0xaf69ad),
    ("ecs::SimEntityNeedsPathSystem::Update/list", 0xb219b3),
    (
        "ecs::SimEntityNeedsPathSystem::EntityToBeRemoved/data getter call",
        0xb218c1,
    ),
    ("ecs::Engine::EndModification/free-id append", 0x2be2a51),
    ("UI::StreetBuilder::Step", 0x5864d0),
    ("UI::StreetBuilder ctor/player store", 0x56ae94),
    ("UI::TrackModifier::Step", 0x5cc590),
    ("UI::TrackModifier ctor/player store", 0x5b4968),
    ("UI::Bulldozer::Step", 0x4d6a90),
    ("UI::Bulldozer ctor/filter", 0x4c52e2),
    ("UI::ConstructionBuilder::Step", 0x51d520),
    ("UI::ConstructionBuilder ctor/player store", 0x50bbc8),
    ("UI::StreetTerminalBuilder::Step", 0x596670),
    ("UI::StreetTerminalBuilder ctor/player store", 0x590eb3),
    ("UI::ModuleBuilder::Step", 0x546370),
    ("UI::ModuleBuilder ctor/player store", 0x540c51),
    ("UI::Bulldozer ctor/player store", 0x4c50f1),
    ("probe: StreetBulldozerAction edge test", 0x5f2f30),
    ("probe: bulldozer owner test", 0x5f83c0),
    ("UI::Bulldozer ctor/owner list", 0x4c5185),
    ("UI::Bulldozer set owner list", 0x4d6970),
    (
        "view: HudIconManager::PreemptiveOctreeTraversal/player",
        0x67ce8b,
    ),
    ("view: StationViewer::vf4/player", 0x8430a8),
    ("view: CSelector pick/player", 0x842779),
    ("view: ViewCreator::vf1/player", 0x8702aa),
    ("view: CatchmentAreaHelper/player 1", 0x87f7b5),
    ("view: CatchmentAreaHelper/player 2", 0x8800ec),
    ("view: CatchmentAreaHelper/player 3", 0x880270),
    ("view: CatchmentAreaHelper/player 4", 0x8802b3),
    ("view: CatchmentAreaHelper/owner test", 0x880ce9),
    ("view: LayerManagerColorMap/player 1", 0x884a00),
    ("view: LayerManagerColorMap/player 2", 0x884ad9),
    ("view: LayerManagerColorMap/player 3", 0x884baf),
    ("view: LayerManager colour lambda/player", 0x88c23b),
    ("view: LayerManager colour/owner test", 0x88ee42),
    ("view: react RendererComponentDelegate/player", 0x2a213ca),
    ("view: react RailroadCrossingComp/player", 0x28c51b6),
    ("view: HudIconManager icon pass/owner", 0x675f76),
    ("view: getPlayer binding/push", 0x250e402),
    ("probe: LineViewer route data test", 0x7f9147),
    ("probe: LineViewer GetEdgeGeometries", 0x7f9e00),
    ("view: LineViewer lines of the player/call", 0x7fcc72),
    ("UI::RendererFactory::Create", 0x82f210),
    ("UI::CRendererComponent::AddRenderable", 0x6b0970),
    ("UI::CRendererComponent::RemoveRenderable", 0x6b1e10),
    ("UI::BuilderRenderer::Clear", 0x7c3420),
    ("UI::BuilderRenderer::vf0", 0x7c1830),
    ("builder_renderer_util::AddToRenderer", 0x5e3130),
    ("CreateProposalData", 0xa285c0),
    ("makeProposalData/CreateProposalData call", 0x25333fb),
    ("UI::CGameUI::~CGameUI", 0x650fe0),
    ("UI::CMenuUI::StartGame/CGameUI store", 0x6a6e70),
    ("UI::CGameUI::CreateUI/RendererFactory field", 0x65d414),
    ("UI::CGameUI::CreateUI/mainView store", 0x65c7d4),
    ("ProposalViewer/ModelData read", 0x2acf0c7),
    ("BuilderRenderer::EndHeightMod/upload flag", 0x7c49fa),
    ("UI::BuilderRenderer::EndHeightMod", 0x7c4970),
    ("terrain::ViewTerrain::ApplyBlocks", 0x397a50),
    ("UI::StreetBuilder::ResetProposal", 0x5769c0),
    ("StreetBuilder state swap/ResetProposal call", 0x56e17f),
    ("view: findBestDepot depot owner test", 0x26acf8b),
    ("view: findBestDepot owner test", 0x26ad3d5),
    ("simperf: EmissionGridSystem::Update", 0xab3060),
    ("simperf: EmissionEmitterSystem::Update2", 0xaaeed0),
    ("simperf: TownSystem::Update2", 0xb6b190),
    ("simperf: UpdateParcelCollision", 0x9399f0),
    ("simperf: UpdateParcelCollision call", 0x261fdaa),
    (
        "fast-component-index: Engine::GetComponentDataIndex",
        0xa4f10,
    ),
    ("emission::EmissionGridSystem::Update", 0xab3060),
    ("industry schedule", 0x1517d0),
    ("fast-bitmap-span: BitmapRasterizable::Fill", 0x91c340),
    ("UI::CGameUI::GameStep/CGame::Step call", 0x65faa2),
];

#[test]
fn profile_pins_public_steam_build_40444_and_all_target_names() {
    let profile = Profile::from_toml(PROFILE).unwrap();
    assert_eq!(
        profile.name,
        "Transport Fever 3 Build 40444 (Steam, Windows x64)"
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
        .unwrap_or_else(|| panic!("no 40444 RVA recorded for {name}"))
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
        ("BaseEdge", layout::BASE_EDGE_POOL_VTABLE, 0x22a910),
        ("BaseNode", layout::BASE_NODE_POOL_VTABLE, 0x22aae0),
        (
            "BaseNodeConfig",
            layout::BASE_NODE_CONFIG_POOL_VTABLE,
            0x22ab80,
        ),
        ("Construction", layout::CONSTRUCTION_POOL_VTABLE, 0x22b080),
        (
            "RailroadCrossing",
            layout::RAILROAD_CROSSING_POOL_VTABLE,
            0x22bb00,
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
            0x958fb,
            &[0x48, 0x69, 0xc0, 0x18, 0x01, 0x00, 0x00],
            "imul rax, 0x118 (BaseEdge size)",
        ),
        (
            0x2816fc,
            &[0x48, 0x6b, 0xc0, 0x78],
            "imul rax, 0x78 (BaseNodeConfig size)",
        ),
        (
            0x280bcb,
            &[0x48, 0x69, 0xc0, 0x88, 0x02, 0x00, 0x00],
            "imul rax, 0x288 (Construction size)",
        ),
        (
            0x280a68,
            &[0x48, 0x8b, 0x4b, 0x78],
            "mov rcx, [rbx+0x78] (the pools)",
        ),
        (
            0x280a70,
            &[0x3d, 0x00, 0x00, 0x00, 0x40],
            "cmp eax, 0x40000000 (paged from)",
        ),
        (
            0x280a8c,
            &[0x49, 0x8b, 0x91, 0x80, 0x00, 0x00, 0x00],
            "mov rdx, [r9+0x80] (pages)",
        ),
        (
            0x280abd,
            &[0x49, 0x8b, 0x41, 0x68],
            "mov rax, [r9+0x68] (dense)",
        ),
        (
            0xa4f4c,
            &[0x48, 0x8b, 0x81, 0x90, 0x00, 0x00, 0x00],
            "mov rax, [rcx+0x90] (entity table)",
        ),
        (
            0x2be3b71,
            &[0x48, 0x8b, 0x86, 0xc0, 0x00, 0x00, 0x00],
            "mov rax, [rsi+0xc0] (component bits)",
        ),
        (
            0x5458da,
            &[0xc4, 0xc1, 0x7a, 0x10, 0xad, 0x88, 0x00, 0x00, 0x00],
            "vmovss xmm5, [r13+0x88] (transf x)",
        ),
        (
            0x5458f5,
            &[0xc4, 0x41, 0x7a, 0x10, 0xa5, 0x8c, 0x00, 0x00, 0x00],
            "vmovss xmm12, [r13+0x8c] (transf y)",
        ),
        // The RailroadCrossing store's copy (sub_17f260): its 0x40 stride,
        // and its type index at +0x30.
        (
            0x17f2fd,
            &[0x48, 0xc1, 0xe3, 0x06],
            "shl rbx, 6 (RailroadCrossing size)",
        ),
        (
            0x17f34a,
            &[0x8b, 0x46, 0x30, 0x89, 0x43, 0x30],
            "mov eax, [rsi+0x30]; mov [rbx+0x30], eax (a crossing's type index)",
        ),
        // Its Lua binding's field offsets (the binding at 0x189baae): stateTime 0x38,
        // state 0x34, typeIndex 0x30, edges 0x18, then nodes (r15d, 0).
        (
            0x189baae,
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

/// The target of the `call rel32` (`E8`) at `rva`.
fn call_target(image: &[u8], pe: &PeHeaders, rva: u64) -> u64 {
    let call = rva_bytes(image, pe, rva, 5);
    assert_eq!(call[0], 0xe8, "a call at {rva:#x}");
    let rel = i32::from_le_bytes(call[1..5].try_into().unwrap());
    (rva + 5).wrapping_add_signed(i64::from(rel))
}

/// `expected` at `rva`, `None` a byte any value may take.
fn assert_masked(image: &[u8], pe: &PeHeaders, rva: u64, expected: &[Option<u8>], what: &str) {
    let actual = rva_bytes(image, pe, rva, expected.len());
    for (i, (want, got)) in expected.iter().zip(actual).enumerate() {
        if let Some(want) = want {
            assert_eq!(want, got, "{what}: byte {i} at {rva:#x}");
        }
    }
}

/// Every byte the bundle's modules check at a site before they patch it,
/// and the distances and frame slots they rely on, read from the 40444
/// executable: what the hook finds in the game.
fn assert_bundle_sites(image: &[u8], pe: &PeHeaders) {
    use candidate_native::{
        crossings, drawing, edgewatch, guiplayer, menu, order, persons, probe, streettrace, ticks,
        toolchain, toolplayer, townfield, towntrace,
    };
    let bytes = |rva: u64, expected: &[u8], what: &str| {
        assert_eq!(
            rva_bytes(image, pe, rva, expected.len()),
            expected,
            "{what} at {rva:#x}"
        );
    };
    let at = target_rva;
    let offset = |rva: u64, by: i64| rva.wrapping_add_signed(by);

    // The tools' players: each constructor's store, and the setter.
    for (name, expected) in [
        (
            toolplayer::STREET_STORE,
            &toolplayer::STREET_STORE_BYTES[..],
        ),
        (
            toolplayer::MODIFIER_STORE,
            &toolplayer::MODIFIER_STORE_BYTES,
        ),
        (
            toolplayer::CONSTRUCTION_STORE,
            &toolplayer::CONSTRUCTION_STORE_BYTES,
        ),
        (
            toolplayer::TERMINAL_STORE,
            &toolplayer::TERMINAL_STORE_BYTES,
        ),
        (toolplayer::MODULE_STORE, &toolplayer::MODULE_STORE_BYTES),
        (
            toolplayer::BULLDOZER_LIST,
            &toolplayer::BULLDOZER_LIST_BYTES,
        ),
        (
            toolplayer::BULLDOZER_SETTER,
            &toolplayer::BULLDOZER_SETTER_BYTES,
        ),
    ] {
        bytes(at(name), expected, name);
    }
    assert_masked(
        image,
        pe,
        at(toolplayer::BULLDOZER_FILTER),
        &toolplayer::FILTER_BYTES,
        toolplayer::BULLDOZER_FILTER,
    );
    let wild: Vec<Option<u8>> = toolplayer::BULLDOZER_STORE_BYTES
        .iter()
        .enumerate()
        .map(|(i, b)| (!(3..7).contains(&i)).then_some(*b))
        .collect();
    assert_masked(
        image,
        pe,
        at(toolplayer::BULLDOZER_STORE),
        &wild,
        toolplayer::BULLDOZER_STORE,
    );

    // The persons' and vehicles' order fixes and the free-id trace.
    let site = at(persons::candidates::SITE);
    bytes(
        site,
        &persons::candidates::EXPECTED,
        persons::candidates::SITE,
    );
    for (by, expected) in persons::candidates::CONTEXT {
        bytes(offset(site, *by), expected, "the candidates' context");
    }
    let site = at(persons::departures::SITE);
    bytes(
        site,
        &persons::departures::EXPECTED,
        persons::departures::SITE,
    );
    for (by, expected) in persons::departures::CONTEXT {
        bytes(offset(site, *by), expected, "the departures' context");
    }
    bytes(
        at(persons::arrivals::SITE),
        &persons::arrivals::EXPECTED,
        persons::arrivals::SITE,
    );
    let site = at(persons::needs_path::SITE);
    bytes(
        site,
        &persons::needs_path::EXPECTED,
        persons::needs_path::SITE,
    );
    for (by, expected) in persons::needs_path::CONTEXT {
        bytes(offset(site, *by), expected, "the needs-path context");
    }
    let getter = call_target(image, pe, at(persons::needs_path::GETTER_CALL));
    bytes(
        getter,
        &persons::needs_path::GETTER,
        "the needs-path data getter",
    );
    let site = at(persons::freed_ids::SITE);
    bytes(
        site,
        &persons::freed_ids::EXPECTED,
        persons::freed_ids::SITE,
    );
    bytes(
        offset(site, persons::freed_ids::HEAD_FROM_SITE),
        &persons::freed_ids::HEAD,
        "EndModification's head",
    );
    bytes(
        at("GameSim::Step") + persons::freed_ids::STEP_ENGINE_CALL_AT,
        &persons::freed_ids::STEP_ENGINE_CALL,
        "GameSim::Step's call of Engine::Update",
    );
    for (name, expected) in [
        (order::route_trace::SITE, &order::route_trace::EXPECTED[..]),
        (order::land_vehicle::SITE, &order::land_vehicle::EXPECTED),
        (order::terminal::SITE, &order::terminal::EXPECTED),
        (
            order::platform::VISIT_SITE,
            &order::platform::VISIT_EXPECTED,
        ),
        (
            order::platform::CANDIDATES_SITE,
            &order::platform::CANDIDATES_EXPECTED,
        ),
    ] {
        bytes(at(name), expected, name);
    }
    // The vehicles at a stop: the call right before the site is the getter's.
    assert_eq!(
        call_target(image, pe, at(order::terminal::SITE) - 5),
        at(order::terminal::GETTER),
        "the vehicles-at-stop getter"
    );
    // Update2's node count: its r8d home slot, `[entry rsp+0x18]`, is
    // `[rbp+COUNT]` with 40444's `lea rbp, [rax-0x578]` (40425's frame put
    // it at `+0x5b0`), and the visit loop runs to it.
    let update2 = at(order::platform::VISIT_SITE) - 0x1eb;
    bytes(
        update2,
        &[
            0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x20, 0x44, 0x89, 0x40, 0x18,
        ],
        "Update2 spills its node count to its home slot",
    );
    bytes(
        update2 + 0x16,
        &[0x48, 0x8d, 0xa8, 0x88, 0xfa, 0xff, 0xff],
        "Update2's frame: lea rbp, [rax-0x578]",
    );
    assert_eq!(order::platform::COUNT, 0x578 + 0x18);
    bytes(
        update2 + 0x1c3,
        &[0x39, 0x9d, 0x90, 0x05, 0x00, 0x00],
        "the visit loop's bound, cmp [rbp+COUNT], ebx",
    );

    // The edge watch, the street trace, the tick counter, the town traces.
    let apply = at(edgewatch::APPLY);
    bytes(apply, &edgewatch::ENTRY_EXPECTED, edgewatch::APPLY);
    bytes(
        apply + edgewatch::KIND_READ_AT,
        &edgewatch::KIND_READ,
        "the payload kind's read",
    );
    assert_eq!(
        apply + edgewatch::RETURN_SITE_AT,
        at(edgewatch::RETURN_SITE)
    );
    bytes(
        at(edgewatch::RETURN_SITE),
        &edgewatch::RETURN_EXPECTED,
        edgewatch::RETURN_SITE,
    );
    for (call, path) in edgewatch::PATHS {
        assert_eq!(call_target(image, pe, call), apply, "the {path} path");
    }
    let try_candidate = at(streettrace::TRY);
    bytes(try_candidate, &streettrace::TRY_EXPECTED, streettrace::TRY);
    assert_eq!(
        try_candidate + streettrace::TRY_RETURN_AT,
        at(streettrace::TRY_RETURN)
    );
    bytes(
        at(streettrace::TRY_RETURN),
        &streettrace::TRY_RETURN_EXPECTED,
        streettrace::TRY_RETURN,
    );
    bytes(
        at(streettrace::REJECT),
        &streettrace::REJECT_EXPECTED,
        streettrace::REJECT,
    );
    for call in streettrace::REJECT_CALLS_AT {
        assert_eq!(
            call_target(image, pe, try_candidate + call),
            at(streettrace::REJECT),
            "TryCandidate's reject call +{call:#x}"
        );
    }
    bytes(
        at(streettrace::ERRORS),
        &streettrace::ERRORS_EXPECTED,
        streettrace::ERRORS,
    );
    assert_eq!(
        at(ticks::ADVANCE) + ticks::TICK_OFFSET,
        at(ticks::ADVANCE_TICK)
    );
    bytes(
        at(ticks::ADVANCE_TICK),
        &ticks::TICK_BYTES,
        ticks::ADVANCE_TICK,
    );
    let field = at(townfield::FIELD);
    assert_eq!(field + townfield::SITE_AT, at(townfield::SITE));
    bytes(at(townfield::SITE), &townfield::EXPECTED, townfield::SITE);
    // Its `jne rel8`, the last two bytes, reaches the miss path.
    let len = townfield::EXPECTED.len() as u64;
    let rel = u64::from(townfield::EXPECTED[townfield::EXPECTED.len() - 1]);
    assert_eq!(
        field + townfield::SITE_AT + len + rel,
        field + townfield::MISS_AT
    );
    let town = at(towntrace::APPLY);
    assert_eq!(
        town + towntrace::DEVELOP_SITE_AT,
        at(towntrace::DEVELOP_SITE)
    );
    assert_eq!(town + towntrace::RETURN_SITE_AT, at(towntrace::RETURN_SITE));
    bytes(
        at(towntrace::DEVELOP_SITE),
        &towntrace::DEVELOP_SITE_EXPECTED,
        towntrace::DEVELOP_SITE,
    );
    bytes(
        at(towntrace::RETURN_SITE),
        &towntrace::RETURN_EXPECTED,
        towntrace::RETURN_SITE,
    );
    bytes(
        at(towntrace::DEVELOP),
        &towntrace::DEVELOP_EXPECTED,
        towntrace::DEVELOP,
    );
    assert_eq!(
        call_target(image, pe, town + towntrace::DEVELOP_CALL_AT),
        at(towntrace::DEVELOP)
    );

    // The crossing map's insert, the line viewers, getPlayer, the probe.
    bytes(
        call_target(image, pe, at(crossings::MAP_INSERT_CALL)),
        crossings::INSERT_HEAD,
        "the crossing map's insert",
    );
    bytes(
        call_target(image, pe, at(guiplayer::LINES_SITE)),
        &guiplayer::LINES_CALLEE_BYTES,
        "the line system's player-to-lines index",
    );
    bytes(
        call_target(image, pe, at(guiplayer::GET_PLAYER_PUSH)),
        &guiplayer::PUSH_BYTES,
        "the Lua integer push",
    );
    bytes(
        at(probe::VIEWER_TARGET),
        &probe::VIEWER_BYTES,
        probe::VIEWER_TARGET,
    );

    // The street and track tools' chain: the click's capture and call, the
    // callback's two pieces (40444 recompiled it: 0x2a9 apart, `StartAt`'s
    // call 0x58 into the second), `StartAt`'s revision read.
    assert_eq!(
        at(toolchain::CAPTURE) + toolchain::CAPTURE_TO_ADD_CALL as u64,
        at(toolchain::ADD_CALL)
    );
    assert_eq!(
        call_target(image, pe, at(toolchain::ADD_CALL)),
        at("CommandList::Add")
    );
    assert_eq!(
        at(toolchain::RESULT_SCAN) + toolchain::SCAN_TO_SNAP as u64,
        at(toolchain::SNAP)
    );
    assert_eq!(
        call_target(image, pe, at(toolchain::SNAP) + toolchain::SNAP_CALL as u64),
        at(toolchain::START_AT)
    );
    let revision = call_target(
        image,
        pe,
        at(toolchain::START_AT) + toolchain::START_AT_REVISION_CALL as u64,
    );
    assert_eq!(
        call_target(image, pe, revision + toolchain::REVISION_EXISTS_CALL as u64),
        at(toolchain::ENTITY_EXISTS)
    );

    // The industry scheduler's shared state: its control block's vtable,
    // `std::_Ref_count_obj2<IndustryChainSpawnState>`, slot 0 destroying the
    // payload and slot 1 deleting the block.
    let base = pe.image_base;
    assert_eq!(
        rva_u64(image, pe, industry_schedule::STATE_VTABLE),
        base + industry_schedule::STATE_DESTROY
    );
    assert_eq!(
        rva_u64(image, pe, industry_schedule::STATE_VTABLE + 8),
        base + industry_schedule::STATE_DELETE
    );
    bytes(
        at(industry_schedule::SITES[0].0),
        industry_schedule::SITES[0].1,
        industry_schedule::SITES[0].0,
    );

    // The drawing anchors: the opcode before the displacement the hook reads,
    // and the fields, where 40425 had them.
    for ((name, opcode), field) in [
        (drawing::GAME_UI_FIELD, 0x8e0),
        (drawing::FACTORY_FIELD, 0x7b0),
        (drawing::MAIN_VIEW_FIELD, 0xe18),
        (drawing::MODEL_DATA_FIELD, 0x760),
        (drawing::UPLOAD_FIELD, 0xf0),
    ] {
        let rva = at(name);
        bytes(rva, opcode, name);
        let disp = rva_bytes(image, pe, rva + opcode.len() as u64, 4);
        assert_eq!(
            u32::from_le_bytes(disp.try_into().unwrap()),
            field,
            "{name}'s field"
        );
    }

    // `RegisterAppUsertypes` takes five arguments on 40444: a bool in r9b
    // and, on the stack, a `shared_ptr` by value. After its seven pushes and
    // `lea rbp, [rsp-0x4c0]`, `[rbp+0x520]` is the entry's `[rsp+0x28]`.
    // The hook's detour passes that stack argument through (menu.rs).
    let register = at(menu::REGISTER_APP_TARGET);
    bytes(
        register + 0x10,
        &[0x48, 0x8d, 0xac, 0x24, 0x40, 0xfb, 0xff, 0xff],
        "RegisterAppUsertypes' frame",
    );
    bytes(
        register + 0x30,
        &[
            0x41, 0x0f, 0xb6, 0xf9, 0x49, 0x8b, 0xd8, 0x48, 0x8b, 0xf2, 0x4c, 0x8b, 0xf9, 0x48,
            0x8b, 0x85, 0x20, 0x05, 0x00, 0x00,
        ],
        "RegisterAppUsertypes reads a fourth and a fifth argument",
    );
}

fn assert_native_data(image: &[u8], pe: &PeHeaders, text: &[u8], text_rva: u64) {
    // Both compression and decompression read this constant. Rewriting the
    // compressor's instruction must leave the constant (and loader) alone.
    assert_eq!(rva_bytes(image, pe, 0x3814b98, 4), 3u32.to_le_bytes());
    assert_eq!(
        rva_bytes(image, pe, 0x32e224, 6),
        candidate_native::savefast::LEVEL_LOAD_BYTES
    );
    assert_eq!(
        rva_bytes(image, pe, 0x32e384, 6),
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
        0x177dd30,
        candidate_native::network::NODE_CONFIG_VTABLE as u64,
        0x177de30,
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
        rva_bytes(image, pe, 0x178ea8d, 5),
        &[0xba, 0x28, 0x01, 0x00, 0x00],
        "BaseEdge UserdataValue allocation size"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x178eae2, 4),
        &[0x48, 0x8d, 0x5e, 0x10],
        "BaseEdge inline component payload address"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x178eafd, 4),
        &[0x48, 0x89, 0x5e, 0x08],
        "BaseEdge UserdataValue payload pointer"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x176b83b, 5),
        &[0xba, 0x88, 0x00, 0x00, 0x00],
        "BaseNodeConfig UserdataValue allocation size"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x176b892, 4),
        &[0x4c, 0x8d, 0x47, 0x10],
        "BaseNodeConfig inline component payload address"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x176b950, 4),
        &[0x4c, 0x89, 0x47, 0x08],
        "BaseNodeConfig UserdataValue payload pointer"
    );
    assert_eq!(
        candidate_native::network::LANE_VECTOR,
        0x70,
        "BaseEdge laneConfigs field offset"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x177f3e1, 7),
        &[0xc7, 0x45, 0xb0, 0x70, 0x00, 0x00, 0x00],
        "BaseEdge laneConfigs registered field offset"
    );
    assert_eq!(
        rva_bytes(image, pe, 0x177f4c5, 0x1b),
        &[
            0x48, 0x8d, 0x45, 0xb0, 0x48, 0x89, 0x84, 0x24, 0xb0, 0x00, 0x00, 0x00, 0x48, 0x8d,
            0x05, 0x10, 0xe6, 0xfd, 0x01, 0x48, 0x89, 0x84, 0x24, 0xa8, 0x00, 0x00, 0x00,
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
            0x374c688,
            11,
            candidate_native::simperf::EMISSION_GRID_SLOT,
        ),
        (
            candidate_native::simperf::EMISSION_EMITTERS,
            0x374c000,
            12,
            candidate_native::simperf::EMISSION_EMITTERS_SLOT,
        ),
        (
            candidate_native::simperf::TOWNS,
            0x3758738,
            12,
            candidate_native::simperf::TOWNS_SLOT,
        ),
    ] {
        let function = target_rva(name);
        assert_eq!(slot_rva, table + slot * 8, "{name} vtable slot RVA");
        assert_eq!(
            rva_u64(image, pe, slot_rva),
            base + function,
            "{name} vtable slot must call its 40444 target"
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
    let menu_vtable = 0x3717360;
    assert_eq!(
        rva_u64(image, pe, menu_vtable + 52 * 8),
        base + target_rva(candidate_native::menu::MENU_STEP_TARGET),
        "CMenuUI::DoStep remains vtable slot 52 in the 60-slot 40444 class"
    );
}

#[test]
fn supplied_40444_executable_has_the_exact_identity_and_all_170_sites() {
    let Some(exe) = std::env::var_os(EXE_ENV).map(PathBuf::from) else {
        eprintln!("skipping executable resolution: set {EXE_ENV} to the archived 40444 executable");
        return;
    };
    let profile = Profile::from_toml(PROFILE).unwrap();
    let identity = BuildIdentity::of_file(&exe)
        .unwrap_or_else(|error| panic!("read {}: {error}", exe.display()));
    assert_eq!(
        identity.sha256, SHA256,
        "{EXE_ENV} must point at the public Steam 40444 executable"
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
    assert_bundle_sites(&image, &pe);
}
