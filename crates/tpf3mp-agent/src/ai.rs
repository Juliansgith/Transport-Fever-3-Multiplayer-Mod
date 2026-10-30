//! The AI seats this player's game drives (docs/AI_PLAYERS.md, "The
//! driver"): a mind for each ([`tpf3mp_ai::Brain`]), its questions to the
//! game (`ToHook::AiAsk`) and its actions and memo to the room.
//!
//! The minds think on a thread of their own. A rail line's planning takes
//! from a few milliseconds to a large part of a second (a route search
//! with no way through), and the agent's loop relays the game's messages
//! and the room's turns meanwhile: it only hands the minds the game's
//! answers and takes what they want, never waiting on them. Their actions
//! still reach the room one after another, in each seat's order.

use std::{
    collections::HashMap,
    sync::mpsc::{self, RecvTimeoutError, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use tpf3mp_ai::{Brain, Out};
use tpf3mp_bridge::ai::{AiAnswer, AiAsk};
use tpf3mp_proto::{AiSeat, IntentRejection, Payload, PlayerId, Text};
use tracing::{error, info, warn};

/// How often a seat's memo goes to the room while it changes.
const MEMO_EVERY: Duration = Duration::from_secs(30);
/// How long a seat handed to this game waits before its mind looks: the
/// room hands seats to a game that keeps up with it, which may have loaded
/// the room's world a moment ago and still be replaying turns.
const SETTLE_IN: Duration = Duration::from_secs(5);
/// How often the minds' thread looks at them while nothing comes in.
const LOOK_EVERY: Duration = Duration::from_millis(10);

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

/// A seat's mind as the driver uses it: a [`Brain`], or a test's stand-in.
pub(crate) trait Mind: Send {
    fn poll(&mut self, now: Instant, speed: u16) -> Option<Out>;
    fn answered(&mut self, answer: Result<AiAnswer, String>, now: Instant) -> Option<Out>;
    fn rejected(&mut self, reason: IntentRejection, now: Instant);
    fn take_said(&mut self) -> Vec<String>;
    fn memo(&self) -> Vec<u8>;
}

impl Mind for Brain {
    fn poll(&mut self, now: Instant, speed: u16) -> Option<Out> {
        Brain::poll(self, now, speed)
    }

    fn answered(&mut self, answer: Result<AiAnswer, String>, now: Instant) -> Option<Out> {
        Brain::answered(self, answer, now)
    }

    fn rejected(&mut self, reason: IntentRejection, now: Instant) {
        Brain::rejected(self, reason, now);
    }

    fn take_said(&mut self) -> Vec<String> {
        Brain::take_said(self)
    }

    fn memo(&self) -> Vec<u8> {
        Brain::memo(self)
    }
}

/// Makes the mind of a seat newly handed to this game.
pub(crate) type MakeMind = Box<dyn FnMut(&AiSeat) -> Box<dyn Mind> + Send>;

/// A brain from the seat's memo, holding still while the game settles in.
fn brain(seat: &AiSeat) -> Box<dyn Mind> {
    let memo = seat.memo.as_ref().map(Payload::as_bytes);
    let mut brain = Brain::new(seat.player, seat.name.as_str(), seat.difficulty, memo);
    brain.hold(Instant::now() + SETTLE_IN);
    Box::new(brain)
}

/// What the agent's loop hands the minds' thread.
enum ToMinds {
    Drive(Vec<AiSeat>),
    Answer {
        id: u32,
        answer: Result<AiAnswer, Text<256>>,
    },
    Rejected {
        seat: PlayerId,
        reason: IntentRejection,
    },
    /// The room's speed in percent, and whether this game plays its world.
    Plays {
        speed: u16,
        ready: bool,
    },
}

struct Seat {
    seat: AiSeat,
    mind: Box<dyn Mind>,
    /// Its actions sent, as the room counts them.
    seq: u64,
    /// Its memo as last sent, and when.
    memo: Vec<u8>,
    memo_at: Option<Instant>,
}

/// The minds and their questions, on the minds' thread.
struct Minds {
    seats: Vec<Seat>,
    /// Questions asked and not answered: by id, whose.
    asked: HashMap<u32, PlayerId>,
    next_id: u32,
    make: MakeMind,
    speed: u16,
    ready: bool,
}

impl Minds {
    /// The room hands this client these seats, all of them: a mind for each
    /// new one, from its memo; a seat no longer handed over is let go.
    fn drive(&mut self, seats: Vec<AiSeat>) {
        let mut kept = Vec::new();
        for seat in seats {
            match self.seats.iter().position(|s| s.seat.player == seat.player) {
                Some(index) => {
                    let mut known = self.seats.swap_remove(index);
                    known.seat = seat;
                    kept.push(known);
                }
                None => {
                    let mind = (self.make)(&seat);
                    kept.push(Seat {
                        memo: mind.memo(),
                        mind,
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

    /// The game's answer to question `id`, and what it makes its mind want.
    fn answer(&mut self, id: u32, answer: Result<AiAnswer, Text<256>>) -> Option<AiOut> {
        let player = self.asked.remove(&id)?;
        let seat = self.seats.iter_mut().find(|s| s.seat.player == player)?;
        let out = seat.mind.answered(
            answer.map_err(|why| why.as_str().to_owned()),
            Instant::now(),
        )?;
        Some(Self::hand_on(&mut self.asked, &mut self.next_id, seat, out))
    }

    fn rejected(&mut self, seat: PlayerId, reason: IntentRejection) {
        if let Some(s) = self.seats.iter_mut().find(|s| s.seat.player == seat) {
            s.mind.rejected(reason, Instant::now());
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

    /// What the minds want now: only while this game plays the room's
    /// world, at the room's speed.
    fn tick(&mut self, now: Instant) -> Vec<AiOut> {
        let mut out = Vec::new();
        if !self.ready {
            return out;
        }
        for seat in &mut self.seats {
            if let Some(want) = seat.mind.poll(now, self.speed) {
                out.push(Self::hand_on(
                    &mut self.asked,
                    &mut self.next_id,
                    seat,
                    want,
                ));
            }
            for line in seat.mind.take_said() {
                info!(ai = seat.seat.name.as_str(), "{line}");
            }
            let memo = seat.mind.memo();
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

    /// The minds' thread: what comes in, then a look at the minds, until
    /// the driver is gone.
    fn run(mut self, from: &mpsc::Receiver<ToMinds>, to: &mpsc::Sender<AiOut>) {
        loop {
            let mut wants = Vec::new();
            let mut next = from.recv_timeout(LOOK_EVERY);
            loop {
                match next {
                    Ok(ToMinds::Drive(seats)) => self.drive(seats),
                    Ok(ToMinds::Answer { id, answer }) => wants.extend(self.answer(id, answer)),
                    Ok(ToMinds::Rejected { seat, reason }) => self.rejected(seat, reason),
                    Ok(ToMinds::Plays { speed, ready }) => {
                        self.speed = speed;
                        self.ready = ready;
                    }
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                next = from.try_recv().map_err(|e| match e {
                    TryRecvError::Empty => RecvTimeoutError::Timeout,
                    TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
                });
            }
            wants.extend(self.tick(Instant::now()));
            for want in wants {
                if to.send(want).is_err() {
                    return;
                }
            }
        }
    }
}

/// The AI seats this client drives, as the room last said: the agent's
/// side of the minds' thread, started with the first seat.
pub(crate) struct AiDriver {
    /// To the minds' thread and from it, once it runs.
    to: Option<mpsc::Sender<ToMinds>>,
    from: Option<mpsc::Receiver<AiOut>>,
    /// Makes the minds, until the thread takes it.
    make: Option<MakeMind>,
    /// How many seats the room hands this client.
    seats: usize,
    /// The speed and whether this game plays, as last told the minds.
    plays: Option<(u16, bool)>,
    /// The minds' thread ended (a mind failed): no seat is driven further.
    stopped: bool,
}

impl Default for AiDriver {
    fn default() -> Self {
        Self::with_minds(Box::new(brain))
    }
}

impl std::fmt::Debug for AiDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiDriver")
            .field("seats", &self.seats)
            .field("thinking", &self.to.is_some())
            .field("stopped", &self.stopped)
            .finish()
    }
}

impl AiDriver {
    /// A driver whose seats' minds `make` makes.
    pub(crate) fn with_minds(make: MakeMind) -> Self {
        Self {
            to: None,
            from: None,
            make: Some(make),
            seats: 0,
            plays: None,
            stopped: false,
        }
    }

    /// The room hands this client these seats, all of them: a mind for each
    /// new one, from its memo; a seat no longer handed over is let go.
    pub(crate) fn drive(&mut self, seats: Vec<AiSeat>) {
        let names: Vec<&str> = seats.iter().map(|seat| seat.name.as_str()).collect();
        info!(seats = ?names, "this game drives these AI players");
        self.seats = seats.len();
        if self.to.is_none() && !seats.is_empty() {
            self.start();
        }
        self.send(ToMinds::Drive(seats));
    }

    fn start(&mut self) {
        let Some(make) = self.make.take() else {
            return;
        };
        let (to, from_agent) = mpsc::channel();
        let (to_agent, from) = mpsc::channel();
        let minds = Minds {
            seats: Vec::new(),
            asked: HashMap::new(),
            next_id: 0,
            make,
            speed: 100,
            ready: false,
        };
        let spawned = thread::Builder::new()
            .name("tpf3mp-ai".into())
            .spawn(move || minds.run(&from_agent, &to_agent));
        match spawned {
            Ok(_) => {
                self.to = Some(to);
                self.from = Some(from);
            }
            Err(why) => {
                error!(%why, "no thread for the AI players' minds: this game drives none");
                self.stopped = true;
            }
        }
    }

    fn send(&self, message: ToMinds) {
        if let Some(to) = &self.to {
            // A thread gone is seen in `tick`.
            let _ = to.send(message);
        }
    }

    /// Whether this client drives any AI seat.
    pub(crate) fn driving(&self) -> bool {
        self.seats > 0 && !self.stopped
    }

    /// The game's answer to question `id`, for the minds.
    pub(crate) fn answer(&mut self, id: u32, answer: Result<AiAnswer, Text<256>>) {
        self.send(ToMinds::Answer { id, answer });
    }

    /// The room refused an action sent for `seat`.
    pub(crate) fn rejected(&mut self, seat: PlayerId, client_seq: u64, reason: IntentRejection) {
        warn!(%seat, client_seq, ?reason, "the room refused an AI player's action");
        self.send(ToMinds::Rejected { seat, reason });
    }

    /// What the minds want, as far as they have thought: they look only
    /// while this game plays the room's world (`ready`), at the room's
    /// speed in percent. Never waits on them.
    pub(crate) fn tick(&mut self, speed: u16, ready: bool) -> Vec<AiOut> {
        if self.plays != Some((speed, ready)) {
            self.plays = Some((speed, ready));
            self.send(ToMinds::Plays { speed, ready });
        }
        let mut out = Vec::new();
        let Some(from) = &self.from else {
            return out;
        };
        loop {
            match from.try_recv() {
                Ok(want) => out.push(want),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self.stopped {
                        error!("the AI players' minds stopped: this game drives them no further");
                        self.stopped = true;
                    }
                    break;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use tpf3mp_bridge::ai::{AiCompany, AiQuestion};
    use tpf3mp_proto::{
        AiDifficulty, BoundedVec, FixedBytes, RoomId,
        action::{Action, CompanyOp},
    };

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

    /// A driver of brains that look at once.
    fn eager() -> AiDriver {
        AiDriver::with_minds(Box::new(|seat: &AiSeat| -> Box<dyn Mind> {
            Box::new(Brain::new(
                seat.player,
                seat.name.as_str(),
                seat.difficulty,
                None,
            ))
        }))
    }

    /// Ticks as the agent's loop does, every 2 ms, until the minds want
    /// something or `within` passes.
    fn wants(driver: &mut AiDriver, ready: bool, within: Duration) -> Vec<AiOut> {
        let end = Instant::now() + within;
        loop {
            let out = driver.tick(100, ready);
            if !out.is_empty() || Instant::now() >= end {
                return out;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_seat_settles_in_before_its_mind_looks() {
        let mut driver = AiDriver::default();
        driver.drive(vec![seat(0)]);
        assert!(driver.driving());
        assert!(
            wants(&mut driver, true, Duration::from_millis(300)).is_empty(),
            "nothing while it settles in"
        );
    }

    #[test]
    fn a_seat_asks_its_game_then_founds_its_company_through_the_room() {
        let mut driver = eager();
        driver.drive(vec![seat(0)]);
        assert!(
            wants(&mut driver, false, Duration::from_millis(200)).is_empty(),
            "nothing before the world plays"
        );
        let out = wants(&mut driver, true, Duration::from_secs(5));
        let [AiOut::Ask { id, ask }] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(ask.question, AiQuestion::Company);
        assert_eq!(ask.seat, seat(0).player);
        // Not founded yet: its first action founds its company.
        driver.answer(*id, Ok(company(None)));
        let out = wants(&mut driver, true, Duration::from_secs(5));
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
            Action::from_payload(payload).unwrap(),
            Action::CompanyOp(_)
        ));
    }

    #[test]
    fn an_answer_for_a_seat_let_go_is_dropped() {
        let mut driver = eager();
        driver.drive(vec![seat(0)]);
        let out = wants(&mut driver, true, Duration::from_secs(5));
        let [AiOut::Ask { id, .. }] = out.as_slice() else {
            panic!("{out:?}")
        };
        driver.drive(vec![seat(1)]);
        driver.answer(*id, Ok(company(None)));
        let end = Instant::now() + Duration::from_millis(300);
        while Instant::now() < end {
            let later = driver.tick(100, true);
            assert!(
                later.iter().all(|o| !matches!(o, AiOut::Intent { .. })),
                "{later:?}"
            );
            thread::sleep(Duration::from_millis(2));
        }
        assert!(driver.driving());
        driver.drive(Vec::new());
        assert!(!driver.driving());
    }

    /// A mind that asks once and thinks long over the answer, as a brain
    /// planning a rail line where there is no way through.
    struct Slow {
        asked: bool,
        think: Duration,
    }

    impl Mind for Slow {
        fn poll(&mut self, _now: Instant, _speed: u16) -> Option<Out> {
            if self.asked {
                return None;
            }
            self.asked = true;
            Some(Out::Ask(AiQuestion::Company))
        }

        fn answered(&mut self, _answer: Result<AiAnswer, String>, _now: Instant) -> Option<Out> {
            thread::sleep(self.think);
            let found = Action::CompanyOp(CompanyOp::Create {
                name: Text::new("Rossi Transport").unwrap(),
            });
            Some(Out::Act(found.to_payload().unwrap()))
        }

        fn rejected(&mut self, _reason: IntentRejection, _now: Instant) {}

        fn take_said(&mut self) -> Vec<String> {
            Vec::new()
        }

        fn memo(&self) -> Vec<u8> {
            Vec::new()
        }
    }

    #[test]
    fn the_agents_loop_keeps_moving_while_a_mind_thinks() {
        const THINK: Duration = Duration::from_millis(600);
        let mut driver = AiDriver::with_minds(Box::new(|_: &AiSeat| -> Box<dyn Mind> {
            Box::new(Slow {
                asked: false,
                think: THINK,
            })
        }));
        driver.drive(vec![seat(0)]);
        let out = wants(&mut driver, true, Duration::from_secs(5));
        let [AiOut::Ask { id, .. }] = out.as_slice() else {
            panic!("{out:?}")
        };
        // Handing the mind its answer returns at once, and so does every
        // tick while it thinks: the loop relays the game and the room on.
        let asked = Instant::now();
        driver.answer(*id, Ok(company(None)));
        assert!(
            asked.elapsed() < Duration::from_millis(50),
            "the answer waited"
        );
        let (mut ticks, mut slowest, mut intent) = (0_u32, Duration::ZERO, None);
        while intent.is_none() && asked.elapsed() < Duration::from_secs(5) {
            let before = Instant::now();
            let out = driver.tick(100, true);
            slowest = slowest.max(before.elapsed());
            ticks += 1;
            intent = out.into_iter().find(|o| matches!(o, AiOut::Intent { .. }));
            thread::sleep(Duration::from_millis(2));
        }
        assert!(intent.is_some(), "its action came");
        assert!(asked.elapsed() >= THINK, "after it thought");
        assert!(
            slowest < Duration::from_millis(50),
            "a tick waited: {slowest:?}"
        );
        assert!(ticks >= 100, "the loop ran on meanwhile: {ticks} ticks");
    }
}
