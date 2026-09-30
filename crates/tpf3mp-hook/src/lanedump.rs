//! Lane dumps: the full text of chosen lanes, entry by entry, in `hook.log`
//! at chosen checkpoints (docs/HOOKS.md, "Lane dumps"), for two or three
//! games' logs to be diffed (`tools/lane_diff.py`) where the lanes' digests
//! only say *that* a part of the world differs.
//!
//! The room tells only the games whose world differed from its verdict
//! (`Notice::Diverged`), and the others are needed to diff against. So a
//! game told it diverged says so in the room's chat, in a line
//! [`announce`] writes and [`parse`] reads, naming the lanes and the two
//! checkpoint steps to dump them at; every game that hears it, that one
//! included (the room passes chat to its sender too), dumps those lanes at
//! those steps. The steps lie far enough ahead ([`MARGIN_STEPS`]) for the
//! line to reach every game first.
//!
//! Bounded: one dump asked for a minute at most ([`GAP`]), two checkpoints
//! each, a few lanes, steps no further ahead than [`MAX_AHEAD`] checkpoints,
//! and the lines a checkpoint writes are capped by the hook's Lua side
//! (`crate::lua`). A line naming the same steps as one taken already only
//! adds its lanes: both diverged games of a room of three say one.
//!
//! [`ENV`] in the game's environment adds lanes to every checkpoint, for
//! chasing a desync on purpose, or turns dumps off.
//!
//! Pure: the driver (`crate::step`) hands it the steps, the room's lines
//! and the time.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

/// The game's environment: `all`, or lanes by number (`3`, `0,3`), dumped
/// at every checkpoint besides any a divergence asks for; `off` dumps
/// nothing, not even after a divergence. Unset: only after a divergence.
pub const ENV: &str = "TPF3MP_HOOK_LANE_DUMP";

/// The lanes the mod reads (tpf3mp/lanes.lua): `all` means these.
pub const LANES: u16 = 7;
/// Most lanes one dump names.
pub const MAX_LANES: usize = 16;
/// Checkpoints dumped after a divergence.
pub const CHECKPOINTS: usize = 2;
/// Least time between two dumps asked for.
pub const GAP: Duration = Duration::from_secs(60);
/// Steps between asking and the first step dumped, at least: time for the
/// room's chat to reach every game.
pub const MARGIN_STEPS: u64 = 20;
/// Furthest ahead a dump heard may lie, in checkpoints.
pub const MAX_AHEAD: u64 = 20;

/// What the chat line begins with.
pub const PREFIX: &str = "[tpf3mp] lane dump ";

/// A dump the room's chat carries: the lanes, the steps to dump them at, and
/// the checkpoint that differed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub lanes: Vec<u16>,
    pub steps: Vec<u64>,
    pub diverged: u64,
}

/// The chat line for `ask`.
pub fn announce(ask: &Ask) -> String {
    let join = |items: &mut dyn Iterator<Item = String>| items.collect::<Vec<_>>().join(",");
    format!(
        "{PREFIX}lanes={} steps={} diverged={}: this game's world differed from the room's; every game writes these lanes to its hook.log",
        join(&mut ask.lanes.iter().map(u16::to_string)),
        join(&mut ask.steps.iter().map(u64::to_string)),
        ask.diverged,
    )
}

/// The dump a chat line asks for, if it is one; the numbers only, checked
/// by [`LaneDumps::heard`].
pub fn parse(text: &str) -> Option<Ask> {
    let rest = text.strip_prefix(PREFIX)?;
    let fields = rest.split(':').next()?;
    let (mut lanes, mut steps, mut diverged) = (None, None, None);
    for field in fields.split_whitespace() {
        let (key, value) = field.split_once('=')?;
        match key {
            "lanes" => lanes = Some(numbers::<u16>(value)?),
            "steps" => steps = Some(numbers::<u64>(value)?),
            "diverged" => diverged = Some(value.parse().ok()?),
            _ => {}
        }
    }
    Some(Ask {
        lanes: lanes?,
        steps: steps?,
        diverged: diverged?,
    })
}

fn numbers<T: std::str::FromStr>(list: &str) -> Option<Vec<T>> {
    list.split(',').map(|n| n.parse().ok()).collect()
}

/// Which lanes the environment dumps at every checkpoint, and whether dumps
/// are off.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Setting {
    pub always: BTreeSet<u16>,
    pub off: bool,
}

impl Setting {
    /// From [`ENV`]'s value; a value it cannot read is refused with why, and
    /// the default (after a divergence only) kept.
    pub fn from_env(value: Option<&str>) -> (Self, Option<String>) {
        let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
            return (Self::default(), None);
        };
        match value.to_ascii_lowercase().as_str() {
            "off" | "none" | "false" => {
                return (
                    Self {
                        always: BTreeSet::new(),
                        off: true,
                    },
                    None,
                );
            }
            "all" => {
                return (
                    Self {
                        always: (0..LANES).collect(),
                        off: false,
                    },
                    None,
                );
            }
            _ => {}
        }
        match numbers::<u16>(&value.replace(' ', "")) {
            Some(lanes) if !lanes.is_empty() && lanes.len() <= MAX_LANES => (
                Self {
                    always: lanes.into_iter().collect(),
                    off: false,
                },
                None,
            ),
            _ => (
                Self::default(),
                Some(format!(
                    "{ENV}={value} is not `all`, `off` or lane numbers such as `3` or `0,3`; lanes are dumped after a divergence only"
                )),
            ),
        }
    }
}

/// One checkpoint's dump: its step, the lanes, and why, for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpOrder {
    pub step: u64,
    pub lanes: Vec<u16>,
    pub why: String,
}

/// The dumps to come.
#[derive(Debug, Default)]
pub struct LaneDumps {
    setting: Setting,
    /// Lanes to dump, by checkpoint step, and why.
    planned: BTreeMap<u64, (BTreeSet<u16>, String)>,
    /// When the last dump was asked for (here or heard).
    last: Option<Instant>,
}

impl LaneDumps {
    pub fn new(setting: Setting) -> Self {
        Self {
            setting,
            ..Self::default()
        }
    }

    pub fn setting(&self) -> &Setting {
        &self.setting
    }

    /// The room said this game's world differed at `diverged` in `lanes`;
    /// the game runs `next_step` next. Returns the dump to announce in the
    /// room's chat, planned here already; or `None`: dumps are off, or one
    /// was asked for within [`GAP`].
    pub fn diverged(
        &mut self,
        diverged: u64,
        lanes: &[u16],
        next_step: u64,
        interval: u64,
        now: Instant,
    ) -> Option<Ask> {
        if self.setting.off || interval == 0 || !self.rested(now) {
            return None;
        }
        let mut lanes: Vec<u16> = lanes
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        lanes.truncate(MAX_LANES);
        if lanes.is_empty() {
            return None;
        }
        let from = next_step.saturating_add(MARGIN_STEPS);
        let first = from.div_ceil(interval).saturating_mul(interval);
        let steps = (0..CHECKPOINTS as u64)
            .map(|k| first.saturating_add(k.saturating_mul(interval)))
            .collect();
        let ask = Ask {
            lanes,
            steps,
            diverged,
        };
        self.plan(&ask, now);
        Some(ask)
    }

    /// A chat line of the room's; the game runs `next_step` next. Returns
    /// the dump it planned, or why a dump line was not taken; `None` for a
    /// line that is no dump.
    pub fn heard(
        &mut self,
        text: &str,
        next_step: u64,
        interval: u64,
        now: Instant,
    ) -> Option<Result<Ask, String>> {
        let ask = parse(text)?;
        Some(self.take_heard(ask, next_step, interval, now))
    }

    fn take_heard(
        &mut self,
        ask: Ask,
        next_step: u64,
        interval: u64,
        now: Instant,
    ) -> Result<Ask, String> {
        if self.setting.off {
            return Err(format!("lane dumps are off ({ENV})"));
        }
        if interval == 0
            || ask.lanes.is_empty()
            || ask.lanes.len() > MAX_LANES
            || ask.steps.is_empty()
            || ask.steps.len() > CHECKPOINTS
        {
            return Err("it names no lanes or steps, or too many".into());
        }
        if ask
            .steps
            .iter()
            .any(|s| *s == 0 || !s.is_multiple_of(interval))
        {
            return Err(format!(
                "a step it names is not a checkpoint (every {interval})"
            ));
        }
        let furthest = next_step.saturating_add(MAX_AHEAD.saturating_mul(interval));
        if ask.steps.iter().any(|s| *s > furthest) {
            return Err(format!(
                "a step it names lies more than {MAX_AHEAD} checkpoints ahead"
            ));
        }
        if ask.steps.iter().all(|s| *s < next_step) {
            return Err(format!(
                "this game is past its steps (it runs step {next_step} next)"
            ));
        }
        // The same steps as a dump taken: its lanes join; else one a minute.
        let joins = ask.steps.iter().all(|s| self.planned.contains_key(s));
        if !joins && !self.rested(now) {
            return Err("a lane dump was asked for less than a minute ago".into());
        }
        self.plan(&ask, now);
        Ok(ask)
    }

    fn rested(&self, now: Instant) -> bool {
        self.last
            .is_none_or(|last| now.saturating_duration_since(last) >= GAP)
    }

    fn plan(&mut self, ask: &Ask, now: Instant) {
        let joins = ask.steps.iter().all(|s| self.planned.contains_key(s));
        if !joins {
            self.last = Some(now);
        }
        for step in &ask.steps {
            let (lanes, _) = self
                .planned
                .entry(*step)
                .or_insert_with(|| (BTreeSet::new(), format!("step {} diverged", ask.diverged)));
            lanes.extend(ask.lanes.iter().copied());
        }
    }

    /// The dump for the checkpoint at `step`, which the game is about to
    /// read: the environment's lanes and those planned for it. Forgets the
    /// plans for steps up to it.
    pub fn take(&mut self, step: u64) -> Option<DumpOrder> {
        let mut lanes = self.setting.always.clone();
        let mut why = Vec::new();
        if !lanes.is_empty() {
            why.push(ENV.to_owned());
        }
        let later = self.planned.split_off(&step.saturating_add(1));
        let due = std::mem::replace(&mut self.planned, later);
        if let Some((planned, reason)) = due.get(&step) {
            lanes.extend(planned.iter().copied());
            why.push(reason.clone());
        }
        if lanes.is_empty() {
            return None;
        }
        Some(DumpOrder {
            step,
            lanes: lanes.into_iter().collect(),
            why: why.join(", "),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY: u64 = 50;

    #[test]
    fn the_chat_line_reads_back_as_it_was_written() {
        let ask = Ask {
            lanes: vec![1, 3],
            steps: vec![300, 350],
            diverged: 250,
        };
        let line = announce(&ask);
        assert!(line.starts_with("[tpf3mp] lane dump lanes=1,3 steps=300,350 diverged=250: "));
        assert!(
            tpf3mp_proto::ChatText::new(&line).is_ok(),
            "fits a chat line"
        );
        assert_eq!(parse(&line), Some(ask));
        assert_eq!(parse("lanes=3 steps=300 diverged=250"), None, "no prefix");
        assert_eq!(
            parse("[tpf3mp] lane dump lanes=x steps=300 diverged=250"),
            None
        );
        assert_eq!(parse("[tpf3mp] lane dump steps=300 diverged=250: hi"), None);
    }

    #[test]
    fn the_environment_names_lanes_all_or_none() {
        assert_eq!(Setting::from_env(None), (Setting::default(), None));
        assert_eq!(Setting::from_env(Some(" ")), (Setting::default(), None));
        let (all, _) = Setting::from_env(Some("all"));
        assert_eq!(all.always.len(), usize::from(LANES));
        let (some, _) = Setting::from_env(Some("3, 0"));
        assert_eq!(some.always.into_iter().collect::<Vec<_>>(), [0, 3]);
        assert!(Setting::from_env(Some("OFF")).0.off);
        let (kept, why) = Setting::from_env(Some("vehicles"));
        assert_eq!(kept, Setting::default());
        assert!(why.unwrap().contains("after a divergence only"));
    }

    #[test]
    fn a_divergence_plans_the_next_two_checkpoints_past_the_margin() {
        let now = Instant::now();
        let mut dumps = LaneDumps::default();
        // Told at step 262 that step 250 differed in lane 3.
        let ask = dumps.diverged(250, &[3, 3], 262, EVERY, now).unwrap();
        assert_eq!(ask.steps, [300, 350]);
        assert_eq!(ask.lanes, [3]);
        // Told at step 285: 300 is too close for the room's chat.
        let mut late = LaneDumps::default();
        assert_eq!(
            late.diverged(250, &[3], 285, EVERY, now).unwrap().steps,
            [350, 400]
        );

        assert_eq!(dumps.take(250), None, "nothing planned there");
        let first = dumps.take(300).unwrap();
        assert_eq!((first.step, first.lanes.as_slice()), (300, &[3][..]));
        assert!(first.why.contains("step 250 diverged"), "{}", first.why);
        assert_eq!(dumps.take(350).unwrap().lanes, [3]);
        assert_eq!(dumps.take(400), None, "two checkpoints only");
    }

    #[test]
    fn one_dump_a_minute_and_the_same_steps_join() {
        let t0 = Instant::now();
        let mut dumps = LaneDumps::default();
        let ask = dumps.diverged(250, &[3], 262, EVERY, t0).unwrap();
        // Diverged again at 300 and 350: nothing more within the minute.
        assert_eq!(
            dumps.diverged(300, &[3], 312, EVERY, t0 + Duration::from_secs(10)),
            None
        );
        // Its own line comes back, and the other diverged game's, with
        // another lane: they join.
        let echo = announce(&ask);
        assert_eq!(dumps.heard(&echo, 263, EVERY, t0).unwrap(), Ok(ask.clone()));
        let other = announce(&Ask {
            lanes: vec![1],
            ..ask.clone()
        });
        assert!(dumps.heard(&other, 263, EVERY, t0).unwrap().is_ok());
        assert_eq!(dumps.take(300).unwrap().lanes, [1, 3]);
        // A new dump at other steps waits for the minute.
        let later = announce(&Ask {
            lanes: vec![3],
            steps: vec![450, 500],
            diverged: 400,
        });
        let refused = dumps.heard(&later, 410, EVERY, t0 + Duration::from_secs(30));
        assert!(matches!(refused, Some(Err(why)) if why.contains("minute")));
        assert!(dumps.heard(&later, 410, EVERY, t0 + GAP).unwrap().is_ok());
        assert_eq!(
            dumps.heard("hello", 410, EVERY, t0 + GAP),
            None,
            "not a dump"
        );
    }

    #[test]
    fn a_dump_heard_is_checked_before_it_is_taken() {
        let now = Instant::now();
        let heard = |ask: Ask, next: u64| {
            LaneDumps::default()
                .heard(&announce(&ask), next, EVERY, now)
                .unwrap()
        };
        let ask = Ask {
            lanes: vec![3],
            steps: vec![300, 350],
            diverged: 250,
        };
        assert!(heard(ask.clone(), 262).is_ok());
        assert!(
            heard(
                Ask {
                    steps: vec![310],
                    ..ask.clone()
                },
                262
            )
            .is_err(),
            "not a checkpoint"
        );
        assert!(
            heard(
                Ask {
                    steps: vec![300, 350, 400],
                    ..ask.clone()
                },
                262
            )
            .is_err()
        );
        assert!(
            heard(
                Ask {
                    steps: vec![5000],
                    ..ask.clone()
                },
                262
            )
            .is_err(),
            "too far"
        );
        assert!(heard(ask.clone(), 400).is_err(), "past both");
        assert!(
            heard(
                Ask {
                    lanes: (0..17).collect(),
                    ..ask.clone()
                },
                262
            )
            .is_err()
        );
        let mut off = LaneDumps::new(Setting::from_env(Some("off")).0);
        assert!(
            off.heard(&announce(&ask), 262, EVERY, now)
                .unwrap()
                .is_err()
        );
        assert_eq!(off.diverged(250, &[3], 262, EVERY, now), None);
    }

    #[test]
    fn the_environments_lanes_are_dumped_at_every_checkpoint_with_those_planned() {
        let now = Instant::now();
        let mut dumps = LaneDumps::new(Setting::from_env(Some("3")).0);
        let every = dumps.take(50).unwrap();
        assert_eq!(
            (every.lanes.as_slice(), every.why.as_str()),
            (&[3][..], ENV)
        );
        dumps.diverged(50, &[0], 62, EVERY, now).unwrap();
        let both = dumps.take(100).unwrap();
        assert_eq!(both.lanes, [0, 3]);
        assert!(both.why.contains(ENV) && both.why.contains("step 50 diverged"));
    }
}
