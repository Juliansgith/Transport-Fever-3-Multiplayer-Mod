//! The AI players' minds (docs/AI_PLAYERS.md, D32).
//!
//! An AI player is a company a computer plays. Its mind runs here, in the
//! agent of the member whose game drives it: it asks that game about the
//! world and for its verdict on builds (`tpf3mp_bridge::ai`), and sends its
//! actions through the room as the seat's own intents, which every game
//! applies. No difficulty gets money, prices or knowledge a player lacks:
//! [`Profile`] is how well and how fast it plays.

pub mod brain;
pub mod chain;
pub mod costs;
pub mod money;
pub mod place;
pub mod profile;
pub mod rail;

pub use brain::{Brain, Memo, Out};
pub use profile::Profile;
