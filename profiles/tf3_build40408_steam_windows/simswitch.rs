//! Native data for Steam Windows build 40408's simulation-changing big-map
//! switches, a proposal (investigation/TF3_BIGMAP_SIM_COST_2026-10-05.md,
//! "Switches built"). See hooks.toml for identity and the targets' bytes.

/// `ecs::EmissionGridSystem::Update(Engine*, INodeList*, float dt) const`
/// (0xaa9230): the noise and pollution grids' diffusion, wind and average.
pub const GRID_UPDATE: &str = "bigmap::EmissionGridSystem::Update";

/// `lea rax,[EmissionGridSystem::vftable]` in the system's constructor
/// (0xaa7ea9).
pub const GRID_VTABLE: &str = "bigmap::EmissionGridSystem vtable load";

/// `Update`'s slot in that vtable: `ecs::Engine::Update` (0x2bb8a50) calls
/// every system's slot 11 (`call [r9+0x58]`) in turn, on the step's thread.
pub const GRID_SLOT: usize = 11;

/// `ecs::EmissionEmitterSystem::Update2(Engine*, int, float dt) const`
/// (0xaa51c0): every emitter splatted into both grids.
pub const EMITTER_UPDATE2: &str = "bigmap::EmissionEmitterSystem::Update2";

/// `lea rax,[EmissionEmitterSystem::vftable]` in the system's constructor
/// (0xaa402f).
pub const EMITTER_VTABLE: &str = "bigmap::EmissionEmitterSystem vtable load";

/// `Update2`'s slot: the system's own slot 11 (0xaa5920) calls it through
/// the vtable (`call [r9+0x60]`).
pub const EMITTER_SLOT: usize = 12;

/// `lea rax,[rip+disp32]`: the vtable loads' first three bytes.
pub const LEA_RAX_RIP: [u8; 3] = [0x48, 0x8D, 0x05];

/// `ecs::component::CreateEmissionGrid` (0xba0170, EmissionGrid.cpp): the
/// noise and pollution grids of a new world, 16 cells per tile. Called only
/// from the new-game path (0x155630); a load takes the grid from the save.
pub const CREATE_GRID: &str = "bigmap::CreateEmissionGrid";

/// Where the checked body starts, from the function: the first `shl`.
pub const CREATE_GRID_BODY_OFFSET: usize = 0x91;

/// The body from the first `shl` to the gridPointSize multiplies
/// (0xba0201..0xba026b): `X0`, `Y0`, `width`, `height` as `tiles << 4`,
/// the `> 2` checks, the two tile-size calls, then `vmovss xmm2,[0.0625]`
/// and the two multiplies. The load's displacement (`+0x5d..+0x61` here)
/// is checked through the constant it reads instead.
pub const CREATE_GRID_BODY: [u8; 106] = [
    0xC1, 0xE0, 0x04, 0x89, 0x03, 0x49, 0x8B, 0xCF, 0xE8, 0x12, 0xBD, 0x00, 0x00, 0x48, 0xC1, 0xE8,
    0x20, 0xC1, 0xE0, 0x04, 0x89, 0x43, 0x04, 0x41, 0x8B, 0x07, 0xC1, 0xE0, 0x04, 0x89, 0x43, 0x08,
    0x41, 0x8B, 0x4F, 0x04, 0xC1, 0xE1, 0x04, 0x89, 0x4B, 0x0C, 0x83, 0xF8, 0x02, 0x0F, 0x8E, 0xB1,
    0x01, 0x00, 0x00, 0x83, 0xF9, 0x02, 0x0F, 0x8E, 0x87, 0x01, 0x00, 0x00, 0x49, 0x8B, 0xCF, 0xE8,
    0xAB, 0xBC, 0x00, 0x00, 0x48, 0x89, 0x44, 0x24, 0x34, 0x49, 0x8B, 0xCF, 0xE8, 0x9E, 0xBC, 0x00,
    0x00, 0x48, 0x89, 0x84, 0x24, 0xB8, 0x00, 0x00, 0x00, 0xC5, 0xFA, 0x10, 0x15, 0x02, 0x38, 0xB0,
    0x02, 0xC5, 0xEA, 0x59, 0x9C, 0x24, 0xB8, 0x00, 0x00, 0x00,
];

/// The four `shl reg,4` (cells per tile), from the function: X0, Y0,
/// width (`shl eax,4`) and height (`shl ecx,4`).
pub const CREATE_GRID_SHIFTS: [usize; 4] = [0x91, 0xa2, 0xab, 0xb5];

/// `vmovss xmm2,[rip+disp32]` loading the cell factor, from the function
/// (0xba025a); 8 bytes, the displacement in the last four.
pub const CREATE_GRID_FACTOR_LOAD: usize = 0xea;

/// The load's opcode bytes before its displacement.
pub const VMOVSS_XMM2_RIP: [u8; 4] = [0xC5, 0xFA, 0x10, 0x15];

/// From the 0.0625f the load reads (0x36a3a64, shared by ten other
/// instructions, never written) to a 0.125f in the same section
/// (0x36b2fa4, read by 22 instructions), which the load is pointed at.
pub const CREATE_GRID_COARSE_FACTOR_DELTA: usize = 0x36b2fa4 - 0x36a3a64;
