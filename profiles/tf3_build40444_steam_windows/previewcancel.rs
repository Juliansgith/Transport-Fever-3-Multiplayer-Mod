//! Native target for Steam Windows build 40444. See hooks.toml for identity.

pub const RESET_TARGET: &str = "UI::StreetBuilder::ResetProposal";

/// The profile's name for the tool's game-state swap callback's call of
/// [`RESET_TARGET`] (the constructor's lambda_11), which resets when an
/// entity the proposal snaps to changed revision. The reset returns 5 bytes
/// past it.
pub const SWAP_RESET_CALL: &str = "StreetBuilder state swap/ResetProposal call";
