//! The AI seats this player's game drives (docs/AI_PLAYERS.md, "The
//! driver"): what the room handed this client, and what it refused.

use tpf3mp_proto::{AiSeat, IntentRejection, PlayerId};
use tracing::{info, warn};

/// The AI seats this client drives, as the room last said.
#[derive(Debug, Default)]
pub(crate) struct AiDriver {
    seats: Vec<AiSeat>,
}

impl AiDriver {
    /// The room hands this client these seats, all of them.
    pub(crate) fn drive(&mut self, seats: Vec<AiSeat>) {
        let names: Vec<&str> = seats.iter().map(|seat| seat.name.as_str()).collect();
        info!(seats = ?names, "this game drives these AI players");
        self.seats = seats;
    }

    /// The game's answer to question `id`.
    pub(crate) fn answer(
        &mut self,
        id: u32,
        answer: Result<tpf3mp_bridge::ai::AiAnswer, tpf3mp_proto::Text<256>>,
    ) {
        let _ = (id, answer);
    }

    /// The room refused an action sent for `seat`.
    pub(crate) fn rejected(&mut self, seat: PlayerId, client_seq: u64, reason: IntentRejection) {
        warn!(%seat, client_seq, ?reason, "the room refused an AI player's action");
    }
}
