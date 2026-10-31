//! The AI players' questions this game answers for its agent, and the
//! answers on their way back (docs/AI_PLAYERS.md, "What the driver's game
//! does"). The room hands an AI seat to one member's game; its agent's AI
//! asks that game about the world through the link ([`tpf3mp_bridge::ai`]),
//! and the mod's GUI answers a few a frame (`tpf3mp/ai.lua`, through
//! `aiAsk` and `aiAnswer`).

use std::{
    collections::VecDeque,
    sync::{Mutex, PoisonError},
};

use tpf3mp_bridge::ai::{AiAnswer, AiAsk};
use tpf3mp_proto::Text;

/// Questions kept unanswered at most: the agent asks a few at a time and
/// asks again after a timeout, so more only means a stale one.
pub const MAX_ASKS: usize = 32;
/// Answers kept until the link takes them.
pub const MAX_ANSWERS: usize = 64;

#[derive(Default)]
struct Queues {
    asks: VecDeque<(u32, AiAsk)>,
    answers: VecDeque<(u32, Result<AiAnswer, Text<256>>)>,
}

static QUEUES: Mutex<Queues> = Mutex::new(Queues {
    asks: VecDeque::new(),
    answers: VecDeque::new(),
});

fn queues() -> std::sync::MutexGuard<'static, Queues> {
    QUEUES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The agent asked question `id`. The oldest is dropped when too many wait:
/// the agent will have given up on it.
pub fn asked(id: u32, ask: AiAsk) {
    let mut queues = queues();
    if queues.asks.len() >= MAX_ASKS {
        queues.asks.pop_front();
    }
    queues.asks.push_back((id, ask));
}

/// The next question to answer, oldest first.
pub fn next() -> Option<(u32, AiAsk)> {
    queues().asks.pop_front()
}

/// The mod's answer to question `id`.
pub fn answer(id: u32, answer: Result<AiAnswer, Text<256>>) {
    let mut queues = queues();
    if queues.answers.len() >= MAX_ANSWERS {
        queues.answers.pop_front();
    }
    queues.answers.push_back((id, answer));
}

/// The answers for the link, oldest first.
pub fn take_answers() -> Vec<(u32, Result<AiAnswer, Text<256>>)> {
    queues().answers.drain(..).collect()
}

/// Forgets every question and answer: the room's game ended.
pub fn clear() {
    let mut queues = queues();
    queues.asks.clear();
    queues.answers.clear();
}

#[cfg(test)]
mod tests {
    use tpf3mp_bridge::ai::{AiQuestion, AiVerdict};
    use tpf3mp_proto::{BoundedVec, FixedBytes, PlayerId};

    use super::*;

    #[test]
    fn questions_wait_in_order_and_the_oldest_goes_first_when_too_many_wait() {
        clear();
        let ask = |n: u8| AiAsk {
            seat: PlayerId(FixedBytes([n; 32])),
            question: AiQuestion::Towns,
        };
        for id in 0..(MAX_ASKS as u32 + 2) {
            asked(id, ask(1));
        }
        assert_eq!(next().map(|(id, _)| id), Some(2), "the two oldest dropped");
        answer(
            9,
            Ok(AiAnswer::Verdict(AiVerdict {
                ok: true,
                cost: 1,
                messages: BoundedVec::default(),
                collisions: 0,
            })),
        );
        answer(10, Err(Text::new("no").unwrap()));
        let ids: Vec<u32> = take_answers().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, [9, 10]);
        assert!(take_answers().is_empty());
        clear();
        assert!(next().is_none());
    }
}
