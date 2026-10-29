//! What the agent asks the game that drives AI players, and what it answers
//! (docs/AI_PLAYERS.md, "What the driver's game does"). The mod reads the
//! world in the HUD's Lua state and judges builds there, a few a frame at
//! most; the agent's AI decides (`tpf3mp-ai`).
//!
//! The answers travel as Lua tables the hook converts
//! (`tpf3mp_proto::lua::from_lua`): positions in metres become [`Pos`]
//! millimetres, as an action's do; every other number is whole.

use serde::{Deserialize, Serialize};
use tpf3mp_proto::{
    BoundedVec, Payload, PlayerId, Text,
    action::{LineId, Pos, ResName, StationId, TownId, VehicleId},
};

/// Most towns a [`AiAnswer::Towns`] lists: the largest.
pub const MAX_AI_TOWNS: usize = 160;
/// Most street edges a [`AiAnswer::Streets`] lists: the nearest.
pub const MAX_AI_STREETS: usize = 320;
/// Most vehicle models a [`AiAnswer::Models`] lists.
pub const MAX_AI_MODELS: usize = 48;
/// Most stations, depots, lines or vehicles an [`AiCompany`] lists.
pub const MAX_AI_OWNED: usize = 160;
/// Most messages an [`AiVerdict`] carries.
pub const MAX_AI_MESSAGES: usize = 8;

/// One question about the world, for an AI seat the asking game drives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiAsk {
    /// The seat the question is for: its company is the one a verdict
    /// builds as, and the one [`AiQuestion::Company`] describes.
    pub seat: PlayerId,
    pub question: AiQuestion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiQuestion {
    /// The seat's company and what it owns ([`AiCompany`]).
    Company,
    /// The world's towns, largest first.
    Towns,
    /// The street edges within `radius` metres of `at`, nearest first.
    Streets { at: Pos, radius: u32 },
    /// The vehicle models the company may buy now for a transport mode,
    /// by the game's name (`BUS`, `TRUCK`, `TRAIN`, ...).
    Models { mode: Text<16> },
    /// Whether this action (an intent's payload) would be built as the
    /// seat's company would build it, and what it would cost: the game's
    /// own verdict, nothing built ([`AiVerdict`]).
    Verdict(Payload),
}

/// What the game answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiAnswer {
    Company(AiCompany),
    Towns(BoundedVec<AiTown, MAX_AI_TOWNS>),
    Streets(BoundedVec<AiStreet, MAX_AI_STREETS>),
    Models(BoundedVec<AiModel, MAX_AI_MODELS>),
    Verdict(AiVerdict),
}

/// An AI seat's company as its game sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiCompany {
    /// The company the seat plays for, by the room's id for it; none
    /// while it plays for the room's first company, before it founds its
    /// own.
    pub company: Option<u32>,
    /// Its money, in whole units of the game's currency.
    pub money: i64,
    /// The game's year.
    pub year: i32,
    /// Its station groups.
    pub stations: BoundedVec<AiStation, MAX_AI_OWNED>,
    /// Its depots.
    pub depots: BoundedVec<AiDepot, MAX_AI_OWNED>,
    pub lines: BoundedVec<AiLine, MAX_AI_OWNED>,
    pub vehicles: BoundedVec<AiVehicle, MAX_AI_OWNED>,
}

/// A station group of the company's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiStation {
    pub id: StationId,
    pub pos: Pos,
    /// Its terminals, all stations of the group together.
    pub terminals: u16,
    /// Passengers and cargo waiting there now.
    pub waiting: u32,
}

/// A depot of the company's: a construction, named as a purchase names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiDepot {
    pub file: ResName,
    /// Its construction's origin.
    pub at: Pos,
    /// The street node its entrance ends in, where it has one.
    pub entrance: Option<Pos>,
    /// Whether that entrance reaches the street network: an edge other
    /// than its own meets it.
    pub connected: bool,
}

/// A line of the company's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiLine {
    pub id: LineId,
    pub name: Text<64>,
    pub stops: BoundedVec<StationId, 32>,
    pub vehicles: BoundedVec<VehicleId, 64>,
    /// How full its vehicles run, in percent of their capacity, on average
    /// over its stops; 0 unknown.
    pub load: u8,
    /// Passengers and cargo waiting at its stops for it.
    pub waiting: u32,
    /// Its income and costs over the last twelve months, in whole units.
    pub income: i64,
    pub costs: i64,
    /// Problems the game sees on it (no path, no vehicle, ...).
    pub problems: u16,
}

/// A vehicle of the company's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiVehicle {
    pub id: VehicleId,
    pub line: Option<LineId>,
    /// Its first part's model.
    pub model: ResName,
    /// Its age, in days of the game's calendar.
    pub age: u32,
    /// On no line: in a depot, or waiting for one.
    pub idle: bool,
}

/// A town.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiTown {
    pub id: TownId,
    pub name: Text<48>,
    pub pos: Pos,
    /// Its town buildings: how large it is.
    pub buildings: u32,
    /// Its districts' centres: residential, commercial, industrial.
    pub centres: BoundedVec<Pos, 3>,
}

/// A street edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiStreet {
    /// Its ends, as an action's `EdgeEnds` names them.
    pub a: Pos,
    pub b: Pos,
    /// Its street template, as a road build names one.
    pub template: ResName,
    /// Its length, in metres.
    pub length: u32,
    /// Owned by a company (a player's road), not the town's.
    pub owned: bool,
    /// The stops and other objects on it.
    pub objects: u8,
    /// A bridge or tunnel.
    pub structure: bool,
}

/// A vehicle model a company may buy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiModel {
    /// Its file, as a purchase's part names it.
    pub model: ResName,
    /// What it costs, and costs a year to run, in whole units.
    pub price: i64,
    pub running: i64,
    /// What it carries, and how fast it goes, in km/h.
    pub capacity: u32,
    pub speed: u32,
    /// How many years it lasts.
    pub lifespan: u32,
}

/// The game's verdict on a build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiVerdict {
    /// The game would build it.
    pub ok: bool,
    /// What it would cost, in whole units.
    pub cost: i64,
    /// What the game says of it: why not, or what it warns of.
    pub messages: BoundedVec<Text<160>, MAX_AI_MESSAGES>,
    /// What it would collide with.
    pub collisions: u16,
}
