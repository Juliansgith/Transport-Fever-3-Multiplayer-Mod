//! The AI seats this player's game drives (docs/AI_PLAYERS.md, "The
//! driver"): a mind for each ([`tpf3mp_ai::Brain`]), its questions to the
//! game (`ToHook::AiAsk`) and its actions and memo to the room.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use tpf3mp_ai::{Brain, Out};
use tpf3mp_bridge::ai::{AiAnswer, AiAsk};
use tpf3mp_proto::{AiSeat, IntentRejection, Payload, PlayerId, Text};
use tracing::{info, warn};

/// How often a seat's memo goes to the room while it changes.
const MEMO_EVERY: Duration = Duration::from_secs(30);
/// How long a seat handed to this game waits before its mind looks: the
/// room hands seats to a game that keeps up with it, which may have loaded
/// the room's world a moment ago and still be replaying turns.
const SETTLE_IN: Duration = Duration::from_secs(5);

/// What the driver hands on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AiOut {
    /// To the game: a question for a seat.
    Ask { id: u32, ask: AiAsk },
    /// To the room: an action for a seat.
    Intent {
        seat: PlayerId,
        seq: u64,
        payload: Payload,
    },
    /// To the room: what a seat keeps for its next driver.
    Memo { seat: PlayerId, memo: Payload },
}

struct Seat {
    seat: AiSeat,
    brain: Brain,
    /// Its actions sent, as the room counts them.
    seq: u64,
    /// Its memo as last sent, and when.
    memo: Vec<u8>,
    memo_at: Option<Instant>,
}

/// The AI seats this client drives, as the room last said, and their minds.
#[derive(Default)]
pub(crate) struct AiDriver {
    seats: Vec<Seat>,
    /// Questions asked and not answered: by id, whose.
    asked: HashMap<u32, PlayerId>,
    next_id: u32,
    /// What answers and refusals made the brains want, for the next tick.
    queued: Vec<AiOut>,
}

impl std::fmt::Debug for AiDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiDriver")
            .field("seats", &self.seats.len())
            .field("asked", &self.asked.len())
            .finish()
    }
}

impl AiDriver {
    /// The room hands this client these seats, all of them: a mind for each
    /// new one, from its memo; a seat no longer handed over is let go.
    pub(crate) fn drive(&mut self, seats: Vec<AiSeat>) {
        let names: Vec<&str> = seats.iter().map(|seat| seat.name.as_str()).collect();
        info!(seats = ?names, "this game drives these AI players");
        let mut kept = Vec::new();
        for seat in seats {
            match self.seats.iter().position(|s| s.seat.player == seat.player) {
                Some(index) => {
                    let mut known = self.seats.swap_remove(index);
                    known.seat = seat;
                    kept.push(known);
                }
                None => {
                    let memo = seat.memo.as_ref().map(Payload::as_bytes);
                    let mut brain =
                        Brain::new(seat.player, seat.name.as_str(), seat.difficulty, memo);
                    brain.hold(Instant::now() + SETTLE_IN);
                    kept.push(Seat {
                        memo: brain.memo(),
                        brain,
                        seat,
                        seq: 0,
                        memo_at: None,
                    });
                }
            }
        }
        self.seats = kept;
        let driven: Vec<PlayerId> = self.seats.iter().map(|s| s.seat.player).collect();
        self.asked.retain(|_, seat| driven.contains(seat));
    }

    /// Whether this client drives any AI seat.
    pub(crate) fn driving(&self) -> bool {
        !self.seats.is_empty()
    }

    /// The game's answer to question `id`.
    pub(crate) fn answer(&mut self, id: u32, answer: Result<AiAnswer, Text<256>>) {
        let Some(player) = self.asked.remove(&id) else {
            return;
        };
        let now = Instant::now();
        let Some(seat) = self.seats.iter_mut().find(|s| s.seat.player == player) else {
            return;
        };
        let out = seat
            .brain
            .answered(answer.map_err(|why| why.as_str().to_owned()), now);
        if let Some(out) = out {
            let made = Self::hand_on(&mut self.asked, &mut self.next_id, seat, out);
            self.queued.push(made);
        }
    }

    /// The room refused an action sent for `seat`.
    pub(crate) fn rejected(&mut self, seat: PlayerId, client_seq: u64, reason: IntentRejection) {
        warn!(%seat, client_seq, ?reason, "the room refused an AI player's action");
        if let Some(s) = self.seats.iter_mut().find(|s| s.seat.player == seat) {
            s.brain.rejected(reason, Instant::now());
        }
    }

    fn hand_on(
        asked: &mut HashMap<u32, PlayerId>,
        next_id: &mut u32,
        seat: &mut Seat,
        out: Out,
    ) -> AiOut {
        match out {
            Out::Ask(question) => {
                let id = *next_id;
                *next_id = next_id.wrapping_add(1);
                asked.insert(id, seat.seat.player);
                AiOut::Ask {
                    id,
                    ask: AiAsk {
                        seat: seat.seat.player,
                        question,
                    },
                }
            }
            Out::Act(payload) => {
                let seq = seat.seq;
                seat.seq += 1;
                AiOut::Intent {
                    seat: seat.seat.player,
                    seq,
                    payload,
                }
            }
        }
    }

    /// What the minds want now: only while this game plays the room's world
    /// (`ready`), at the room's speed in percent.
    pub(crate) fn tick(&mut self, now: Instant, speed: u16, ready: bool) -> Vec<AiOut> {
        let mut out = std::mem::take(&mut self.queued);
        if !ready {
            return out;
        }
        for seat in &mut self.seats {
            if let Some(want) = seat.brain.poll(now, speed) {
                out.push(Self::hand_on(
                    &mut self.asked,
                    &mut self.next_id,
                    seat,
                    want,
                ));
            }
            for line in seat.brain.take_said() {
                info!(ai = seat.seat.name.as_str(), "{line}");
            }
            let memo = seat.brain.memo();
            let due = seat
                .memo_at
                .is_none_or(|at| now.saturating_duration_since(at) >= MEMO_EVERY);
            if memo != seat.memo
                && due
                && let Ok(payload) = Payload::new(memo.clone())
            {
                seat.memo = memo;
                seat.memo_at = Some(now);
                out.push(AiOut::Memo {
                    seat: seat.seat.player,
                    memo: payload,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use tpf3mp_bridge::ai::{AiCompany, AiQuestion};
    use tpf3mp_proto::{AiDifficulty, BoundedVec, FixedBytes, RoomId};

    use super::*;

    fn seat(n: u32) -> AiSeat {
        AiSeat {
            player: PlayerId::ai_seat(&RoomId(FixedBytes([1; 16])), n),
            name: Text::new("Rossi Transport").unwrap(),
            difficulty: AiDifficulty::Hard,
            memo: None,
        }
    }

    fn company(founded: Option<u32>) -> AiAnswer {
        AiAnswer::Company(AiCompany {
            company: founded,
            money: 1_000_000,
            year: 1925,
            stations: BoundedVec::default(),
            depots: BoundedVec::default(),
            lines: BoundedVec::default(),
            vehicles: BoundedVec::default(),
            offers: BoundedVec::default(),
            loans: 0,
        })
    }

    #[test]
    fn a_seat_asks_its_game_then_founds_its_company_through_the_room() {
        let mut driver = AiDriver::default();
        driver.drive(vec![seat(0)]);
        assert!(
            driver.tick(Instant::now(), 100, true).is_empty(),
            "nothing while it settles in"
        );
        let now = Instant::now() + SETTLE_IN;
        assert!(
            driver.tick(now, 100, false).is_empty(),
            "nothing before the world plays"
        );
        let out = driver.tick(now, 100, true);
        let [AiOut::Ask { id, ask }] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(ask.question, AiQuestion::Company);
        assert_eq!(ask.seat, seat(0).player);
        // Not founded yet: its first action founds its company.
        driver.answer(*id, Ok(company(None)));
        let out = driver.tick(now, 100, true);
        // Its founding first (the test's clock runs ahead of the real one,
        // so the next look may follow at once).
        let [
            AiOut::Intent {
                seat: who,
                seq,
                payload,
            },
            ..,
        ] = out.as_slice()
        else {
            panic!("{out:?}")
        };
        assert_eq!((*who, *seq), (seat(0).player, 0));
        assert!(matches!(
            tpf3mp_proto::action::Action::from_payload(payload).unwrap(),
            tpf3mp_proto::action::Action::CompanyOp(_)
        ));
    }

    #[test]
    fn an_answer_for_a_seat_let_go_is_dropped() {
        let mut driver = AiDriver::default();
        driver.drive(vec![seat(0)]);
        let out = driver.tick(Instant::now() + SETTLE_IN, 100, true);
        let [AiOut::Ask { id, .. }] = out.as_slice() else {
            panic!("{out:?}")
        };
        driver.drive(vec![seat(1)]);
        driver.answer(*id, Ok(company(None)));
        let later = driver.tick(Instant::now() + SETTLE_IN, 100, true);
        assert!(
            later.iter().all(|o| !matches!(o, AiOut::Intent { .. })),
            "{later:?}"
        );
        assert!(driver.driving());
        driver.drive(Vec::new());
        assert!(!driver.driving());
    }
}
