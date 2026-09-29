//! Big maps for Transport Fever 3: a prototype of silver2127's Big Maps for
//! TPF2 (the `bigmap/` plugin of TpF2 Multiplayer), in the parts that can be
//! built and tested before anyone has read TF3's executable.
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
//! - [`world`]: the engine facts a size depends on, each with its evidence.
//!   Only TPF2 build 35924's are known; TF3's are measured on release day,
//!   and nothing here assumes they are the same.
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
//!
//! The native patches themselves wait for TF3's executable (docs/DAY_ONE.md);
//! the crate names what each one needs, and the hook fills it in.

pub mod ceilings;
pub mod config;
pub mod features;
pub mod ladder;
pub mod mod_data;
pub mod terms;
pub mod world;
