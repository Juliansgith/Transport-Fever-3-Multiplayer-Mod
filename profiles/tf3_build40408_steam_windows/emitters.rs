//! Native data for Steam Windows build 40408's banded emitter splat
//! (crates/tpf3mp-hook/src/emitters; investigation/
//! TF3_EMITTER_SPLAT_2026-10-06.md). Every address is relative to
//! `ecs::EmissionEmitterSystem::Update2`, the one profile target; see
//! hooks.toml for its signature.

/// `ecs::EmissionEmitterSystem::Update2(this, engine, int count, float dt)`
/// (0xaa51c0, vtable 0x1436fc430 slot 12).
pub const UPDATE2: &str = "emitters::EmissionEmitterSystem::Update2";

/// A stretch of the game's code the banded splat models, by its FNV-1a
/// 64 hash: anything else there leaves the game's own update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Code {
    pub what: &'static str,
    /// From `Update2`, in bytes.
    pub offset: i64,
    pub len: usize,
    pub fnv1a: u64,
}

/// `LoopImpl` over lambda_1 (0xaa3e70): buckets the emitters into the
/// 16 regions; `(pool, &lambda, count, 1024, &flag, byte, byte)`.
pub const LAMBDA1_DISPATCH: i64 = -0x1350;
/// Lambda_2's loop over its tasks (0xaa4ab0), `(&lambda, first, end)`:
/// the inline path, with one pool thread.
pub const LAMBDA2_LOOP: i64 = -0x710;
/// `LoopImpl` over lambda_2 (0xaa33a0), `(pool, &lambda, 32, 1, &futures,
/// byte)`: the pool path; `Update2` then waits on the futures it added.
pub const LAMBDA2_POOL: i64 = -0x1e20;

/// `Update2`'s three calls the hook redirects (0xaa56cd, 0xaa5743,
/// 0xaa57dd).
pub const LAMBDA1_CALL: i64 = 0x50d;
pub const LAMBDA2_LOOP_CALL: i64 = 0x583;
pub const LAMBDA2_POOL_CALL: i64 = 0x61d;

/// The `logf` thunk lambda_2's body calls (0x1431873c5, `jmp [rip+x]`),
/// and the import slot it jumps through (0x143665bf0).
pub const LOGF_THUNK: i64 = 0x26e_2205;
pub const LOGF_SLOT: i64 = 0x2bc_0a30;

/// The modelled code: `Update2`, its lambdas and dispatchers, the
/// bucket bookkeeping that fixes the order the emitters are walked in, and
/// the insert helpers, whole; and the `logf` thunk.
pub const CODE: [Code; 16] = [
    Code {
        what: "EmissionEmitterSystem::Update2",
        offset: 0,
        len: 0x758,
        fnv1a: 0x6060_3300_75a4_6ca4,
    },
    Code {
        what: "lambda_1's dispatcher",
        offset: LAMBDA1_DISPATCH,
        len: 0x166,
        fnv1a: 0xad86_6e7f_310c_4490,
    },
    Code {
        what: "lambda_1",
        offset: -0xeb0,
        len: 0x331,
        fnv1a: 0x5b1c_57ad_9b1f_3633,
    },
    Code {
        what: "lambda_2's loop",
        offset: LAMBDA2_LOOP,
        len: 0x25d,
        fnv1a: 0xe25a_cf5c_7525_647f,
    },
    Code {
        what: "lambda_2's pool dispatcher",
        offset: LAMBDA2_POOL,
        len: 0x1c5,
        fnv1a: 0xb8b5_5100_7581_d6fd,
    },
    Code {
        what: "LoopResults' record of a chunk",
        offset: -0x420,
        len: 0x14e,
        fnv1a: 0x4eb7_c5ba_6582_af78,
    },
    Code {
        what: "the bucket walk",
        offset: -0x2d0,
        len: 0x2c3,
        fnv1a: 0x4bee_ad1e_59eb_5dfd,
    },
    Code {
        what: "LoopResults' iterator",
        offset: -0x1110,
        len: 0x1ea,
        fnv1a: 0xb2ea_c00b_46a4_71fc,
    },
    Code {
        what: "the records' sort",
        offset: -0x7c_0b60,
        len: 0x263,
        fnv1a: 0x6399_477c_9049_55fb,
    },
    Code {
        what: "lambda_2's std::function call",
        offset: 0xa60,
        len: 0x2f,
        fnv1a: 0xf3ff_9e3f_9595_2827,
    },
    Code {
        what: "lambda_2's body",
        offset: -0xb70,
        len: 0x45a,
        fnv1a: 0xade5_8041_6f86_10a3,
    },
    Code {
        what: "the cell of a position",
        offset: 0xf_c9e0,
        len: 0x85,
        fnv1a: 0xf4b5_2d68_25c8_2f44,
    },
    Code {
        what: "the point insert",
        offset: 0xf_cc80,
        len: 0xfe,
        fnv1a: 0x0126_b8c6_7d28_886a,
    },
    Code {
        what: "the four corners",
        offset: 0xf_cd80,
        len: 0x2e8,
        fnv1a: 0x94da_8855_7b21_205d,
    },
    Code {
        what: "the radius insert",
        offset: 0xf_d070,
        len: 0x237,
        fnv1a: 0xf73d_558a_a9d2_a7c7,
    },
    Code {
        what: "the logf thunk",
        offset: LOGF_THUNK,
        len: 6,
        fnv1a: 0x6ec5_6f39_81f6_4199,
    },
];

/// The system's fields: `[+8]` the node list's vector of `{entity, int
/// component index}`; `[+0x10]` the component storage's vector of
/// emitters (36 bytes each).
pub const SYSTEM_NODES: usize = 8;
pub const SYSTEM_DATA: usize = 0x10;
/// The `EmissionGrid` component: the grid point size (2 x f32) and the
/// concentration `Grid<float>` (`{x0, y0, width, height, std::vector}`).
pub const COMP_GRID_POINT_SIZE: usize = 0x10;
pub const COMP_CONCENTRATION: usize = 0x18;
