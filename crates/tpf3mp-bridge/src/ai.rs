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
    action::{LineId, LoanTerms, Pos, ResName, StationId, TownId, VehicleId},
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
/// Most industries a [`AiAnswer::Industries`] lists.
pub const MAX_AI_INDUSTRIES: usize = 160;
/// Most cargos an industry takes or makes, as an [`AiIndustry`] lists them.
pub const MAX_AI_CARGOS: usize = 8;
/// Most cargo needs an [`AiTown`] lists: the least supplied.
pub const MAX_AI_NEEDS: usize = 4;
/// Most cells an [`AiLand`] holds: a grid of 24 by 24.
pub const MAX_AI_LAND: usize = 576;
/// Most track ends an [`AiStation`] lists.
pub const MAX_AI_ENDS: usize = 8;

/// What stands on a cell of an [`AiLand`], a bit each.
pub mod land {
    /// Water.
    pub const WATER: u8 = 1;
    /// A construction: a town building, an industry, a station, a depot.
    pub const BUILT: u8 = 2;
    /// A street.
    pub const STREET: u8 = 4;
    /// A track.
    pub const TRACK: u8 = 8;
}

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
    /// by the game's name (`BUS`, `TRUCK`, `TRAIN`, ...); with `cargo`,
    /// only those that carry it, each with its capacity for it.
    Models {
        mode: Text<16>,
        cargo: Option<ResName>,
    },
    /// The world's industries, what each takes and makes ([`AiIndustry`]).
    Industries,
    /// Whether a truck can drive from the street network by `from` to the
    /// street network by `to`, by the game's pathfinder ([`AiRoute`]).
    Route { from: Pos, to: Pos },
    /// The land on a grid of `cells` by `cells` cells of `step` metres, the
    /// first cell's centre at `at`, in rows of rising y, each of rising x
    /// ([`AiLand`]): where a track can go.
    Land { at: Pos, cells: u8, step: u16 },
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
    Industries(BoundedVec<AiIndustry, MAX_AI_INDUSTRIES>),
    Route(AiRoute),
    Land(AiLand),
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
    /// The loans the game offers the company now, on their terms: a
    /// founded company starts with no money and borrows (tpf3mp/companies.lua,
    /// "loans").
    pub offers: BoundedVec<LoanTerms, 8>,
    /// The loans it owes.
    pub loans: u8,
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
    /// A rail station's track ends: where its construction's tracks meet
    /// the world, and whether nothing meets them yet.
    pub ends: BoundedVec<AiEnd, MAX_AI_ENDS>,
}

/// A station track's end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiEnd {
    pub pos: Pos,
    pub open: bool,
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
    /// How far through its life it is, in percent of its lifespan (over
    /// 100 once worn out), as the game's vehicle window reckons it: its
    /// oldest part's purchase against the game's time.
    pub worn: u16,
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
    /// The cargo its buildings take, the least supplied first: where cargo
    /// trucks carry is wanted (an industry takes its inputs only while what
    /// it makes is carried on, as towns take what they need; 2026-10-30).
    pub needs: BoundedVec<AiNeed, MAX_AI_NEEDS>,
}

/// A cargo a town takes: how much it was supplied and may take, as the
/// town window's cargo demand reads them (`getCargoSupplyAndLimit`), and the
/// centre of the district that takes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiNeed {
    pub cargo: ResName,
    pub supply: u32,
    pub limit: u32,
    pub at: Pos,
}

/// An industry: where it stands, the cargo it takes, and what it makes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiIndustry {
    pub name: Text<48>,
    /// Its construction's origin.
    pub pos: Pos,
    /// Its own station, by the room's id for its group, and the station's
    /// terminals: a line loads its cargo there and unloads what it takes
    /// there (build 40425: players' cargo lines stop at an industry's own
    /// station; a street stop only unloads).
    pub station: Option<StationId>,
    pub terminals: u8,
    /// The cargo it takes in, by file (`::/cargos/grain/grain.cargo`).
    pub inputs: BoundedVec<ResName, MAX_AI_CARGOS>,
    /// What it makes.
    pub outputs: BoundedVec<AiOutput, MAX_AI_CARGOS>,
    /// It stands on water, as fishing grounds do: no road or rail reaches
    /// it.
    pub water: bool,
}

/// One cargo an industry makes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiOutput {
    pub cargo: ResName,
    /// The game's number for the cargo type: a stop's load rules list one
    /// entry for each, by this number (build 40425).
    pub index: u16,
    /// How much it made over the last year, and could make at most.
    pub per_year: u32,
    pub most: u32,
}

/// Whether two places are joined by street for trucks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiRoute {
    pub connected: bool,
    /// The edges of the path the game found; 0 for none.
    pub edges: u32,
}

/// A street edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiStreet {
    /// Its ends, as an action's `EdgeEnds` names them.
    pub a: Pos,
    pub b: Pos,
    /// Its curve's middle: where the AI stops buses on it, or splits it.
    pub mid: Pos,
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
    /// A junction at one of its ends has lane settings, and a company's
    /// edge meets it there: a split of it rebuilds those settings, which
    /// the room refuses for an edge not the acting company's (D21,
    /// `tpf3mp/junctions.lua`), though the game's verdict, a preview,
    /// leaves the settings out.
    pub company_junction: bool,
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
    /// A land vehicle's engine: its power in kW and tractive effort in kN,
    /// 0 for a vehicle with none (a wagon); its weight empty in tonnes; its
    /// length in decimetres (`landVehicle`, `extent`).
    pub power: u32,
    pub tractive: u32,
    pub weight: u32,
    pub length: u32,
    /// Its engine is electric (the game's engine type 2): it runs only
    /// under catenary.
    pub electric: bool,
}

/// The land on a grid, cell by cell in the order of
/// [`AiQuestion::Land`]: each cell's height at its centre, in
/// centimetres, and what stands on it ([`land`] bits).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiLand {
    pub heights: BoundedVec<i32, MAX_AI_LAND>,
    pub flags: BoundedVec<u8, MAX_AI_LAND>,
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
    /// How many things that stand it would collide with: another street, a
    /// track, an industry; not what the build itself makes (a depot's own
    /// street, split for its entrance, which the game names among its
    /// collisions with "Collision" said of every depot it builds).
    pub collisions: u16,
}
