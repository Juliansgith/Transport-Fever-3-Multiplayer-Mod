//! Big maps for Transport Fever 3, after silver2127's Big Maps for TPF2
//! (tpf2-bigmap, also the `bigmap/` plugin of TpF2 Multiplayer): its
//! design and its measurements, carried to TF3 stage by stage
//! (investigation/TF3_BIGMAPS_PORT_2026-10-01.md, section 4). No code is
//! taken from it.
//!
//! Big Maps gives the New Game menu sizes past the game's own, up to 128 km
//! a side, and gets the engine past the ceilings such sizes hit. On TPF2
//! those were, in the order a growing map meets them: the street raster's
//! 32-bit cell count (180 tiles), the octree's constant root box (±32,768 m,
//! 256 tiles), and the memory generation needs (2.5 MB per km²). The
//! plugin's other features are density levels for towns and industries,
//! a placement budget, and memory and load-time work on the terrain caches
//! ([docs/BIGMAPS.md](../../docs/BIGMAPS.md) has what carries over).
//!
//! What this crate holds, all pure and host-independent:
//!
//! - [`world`]: the engine facts a size depends on, each with its evidence:
//!   TPF2 build 35924's from Big Maps' measurements, TF3 build 40408's from
//!   its executable and scripts
//!   (investigation/TF3_BIGMAPS_PORT_2026-10-01.md) and its memory law
//!   from Stage 0's runs.
//! - [`memory_gate`]: the constants of the New Game page's memory gate (MB
//!   per km² and the game's own use), measured by Stage 0 on one climate,
//!   and the one place to recalibrate them.
//! - [`stock`]: TF3's own sizes and the walls stock TF3 stays inside.
//! - [`ladder`]: the added map sizes and the shapes the ratio dropdown gives
//!   each one.
//! - [`ceilings`]: what a size costs and which ceilings it hits, so a size
//!   the engine cannot build is refused before generation, not after.
//! - [`config`]: the settings file. Every feature ships off.
//! - [`terms`]: what every player in a room must share, with a fingerprint
//!   the room compares.
//! - [`features`]: which features a build can run, from the hook's resolved
//!   profile. A feature that changes the simulation and cannot run on this
//!   build refuses the room; one that only changes what a player sees is
//!   left off with a reason.
//! - [`mod_data`]: the ladder as the data file of the mod,
//!   `mod/tpf3mp_bigmap_1`, which holds the menu's side.
//! - [`page`]: the mod's copy of TF3's New Game settings page, the game's
//!   file plus marked blocks (stage 1).
//! - [`measure`]: what a game log says about generating, entering and
//!   saving a map (stage 0).
//!
//! Stage 1 needs no native patch: its sizes stay inside every wall stock
//! TF3 has. The native patches of the later stages are named in
//! [`features`]; the hook fills them in.

pub mod ceilings;
pub mod config;
pub mod features;
pub mod ladder;
pub mod measure;
pub mod memory_gate;
pub mod mod_data;
pub mod page;
pub mod stock;
pub mod terms;
pub mod world;
