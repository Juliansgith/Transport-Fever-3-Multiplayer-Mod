//! Native data for Steam Windows build 40444's fast emission grid
//! (crates/tpf3mp-hook/src/emission; investigation/
//! TF3_SIM_COST_2026-10-05.md §1). Every address is relative to
//! `ecs::EmissionGridSystem::Update`, the one profile target; see hooks.toml
//! for its signature.

/// `ecs::EmissionGridSystem::Update(this, engine, nodes, dt)` (0xab3060).
pub const UPDATE: &str = "emission::EmissionGridSystem::Update";

/// A stretch of the game's code the fast path models, by its FNV-1a 64
/// hash: anything else there leaves the game's own update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Code {
    pub what: &'static str,
    /// From `Update`, in bytes.
    pub offset: i64,
    pub len: usize,
    pub fnv1a: u64,
}

/// `LoopImpl` over Diffuse's rows (0xab1800).
pub const DIFFUSE_DISPATCH: i64 = -0x1860;
/// `LoopImpl` over Wind's rows (0xab19b0), pollution only.
pub const WIND_DISPATCH: i64 = -0x16b0;
/// `LoopImpl` over Average's rows (0xab1b50).
pub const AVERAGE_DISPATCH: i64 = -0x1510;
/// The row kernels (`Diffuse` 0xab23a0, `Wind` 0xab3520, `Average`
/// 0xab1fc0), as each dispatcher's inline path calls them.
pub const DIFFUSE_KERNEL: i64 = -0xcc0;
pub const WIND_KERNEL: i64 = 0x4c0;
pub const AVERAGE_KERNEL: i64 = -0x10a0;

/// `Update`'s three `call <dispatcher>` sites (0xab3252, 0xab32fe,
/// 0xab337c), which the hook redirects.
pub const DIFFUSE_CALL: i64 = 0x1f2;
pub const WIND_CALL: i64 = 0x29e;
pub const AVERAGE_CALL: i64 = 0x31c;

/// The modelled code: `Update`, the three dispatchers and the three
/// kernels, whole. On 40444 each is 40408's instruction for instruction:
/// with 40408's call and data targets put back, the bytes hash to the
/// 40408 bundle's values (investigation/UPDATE_40444_2026-11-21.md).
pub const CODE: [Code; 7] = [
    Code {
        what: "EmissionGridSystem::Update",
        offset: 0,
        len: 0x4b2,
        fnv1a: 0xcf2d_3658_c197_5b52,
    },
    Code {
        what: "the Diffuse dispatcher",
        offset: DIFFUSE_DISPATCH,
        len: 0x1a8,
        fnv1a: 0x31a5_0c18_e13e_7729,
    },
    Code {
        what: "the Wind dispatcher",
        offset: WIND_DISPATCH,
        len: 0x197,
        fnv1a: 0x0094_3ae1_2350_b36b,
    },
    Code {
        what: "the Average dispatcher",
        offset: AVERAGE_DISPATCH,
        len: 0x170,
        fnv1a: 0x9609_65ca_4fa2_5525,
    },
    Code {
        what: "the Diffuse kernel",
        offset: DIFFUSE_KERNEL,
        len: 0x416,
        fnv1a: 0x8ddf_fc7b_e656_22d1,
    },
    Code {
        what: "the Average kernel",
        offset: AVERAGE_KERNEL,
        len: 0x1b5,
        fnv1a: 0x48f1_8e1e_d16f_71fd,
    },
    Code {
        what: "the Wind kernel",
        offset: WIND_KERNEL,
        len: 0x4e2,
        fnv1a: 0x5f24_a79a_4c47_a105,
    },
];

/// The component's fields (`ecs::component::EmissionGrid`): the grid
/// point size (2 x f32), the concentration grid, the average grid, the
/// wind (2 x f32). A grid is `{i32 x0, i32 y0, i32 width, i32 height,
/// std::vector<float>}` (0x28 bytes).
pub const COMP_GRID_POINT_SIZE: usize = 0x10;
pub const COMP_CONCENTRATION: usize = 0x18;
pub const COMP_AVERAGE: usize = 0x40;
pub const COMP_WIND: usize = 0x6c;
/// The system's fields: the shared temporary grid and Average's,
/// Diffuse's decay and Diffuse's spread coefficients (f32).
pub const SYSTEM_TEMP: usize = 0x18;
pub const SYSTEM_AVERAGE_C: usize = 0x48;
pub const SYSTEM_DECAY_B: usize = 0x4c;
pub const SYSTEM_SPREAD_A: usize = 0x50;
/// Where a grid keeps its width, height and data vector.
pub const GRID_WIDTH: usize = 8;
pub const GRID_HEIGHT: usize = 0xc;
pub const GRID_DATA: usize = 0x10;
