//! Native data for Steam Windows build 40408's big-map patches
//! (investigation/TF3_BIGMAPS_256KM_2026-10-05.md). See hooks.toml for
//! identity and the targets' bytes.

/// `OctreeSystem::Resize(octree, depth, half)`: stores the root box and
/// the depth, nothing else.
pub const RESIZE: &str = "bigmap::OctreeSystem::Resize";

/// The octree root's site on the new-game path (`InitGame`).
pub const ROOT_INIT: &str = "bigmap::octree_root_init";

/// The octree root's site on the load path (`GameState::Load`).
pub const ROOT_LOAD: &str = "bigmap::octree_root_load";

/// Where the site's `call Resize` is, from the site.
pub const ROOT_CALL_OFFSET: usize = 17;

/// Where the instruction after the call is, from the site: the splice.
pub const ROOT_NEXT_OFFSET: usize = 22;

/// `mov edx, 10` at the site's +12: the stock large-map depth.
pub const ROOT_DEPTH_BYTES: [u8; 5] = [0xBA, 0x0A, 0x00, 0x00, 0x00];

/// Where the site's owner load, `mov rcx,[owner+0x38]`, is, from the site.
pub const ROOT_OWNER_OFFSET: usize = 8;

/// The owner's field holding the `OctreeSystem*`.
pub const OCTREE_FIELD: usize = 0x38;

/// A general-purpose register a site keeps a pointer in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg {
    Rbp,
    Rsi,
}

/// One root site: which register holds the world's tile counts (two
/// `int32`, x then y), which the owner, the owner load's bytes, and the
/// instruction after the call, which is spliced (5 bytes, no RIP-relative
/// operand).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootSite {
    pub target: &'static str,
    pub tiles: Reg,
    pub owner: Reg,
    pub owner_load: [u8; 4],
    pub next: [u8; 5],
}

/// init (0x244b8b): tiles in rbp, owner in rsi; next `vmovss xmm2,[rbp+0x20]`.
pub const INIT_SITE: RootSite = RootSite {
    target: ROOT_INIT,
    tiles: Reg::Rbp,
    owner: Reg::Rsi,
    owner_load: [0x48, 0x8B, 0x4E, 0x38],
    next: [0xC5, 0xFA, 0x10, 0x55, 0x20],
};

/// load (0x20267d): tiles in rsi, owner in rbp; next `vmovss xmm2,[rsi+0x20]`.
pub const LOAD_SITE: RootSite = RootSite {
    target: ROOT_LOAD,
    tiles: Reg::Rsi,
    owner: Reg::Rbp,
    owner_load: [0x48, 0x8B, 0x4D, 0x38],
    next: [0xC5, 0xFA, 0x10, 0x56, 0x20],
};

/// Where `Resize` stores the depth, in the `OctreeSystem`.
pub const OCTREE_DEPTH_FIELD: usize = 0x28;

/// Where `Resize` stores `+half` (x), in the `OctreeSystem`.
pub const OCTREE_HALF_FIELD: usize = 0x1c;
