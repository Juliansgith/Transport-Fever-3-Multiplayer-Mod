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

/// `Obstacle::Obstacle(this, box, cell)`: the street and obstacle raster.
pub const OBSTACLE: &str = "bigmap::Obstacle::Obstacle";

/// The constructor's body from the box copy to its resize call
/// (0x8cea67..0x8ceaee): the counts `floor(extent / cell) + 1`, stored at
/// `+0x40`/`+0x44`, and the 32-bit `imul` the resize is sized with. The
/// hook's cell rule models exactly this; a build that changed it is left
/// alone.
pub const OBSTACLE_BODY_OFFSET: usize = 0x17;
pub const OBSTACLE_BODY: [u8; 135] = [
    0xC5, 0xF8, 0x10, 0x02, // vmovups xmm0,[rdx]
    0xC5, 0xF8, 0x11, 0x41, 0x08, // vmovups [rcx+8],xmm0
    0xC5, 0xFA, 0x11, 0x51, 0x18, // vmovss [rcx+0x18],xmm2
    0x48, 0x83, 0xC1, 0x20, // add rcx,0x20
    0x33, 0xC0, // xor eax,eax
    0x48, 0x89, 0x01, // mov [rcx],rax
    0x48, 0x89, 0x41, 0x08, // mov [rcx+8],rax
    0x48, 0x89, 0x41, 0x10, // mov [rcx+0x10],rax
    0x48, 0x89, 0x41, 0x18, // mov [rcx+0x18],rax
    0xC5, 0xFA, 0x10, 0x52, 0x08, // vmovss xmm2,[rdx+8]
    0xC5, 0xFA, 0x10, 0x42, 0x0C, // vmovss xmm0,[rdx+0xc]
    0xC5, 0xFA, 0x5C, 0x4B, 0x0C, // vsubss xmm1,xmm0,[rbx+0xc]
    0xC5, 0xF2, 0x5E, 0x63, 0x18, // vdivss xmm4,xmm1,[rbx+0x18]
    0xC5, 0x7A, 0x2C, 0xC4, // vcvttss2si r8d,xmm4
    0xC5, 0xEA, 0x5C, 0x43, 0x08, // vsubss xmm0,xmm2,[rbx+8]
    0xC5, 0xFA, 0x5E, 0x4B, 0x18, // vdivss xmm1,xmm0,[rbx+0x18]
    0xC5, 0xFA, 0x2C, 0xD1, // vcvttss2si edx,xmm1
    0x8D, 0x42, 0xFF, // lea eax,[rdx-1]
    0xC5, 0xF8, 0x57, 0xC0, // vxorps xmm0,xmm0,xmm0
    0xC5, 0xF8, 0x2F, 0xC8, // vcomiss xmm1,xmm0
    0x0F, 0x42, 0xD0, // cmovb edx,eax
    0x41, 0x8D, 0x40, 0xFF, // lea eax,[r8-1]
    0xC5, 0xF8, 0x2F, 0xE0, // vcomiss xmm4,xmm0
    0x44, 0x0F, 0x42, 0xC0, // cmovb r8d,eax
    0xFF, 0xC2, // inc edx
    0x89, 0x54, 0x24, 0x38, // mov [rsp+0x38],edx
    0x41, 0x8D, 0x40, 0x01, // lea eax,[r8+1]
    0x89, 0x44, 0x24, 0x3C, // mov [rsp+0x3c],eax
    0x48, 0x8B, 0x44, 0x24, 0x38, // mov rax,[rsp+0x38]
    0x48, 0x89, 0x43, 0x40, // mov [rbx+0x40],rax
    0x48, 0xC1, 0xE8, 0x20, // shr rax,0x20
    0x0F, 0xAF, 0xC2, // imul eax,edx
    0x48, 0x63, 0xD0, // movsxd rdx,eax
    0x45, 0x33, 0xC0, // xor r8d,r8d
];

/// The placement spacing score.
pub const SPACING: &str = "bigmap::placement spacing";

/// The score's two int32 squarings, by their offset in it: candidate
/// pairs (`imul edx,edx; imul eax,eax` at 0x8d334b) and exclusions
/// (`imul ecx,ecx; imul eax,eax` at 0x8d33ab). The hook replaces the score
/// only where both are as read.
pub const SPACING_SQUARES: [(usize, [u8; 6]); 2] = [
    (0x15b, [0x0F, 0xAF, 0xD2, 0x0F, 0xAF, 0xC0]),
    (0x1bb, [0x0F, 0xAF, 0xC9, 0x0F, 0xAF, 0xC0]),
];

/// The score a candidate closer than the minimum gets (`[0x36bbe18]`).
pub const SPACING_TOO_CLOSE: f32 = 99_999.0;

/// The octree's descent (`detail::EcsOctreeIterator`), recursive.
pub const DESCENT: &str = "bigmap::octree descent";

/// The descent's only other caller, which starts it at the root.
pub const DESCENT_START: &str = "bigmap::octree descent start";

/// The descent's body where the depth-12 ids rely on it, by offset:
/// where it reads its box (entry `rsp+0x48`) and its node (`+0x50`), the
/// remaining depth's decrement (`+0x40`) and the leaf stop, and the child
/// id `8·id + 1 + octant` in 32 bits.
pub const DESCENT_CHECKS: [(usize, &[u8]); 4] = [
    (0x85, &[0x4C, 0x8B, 0xAD, 0x60, 0x01, 0x00, 0x00]),
    (0xa7, &[0x8B, 0x9D, 0x68, 0x01, 0x00, 0x00, 0x83, 0xFB, 0xFF]),
    (0x37b, &[0x83, 0xAD, 0x58, 0x01, 0x00, 0x00, 0x01, 0x75, 0x27]),
    (
        0x4c9,
        &[0x44, 0x8D, 0x0C, 0xC5, 0x01, 0x00, 0x00, 0x00, 0x45, 0x03, 0xCC],
    ),
];

/// The starter's reads of the EcsOctree: `mov eax,[rcx+0x24]` (the root
/// entity) at +0x5a and `mov edx,[rcx+0x20]` (the depth) at +0x60, and its
/// call of the descent at +0xf8.
pub const DESCENT_START_CHECKS: [(usize, &[u8]); 2] = [
    (0x5a, &[0x8B, 0x41, 0x24]),
    (0x60, &[0x8B, 0x51, 0x20]),
];
pub const DESCENT_START_CALL: usize = 0xf8;

/// The EcsOctree the descent's `rcx` points at: the root box (min x, y, z,
/// max x, y, z) at +8 and the depth at +0x20.
pub const ECS_OCTREE_BOX: usize = 0x08;
pub const ECS_OCTREE_DEPTH: usize = 0x20;

/// The level decoder's splice: `mov r12d,r9d; cmp edx,r8d`, then `jl` past
/// the loop.
pub const DECODER: &str = "bigmap::octree level decoder";
pub const DECODER_STOLEN: [u8; 6] = [0x45, 0x8B, 0xE1, 0x41, 0x3B, 0xD0];
