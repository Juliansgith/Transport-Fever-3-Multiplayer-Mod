//! Rail lines as plain geometry over what the game told the AI
//! (docs/AI_PLAYERS.md, "Track and trains"): a one-track cargo station and
//! where its track ends, a rail depot beyond a station's end, the land on a
//! grid, a route over it from one station's end to the other's that a train
//! can run, smoothed into track, and that track cut into builds the game is
//! asked about one at a time. Every action made here is put to the game's
//! verdict before it is sent.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use tpf3mp_bridge::ai::{AiLand, land};
use tpf3mp_proto::{
    BoundedVec, Text,
    action::{
        ConstructionBuild, Link, Param, ParamValue, Polyline, Pos, ResName, Resolve, Structure,
        Tangent, TrackBuild, Transform, Vertex,
    },
};

use crate::place::P;

/// The game's modular rail station.
pub const STATION: &str = "::/stations/rail/modular_station/modular_station.con";
/// The game's rail depot.
pub const DEPOT: &str = "::/depots/rail/rail_depot.con";
/// A station module's length along its track, in metres
/// (`modular_station.script.lua`, `platformLength`).
pub const MODULE: f64 = 40.0;
/// Where a one-track cargo station's track runs, from its origin across
/// its own x axis: its track modules are its third row, 5 m each
/// (`modular_station.script.lua`: `AddTrack(2, ...)`; the fixture's cargo
/// station's ends read at local (10, -140) and (10, 180)).
pub const STATION_TRACK_X: f64 = 10.0;
/// Where a rail depot's track meets the world's, on its own axes (the
/// fixture's depot: its boundary node at local (-2.2, -70)).
pub const DEPOT_JOIN: (f64, f64) = (-2.2, -70.0);
/// How far beyond a station's outer end a depot's track joins, in metres.
pub const DEPOT_GAP: f64 = 30.0;
/// The steepest a line climbs, and its tightest curve, in metres: well
/// inside the simple track's limits (8.5 % and 55 m to build), for early
/// locomotives and a line's speed.
pub const MAX_GRADE: f64 = 0.025;
pub const MIN_RADIUS: f64 = 180.0;
/// The land grid's cell, in metres, and a tile's cells a side: a tile is
/// one question, about 7 ms of the driving game's frame.
pub const STEP: f64 = 40.0;
pub const TILE: u8 = 16;
/// How far beyond the line's two ends the land is read, in metres.
pub const MARGIN: f64 = 480.0;
/// How far the line runs straight out of a station before the route
/// turns, in metres: two cells.
pub const LEAD: f64 = 2.0 * STEP;
/// How long a track edge is, about, and how many edges one build lays.
pub const EDGE: f64 = 100.0;
pub const PIECE_EDGES: usize = 6;
/// Above the ground by more than this at an edge's middle, it is a bridge.
pub const BRIDGE_ABOVE: f64 = 5.0;
/// The bridge it builds: stone, offered every year, for rail and road
/// (`bridge/stone.bridge.lua`), as `bridgeTypeRep` names it.
pub const BRIDGE: &str = "::/infrastructure/bridge/stone.bridge";

fn text<const N: usize>(s: &str) -> Text<N> {
    Text::lossy(s)
}

fn res(s: &str) -> ResName {
    text(s)
}

/// The track TF3 lays in `year`: simple until standard comes in 1930
/// (`track/standard/standard.street_template.lua`).
pub fn track_template(year: i32) -> &'static str {
    if year >= 1930 {
        "::/infrastructure/track/standard/standard.street_template"
    } else {
        "::/infrastructure/track/simple/simple.street_template"
    }
}

/// The same track as a station's track module names it.
fn track_module(year: i32) -> &'static str {
    if year >= 1930 {
        "::/trainstation___/infrastructure/track/standard/standard.street_template"
    } else {
        "::/trainstation___/infrastructure/track/simple/simple.street_template"
    }
}

/// A universal cargo platform of `year`'s era.
fn platform_module(year: i32) -> String {
    let era = if year >= 1990 {
        "c"
    } else if year >= 1950 {
        "b"
    } else {
        "a"
    };
    format!("::/stations/rail/modular_station/platform_cargo_era_{era}.module")
}

/// A unit vector at `heading` radians.
fn unit(heading: f64) -> (f64, f64) {
    (heading.cos(), heading.sin())
}

/// A one-track cargo station: its origin, the heading of its track (its
/// own +y axis), and its track modules from `first` to `last` (0 its
/// middle, each [`MODULE`] long).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Station {
    pub origin: P,
    pub heading: f64,
    pub first: i32,
    pub last: i32,
}

impl Station {
    /// A station of three modules, its inner end (`last`) towards
    /// `heading`.
    pub fn new(origin: P, heading: f64) -> Self {
        Self {
            origin,
            heading,
            first: -1,
            last: 1,
        }
    }

    /// A point on its own axes: x across, y along its track.
    pub fn local(&self, lx: f64, ly: f64) -> P {
        let (ux, uy) = unit(self.heading);
        // Its x axis is its y axis turned a quarter clockwise.
        let (xx, xy) = (uy, -ux);
        P {
            x: self.origin.x + lx * xx + ly * ux,
            y: self.origin.y + lx * xy + ly * uy,
            z: self.origin.z,
        }
    }

    /// Its track's end towards `heading`, where the line leaves.
    pub fn inner(&self) -> P {
        self.local(
            STATION_TRACK_X,
            f64::from(self.last) * MODULE + MODULE / 2.0,
        )
    }

    /// Its track's other end, where a depot joins.
    pub fn outer(&self) -> P {
        self.local(
            STATION_TRACK_X,
            f64::from(self.first) * MODULE - MODULE / 2.0,
        )
    }

    /// Its line's end and the heading out of it, whichever end is nearer
    /// `other`; then its other end and the heading out of that.
    pub fn ends_toward(&self, other: P) -> ((P, f64), (P, f64)) {
        let inner = (self.inner(), self.heading);
        let outer = (self.outer(), self.heading + std::f64::consts::PI);
        if inner.0.flat_distance(other) <= outer.0.flat_distance(other) {
            (inner, outer)
        } else {
            (outer, inner)
        }
    }

    /// Points over its ground every 20 m: its building's side, its
    /// platforms and its track, end to end.
    pub fn footprint(&self) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let first = f64::from(self.first) * MODULE - MODULE / 2.0;
        let last = f64::from(self.last) * MODULE + MODULE / 2.0;
        let mut y = first;
        while y <= last + 1e-9 {
            for x in [-15.0, 0.0, STATION_TRACK_X] {
                let p = self.local(x, y);
                out.push((p.x, p.y));
            }
            y += 20.0;
        }
        out
    }

    /// The station as the game's station tool lays it for a cargo station
    /// of one track: universal platforms along its first row, the track on
    /// its third, the small cargo building (`createTemplateFn`; the fixture's
    /// stations carry their modules' names alone).
    pub fn build(&self, year: i32, seed: i64, name: &str) -> Option<ConstructionBuild> {
        let int = |key: &str, v: i64| Param {
            key: text(key),
            value: ParamValue::Int(v),
        };
        let module = |slot: i64, file: &str| {
            [
                Param {
                    key: text(&format!("modules[{slot}].name")),
                    value: ParamValue::Text(text(file)),
                },
                Param {
                    key: text(&format!("modules[{slot}].variant")),
                    value: ParamValue::Int(0),
                },
            ]
        };
        let modules = self.last - self.first;
        // The station tool's length parameter: the index of its module
        // count less one in { 0, 1, 2, 3, 5, 7, 9 }.
        let length = [0, 1, 2, 3, 5, 7, 9].iter().position(|n| *n == modules)?;
        let mut params = vec![
            int("seed", seed),
            int("year", i64::from(year)),
            int("tracks", 1),
            int("length", i64::try_from(length).ok()?),
            // Universal, counted from 1.
            int("specialization", 1),
        ];
        let platform = platform_module(year);
        for j in self.first..=self.last {
            params.extend(module(6_400_000 + 10 * i64::from(j), &platform));
            params.extend(module(8_402_000 + 10 * i64::from(j), track_module(year)));
        }
        // A through station's building: its slot by whether its module
        // count is even.
        let k = if modules % 2 == 0 { 0 } else { 2 };
        params.extend(module(
            3_400_000 + 10 * k,
            "::/stations/rail/modular_station/main_building_1_cargo.module",
        ));
        Some(ConstructionBuild {
            file: res(STATION),
            transform: transform(self.origin, self.heading),
            params: BoundedVec::new(params).ok()?,
            name: text(name),
            replaces: None,
            connection: None,
        })
    }
}

/// A construction's transform at `origin`, its own +y axis at `heading`.
#[allow(clippy::cast_possible_truncation)]
fn transform(origin: P, heading: f64) -> Transform {
    let micro = |v: f64| (v * 1_000_000.0).round() as i32;
    let (ux, uy) = unit(heading);
    // Its x axis, (cos, sin) of the basis, is its y axis turned a quarter
    // clockwise.
    let (cos, sin) = (uy, -ux);
    Transform {
        basis: [
            micro(cos),
            micro(sin),
            0,
            micro(-sin),
            micro(cos),
            0,
            0,
            0,
            1_000_000,
        ],
        origin: origin.pos(),
    }
}

/// A rail depot beyond a station's `end`, its track joining [`DEPOT_GAP`]
/// beyond it in line with the station's, out at `out_heading`: the depot
/// and where its track joins.
pub fn depot(
    end: P,
    out_heading: f64,
    year: i32,
    seed: i64,
    name: &str,
) -> Option<(P, ConstructionBuild)> {
    let (ox, oy) = unit(out_heading);
    let join = P {
        x: end.x + ox * DEPOT_GAP,
        y: end.y + oy * DEPOT_GAP,
        z: end.z,
    };
    // The depot's +y axis points away from the station; its join is at
    // DEPOT_JOIN on its own axes.
    let (xx, xy) = (oy, -ox);
    let origin = P {
        x: join.x - DEPOT_JOIN.0 * xx - DEPOT_JOIN.1 * ox,
        y: join.y - DEPOT_JOIN.0 * xy - DEPOT_JOIN.1 * oy,
        z: join.z,
    };
    let params = vec![
        Param {
            key: text("seed"),
            value: ParamValue::Int(seed),
        },
        Param {
            key: text("year"),
            value: ParamValue::Int(i64::from(year)),
        },
    ];
    Some((
        join,
        ConstructionBuild {
            file: res(DEPOT),
            transform: transform(origin, out_heading),
            params: BoundedVec::new(params).ok()?,
            name: text(name),
            replaces: None,
            connection: None,
        },
    ))
}

/// Station placements around `target` for a line towards `other`: on
/// rings of `radii` metres, 16 bearings round, each turned so its building
/// (on its own -x side) faces `target`, at the ground's height; those whose
/// ground the land shows built on, a street, a track or water left out.
/// The nearest ring first, and on each the bearings nearest `other` first.
#[allow(clippy::cast_precision_loss)]
pub fn sites(land: &Land, target: P, other: P, radii: &[f64]) -> Vec<Station> {
    let towards = (other.y - target.y).atan2(other.x - target.x);
    let mut out = Vec::new();
    for r in radii {
        let mut ring: Vec<(f64, Station)> = (0..16)
            .map(|k| {
                let a = std::f64::consts::TAU * f64::from(k) / 16.0;
                let (x, y) = (target.x + r * a.cos(), target.y + r * a.sin());
                let origin = P {
                    x,
                    y,
                    z: land.height(x, y),
                };
                (
                    angle_between(a, towards),
                    Station::new(origin, a + std::f64::consts::FRAC_PI_2),
                )
            })
            .filter(|(_, s)| {
                s.footprint().iter().all(|(x, y)| {
                    land.flags_at(*x, *y).is_some_and(|f| {
                        f & (land::BUILT | land::STREET | land::TRACK | land::WATER) == 0
                    })
                })
            })
            .collect();
        ring.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.extend(ring.into_iter().map(|(_, s)| s));
    }
    out
}

/// The land read for a line: heights in metres and what stands on each
/// cell ([`land`] bits), on a grid of [`STEP`] whose first cell's centre is
/// `(x0, y0)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Land {
    pub x0: f64,
    pub y0: f64,
    pub w: usize,
    pub h: usize,
    pub z: Vec<f64>,
    pub flags: Vec<u8>,
    /// Which tiles have been read.
    pub read: Vec<bool>,
}

impl Land {
    /// The grid around a line from `a` to `b`, [`MARGIN`] beyond both,
    /// whole tiles, nothing read yet.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn around(a: P, b: P) -> Self {
        let tile = f64::from(TILE) * STEP;
        let (minx, maxx) = (a.x.min(b.x) - MARGIN, a.x.max(b.x) + MARGIN);
        let (miny, maxy) = (a.y.min(b.y) - MARGIN, a.y.max(b.y) + MARGIN);
        let tw = ((maxx - minx) / tile).ceil().max(1.0) as usize;
        let th = ((maxy - miny) / tile).ceil().max(1.0) as usize;
        let (w, h) = (tw * usize::from(TILE), th * usize::from(TILE));
        Self {
            x0: minx + STEP / 2.0,
            y0: miny + STEP / 2.0,
            w,
            h,
            z: vec![0.0; w * h],
            flags: vec![0; w * h],
            read: vec![false; tw * th],
        }
    }

    fn tiles_wide(&self) -> usize {
        self.w / usize::from(TILE)
    }

    /// The next tile not read yet: its number and its first cell's centre.
    #[allow(clippy::cast_precision_loss)]
    pub fn next_tile(&self) -> Option<(usize, Pos)> {
        let n = self.read.iter().position(|r| !r)?;
        let tw = self.tiles_wide();
        let (tx, ty) = (n % tw, n / tw);
        let tile = f64::from(TILE) * STEP;
        let at = P {
            x: self.x0 + tx as f64 * tile,
            y: self.y0 + ty as f64 * tile,
            z: 0.0,
        };
        Some((n, at.pos()))
    }

    /// The game's answer for tile `n`, put in its place.
    pub fn put(&mut self, n: usize, answer: &AiLand) {
        let t = usize::from(TILE);
        let tw = self.tiles_wide();
        let (tx, ty) = (n % tw, n / tw);
        for j in 0..t {
            for i in 0..t {
                let k = j * t + i;
                let cell = (ty * t + j) * self.w + tx * t + i;
                if let (Some(z), Some(f)) = (answer.heights.get(k), answer.flags.get(k))
                    && cell < self.z.len()
                {
                    self.z[cell] = f64::from(*z) / 100.0;
                    self.flags[cell] = *f;
                }
            }
        }
        if let Some(r) = self.read.get_mut(n) {
            *r = true;
        }
    }

    /// The cell a point falls in, if on the grid.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn cell(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let i = ((x - self.x0) / STEP + 0.5).floor();
        let j = ((y - self.y0) / STEP + 0.5).floor();
        if i < 0.0 || j < 0.0 {
            return None;
        }
        let (i, j) = (i as usize, j as usize);
        (i < self.w && j < self.h).then_some((i, j))
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn centre(&self, i: usize, j: usize) -> (f64, f64) {
        (self.x0 + i as f64 * STEP, self.y0 + j as f64 * STEP)
    }

    fn flags_at(&self, x: f64, y: f64) -> Option<u8> {
        self.cell(x, y).map(|(i, j)| self.flags[j * self.w + i])
    }

    /// The ground's height at a point, between the cells' centres.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn height(&self, x: f64, y: f64) -> f64 {
        let fx = ((x - self.x0) / STEP).clamp(0.0, (self.w - 1) as f64);
        let fy = ((y - self.y0) / STEP).clamp(0.0, (self.h - 1) as f64);
        let (i, j) = (fx.floor() as usize, fy.floor() as usize);
        let (i1, j1) = ((i + 1).min(self.w - 1), (j + 1).min(self.h - 1));
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        let z = |i: usize, j: usize| self.z[j * self.w + i];
        let a = z(i, j) * (1.0 - tx) + z(i1, j) * tx;
        let b = z(i, j1) * (1.0 - tx) + z(i1, j1) * tx;
        a * (1.0 - ty) + b * ty
    }

    /// Marks the cells within `r` metres of a point as built on: where the
    /// game refused a piece, the route goes round.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn block(&mut self, x: f64, y: f64, r: f64) {
        let cells = (r / STEP).ceil() as i64;
        let Some((ci, cj)) = self.cell(x, y) else {
            return;
        };
        for dj in -cells..=cells {
            for di in -cells..=cells {
                let (i, j) = (ci as i64 + di, cj as i64 + dj);
                if i < 0 || j < 0 || i >= self.w as i64 || j >= self.h as i64 {
                    continue;
                }
                let (cx, cy) = self.centre(i as usize, j as usize);
                if (cx - x).hypot(cy - y) <= r + STEP / 2.0 {
                    self.flags[j as usize * self.w + i as usize] |= land::BUILT;
                }
            }
        }
    }
}

/// The 16 headings a route moves in, as grid steps: east first, turning
/// left.
const MOVES: [(i64, i64); 16] = [
    (1, 0),
    (2, 1),
    (1, 1),
    (1, 2),
    (0, 1),
    (-1, 2),
    (-1, 1),
    (-2, 1),
    (-1, 0),
    (-2, -1),
    (-1, -1),
    (-1, -2),
    (0, -1),
    (1, -2),
    (1, -1),
    (2, -1),
];

#[allow(clippy::cast_precision_loss)]
fn move_heading(d: usize) -> f64 {
    let (x, y) = MOVES[d];
    (y as f64).atan2(x as f64)
}

/// The move heading nearest `heading`.
fn nearest_move(heading: f64) -> usize {
    (0..16)
        .min_by(|a, b| {
            angle_between(move_heading(*a), heading)
                .total_cmp(&angle_between(move_heading(*b), heading))
        })
        .unwrap_or(0)
}

fn angle_between(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(std::f64::consts::TAU);
    d.min(std::f64::consts::TAU - d)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct State {
    i: usize,
    j: usize,
    d: usize,
    /// It turned on the move that brought it here: it may not turn again.
    turned: bool,
}

struct Open {
    f: f64,
    g: f64,
    state: State,
}

impl PartialEq for Open {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f
    }
}
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.total_cmp(&self.f)
    }
}

/// What a step of route costs over the cost of its length, a factor of its
/// length, or none where it may not go: over built land or another track.
fn step_cost(land: &Land, a: (f64, f64), b: (f64, f64)) -> Option<f64> {
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    let mut extra = 0.0;
    for k in 1..=4 {
        let t = f64::from(k) / 4.0;
        let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        let f = land.flags_at(x, y)?;
        if f & (land::BUILT | land::TRACK) != 0 {
            return None;
        }
        // A street is crossed, never followed: each quarter on one costs.
        if f & land::STREET != 0 {
            extra += 2.5;
        }
        // Water is bridged.
        if f & land::WATER != 0 {
            extra += 2.0;
        }
    }
    let grade = (land.height(b.0, b.1) - land.height(a.0, a.1)).abs() / len;
    extra += 20.0 * grade + 200.0 * (grade - 2.0 * MAX_GRADE).max(0.0);
    Some(1.0 + extra)
}

/// The most states a route search opens before it gives up.
pub const SEARCH_MOST: usize = 600_000;

/// A route over `land` from `from`, leaving at `from_heading`, to `to`,
/// arriving at `to_heading`: the cells' centres it passes, from the first
/// clear cell [`LEAD`] out of `from` to the one [`LEAD`] short of `to`.
/// It turns 22.5° at most a step, never on two steps running, and keeps
/// off built land and other tracks.
#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
pub fn route(
    land: &Land,
    from: P,
    from_heading: f64,
    to: P,
    to_heading: f64,
) -> Option<Vec<(f64, f64)>> {
    let (fx, fy) = unit(from_heading);
    let (tx, ty) = unit(to_heading);
    let start = land.cell(from.x + fx * LEAD, from.y + fy * LEAD)?;
    let goal = land.cell(to.x - tx * LEAD, to.y - ty * LEAD)?;
    let (goal_x, goal_y) = land.centre(goal.0, goal.1);
    let goal_d = nearest_move(to_heading);
    let start = State {
        i: start.0,
        j: start.1,
        d: nearest_move(from_heading),
        turned: true,
    };
    let h = |i: usize, j: usize| {
        let (x, y) = land.centre(i, j);
        (x - goal_x).hypot(y - goal_y)
    };
    let mut open = BinaryHeap::new();
    let mut best: HashMap<State, f64> = HashMap::new();
    let mut came: HashMap<State, State> = HashMap::new();
    best.insert(start, 0.0);
    open.push(Open {
        f: h(start.i, start.j),
        g: 0.0,
        state: start,
    });
    let mut opened = 0usize;
    while let Some(Open { g, state, .. }) = open.pop() {
        if best.get(&state).is_some_and(|b| *b < g) {
            continue;
        }
        if (state.i, state.j) == goal && (state.d + 16 - goal_d) % 16 <= 1
            || (goal_d + 16 - state.d) % 16 <= 1 && (state.i, state.j) == goal
        {
            let mut cells = vec![land.centre(state.i, state.j)];
            let mut at = state;
            while let Some(prev) = came.get(&at) {
                cells.push(land.centre(prev.i, prev.j));
                at = *prev;
            }
            cells.reverse();
            return Some(cells);
        }
        opened += 1;
        if opened > SEARCH_MOST {
            return None;
        }
        let here = land.centre(state.i, state.j);
        for turn in [-1i64, 0, 1] {
            if turn != 0 && state.turned {
                continue;
            }
            let d = (state.d as i64 + 16 + turn) as usize % 16;
            let (mx, my) = MOVES[d];
            let (ni, nj) = (state.i as i64 + mx, state.j as i64 + my);
            if ni < 0 || nj < 0 || ni >= land.w as i64 || nj >= land.h as i64 {
                continue;
            }
            let (ni, nj) = (ni as usize, nj as usize);
            let there = land.centre(ni, nj);
            let Some(factor) = step_cost(land, here, there) else {
                continue;
            };
            let len = (there.0 - here.0).hypot(there.1 - here.1);
            let turn_cost = if turn == 0 { 0.0 } else { 0.25 * len };
            let ng = g + len * factor + turn_cost;
            let next = State {
                i: ni,
                j: nj,
                d,
                turned: turn != 0,
            };
            if best.get(&next).is_none_or(|b| ng < *b) {
                best.insert(next, ng);
                came.insert(next, state);
                open.push(Open {
                    f: ng + h(ni, nj),
                    g: ng,
                    state: next,
                });
            }
        }
    }
    None
}

/// A track node of a line: where it is, the heading of the track through
/// it, and whether the edge from it to the next is a bridge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub at: P,
    pub heading: f64,
    pub bridge: bool,
}

/// The line's track from `from` (leaving at `from_heading`) to `to`
/// (arriving at `to_heading`) through the route's cells: nodes about
/// [`EDGE`] apart, smoothed so no curve is tighter than [`MIN_RADIUS`]
/// where the route allows, at heights the ground gives within
/// [`MAX_GRADE`], bridged where it stands high or over water. Both ends
/// keep their stations' heights.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn track(
    land: &Land,
    from: P,
    from_heading: f64,
    to: P,
    to_heading: f64,
    cells: &[(f64, f64)],
) -> Vec<Node> {
    let (fx, fy) = unit(from_heading);
    let (tx, ty) = unit(to_heading);
    // Straight out of each station along its track, then the route.
    let mut path: Vec<(f64, f64)> =
        vec![(from.x, from.y), (from.x + fx * LEAD, from.y + fy * LEAD)];
    path.extend(cells.iter().copied().skip(1));
    if path.len() > 2 {
        path.pop();
    }
    path.push((to.x - tx * LEAD, to.y - ty * LEAD));
    path.push((to.x, to.y));
    // Points about EDGE apart along it, its ends kept.
    let mut lengths = vec![0.0];
    for w in path.windows(2) {
        let l = lengths.last().copied().unwrap_or(0.0) + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        lengths.push(l);
    }
    let total = lengths.last().copied().unwrap_or(0.0);
    let edges = (total / EDGE).round().max(1.0) as usize;
    let at = |s: f64| {
        let k = lengths.partition_point(|l| *l < s).clamp(1, path.len() - 1);
        let (l0, l1) = (lengths[k - 1], lengths[k]);
        let t = if l1 > l0 { (s - l0) / (l1 - l0) } else { 0.0 };
        (
            path[k - 1].0 + (path[k].0 - path[k - 1].0) * t,
            path[k - 1].1 + (path[k].1 - path[k - 1].1) * t,
        )
    };
    let mut points: Vec<(f64, f64)> = (0..=edges)
        .map(|k| at(total * k as f64 / edges as f64))
        .collect();
    // Smoothed: each inner point drawn towards its neighbours' middle, the
    // two next to either end held on their station's axis.
    let n = points.len();
    for _ in 0..12 {
        let before = points.clone();
        for k in 2..n.saturating_sub(2) {
            let mid = (
                (before[k - 1].0 + before[k + 1].0) / 2.0,
                (before[k - 1].1 + before[k + 1].1) / 2.0,
            );
            points[k] = (
                before[k].0 * 0.5 + mid.0 * 0.5,
                before[k].1 * 0.5 + mid.1 * 0.5,
            );
        }
    }
    if n >= 3 {
        let d1 = (points[1].0 - points[0].0).hypot(points[1].1 - points[0].1);
        points[1] = (from.x + fx * d1, from.y + fy * d1);
        let dn = (points[n - 1].0 - points[n - 2].0).hypot(points[n - 1].1 - points[n - 2].1);
        points[n - 2] = (to.x - tx * dn, to.y - ty * dn);
    }
    // Headings: each end its station's; between, along its neighbours.
    let headings: Vec<f64> = (0..n)
        .map(|k| {
            if k == 0 {
                from_heading
            } else if k == n - 1 {
                to_heading
            } else {
                (points[k + 1].1 - points[k - 1].1).atan2(points[k + 1].0 - points[k - 1].0)
            }
        })
        .collect();
    // Heights: the ground's, within the grade of each other, the ends the
    // stations'.
    let mut along = vec![0.0];
    for w in points.windows(2) {
        along.push(along.last().copied().unwrap_or(0.0) + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
    }
    let ground: Vec<f64> = points.iter().map(|p| land.height(p.0, p.1)).collect();
    let mut z = ground.clone();
    z[0] = from.z;
    z[n - 1] = to.z;
    for _ in 0..4 {
        for k in 1..n - 1 {
            let d = along[k] - along[k - 1];
            z[k] = z[k].clamp(z[k - 1] - MAX_GRADE * d, z[k - 1] + MAX_GRADE * d);
        }
        for k in (1..n - 1).rev() {
            let d = along[k + 1] - along[k];
            z[k] = z[k].clamp(z[k + 1] - MAX_GRADE * d, z[k + 1] + MAX_GRADE * d);
        }
        for k in 1..n - 1 {
            z[k] = 0.25 * z[k - 1] + 0.5 * z[k] + 0.25 * z[k + 1];
        }
    }
    (0..n)
        .map(|k| {
            let bridge = k + 1 < n && {
                let (mx, my) = (
                    (points[k].0 + points[k + 1].0) / 2.0,
                    (points[k].1 + points[k + 1].1) / 2.0,
                );
                let wet =
                    |x: f64, y: f64| land.flags_at(x, y).is_some_and(|f| f & land::WATER != 0);
                let high = (z[k] + z[k + 1]) / 2.0 - land.height(mx, my) > BRIDGE_ABOVE;
                high || wet(mx, my)
                    || wet(points[k].0, points[k].1) && wet(points[k + 1].0, points[k + 1].1)
            };
            Node {
                at: P {
                    x: points[k].0,
                    y: points[k].1,
                    z: z[k],
                },
                heading: headings[k],
                bridge,
            }
        })
        .collect()
}

/// The tightest curve along `nodes`, in metres: each inner node's turn
/// over its neighbours' spacing.
pub fn tightest(nodes: &[Node]) -> f64 {
    nodes
        .windows(3)
        .map(|w| {
            let (a, b, c) = (w[0].at, w[1].at, w[2].at);
            let h0 = (b.y - a.y).atan2(b.x - a.x);
            let h1 = (c.y - b.y).atan2(c.x - b.x);
            let turn = angle_between(h0, h1);
            let span = (a.flat_distance(b) + b.flat_distance(c)) / 2.0;
            if turn < 1e-6 {
                f64::INFINITY
            } else {
                span / turn
            }
        })
        .fold(f64::INFINITY, f64::min)
}

/// The track builds for `nodes`, [`PIECE_EDGES`] edges each: every piece
/// starts on the track node the one before it laid (the first on the
/// station's end), the last ends on the other station's. Each edge's
/// tangents follow the track's heading, their length the edge's.
pub fn pieces(nodes: &[Node], year: i32) -> Vec<TrackBuild> {
    let mut out = Vec::new();
    let last = nodes.len().saturating_sub(1);
    let mut k = 0;
    while k < last {
        let end = (k + PIECE_EDGES).min(last);
        let vertices: Vec<Vertex> = (k..=end)
            .map(|i| Vertex {
                pos: nodes[i].at.pos(),
                resolve: if i == k || i == last {
                    Resolve::Node(tpf3mp_proto::action::Network::Track)
                } else {
                    Resolve::New
                },
            })
            .collect();
        let links: Vec<Link> = (k..end)
            .map(|i| {
                let (a, b) = (nodes[i], nodes[i + 1]);
                let len = a.at.flat_distance(b.at);
                let dz = b.at.z - a.at.z;
                let tangent = |heading: f64| {
                    let (ux, uy) = unit(heading);
                    mm_tangent(ux * len, uy * len, dz)
                };
                Link {
                    from: u16::try_from(i - k).unwrap_or(0),
                    to: u16::try_from(i + 1 - k).unwrap_or(0),
                    tangent0: tangent(a.heading),
                    tangent1: tangent(b.heading),
                    structure: if a.bridge {
                        Structure::Bridge(res(BRIDGE))
                    } else {
                        Structure::Ground
                    },
                    kind: None,
                    decorations: BoundedVec::default(),
                    locked: false,
                    owned: true,
                    lanes: BoundedVec::default(),
                    precedence: None,
                    position0: None,
                    position1: None,
                    kept: BoundedVec::default(),
                }
            })
            .collect();
        if let (Ok(vertices), Ok(links)) = (BoundedVec::new(vertices), BoundedVec::new(links))
            && let Ok(polyline) = Polyline::new(vertices, links, BoundedVec::default())
        {
            out.push(TrackBuild {
                track: res(track_template(year)),
                style: None,
                catenary: false,
                polyline,
                crossing_type: None,
            });
        }
        k = end;
    }
    out
}

#[allow(clippy::cast_possible_truncation)]
fn mm_tangent(x: f64, y: f64, z: f64) -> Tangent {
    Tangent {
        x: (x * 1000.0).round() as i32,
        y: (y * 1000.0).round() as i32,
        z: (z * 1000.0).round() as i32,
    }
}

/// A short track from one existing node to another, straight: a depot's
/// track to the station's end.
pub fn link(from: P, to: P, year: i32) -> Option<TrackBuild> {
    let t = mm_tangent(to.x - from.x, to.y - from.y, to.z - from.z);
    let vertices = vec![
        Vertex {
            pos: from.pos(),
            resolve: Resolve::Node(tpf3mp_proto::action::Network::Track),
        },
        Vertex {
            pos: to.pos(),
            resolve: Resolve::Node(tpf3mp_proto::action::Network::Track),
        },
    ];
    let links = vec![Link {
        from: 0,
        to: 1,
        tangent0: t,
        tangent1: t,
        structure: Structure::Ground,
        kind: None,
        decorations: BoundedVec::default(),
        locked: false,
        owned: true,
        lanes: BoundedVec::default(),
        precedence: None,
        position0: None,
        position1: None,
        kept: BoundedVec::default(),
    }];
    Some(TrackBuild {
        track: res(track_template(year)),
        style: None,
        catenary: false,
        polyline: Polyline::new(
            BoundedVec::new(vertices).ok()?,
            BoundedVec::new(links).ok()?,
            BoundedVec::default(),
        )
        .ok()?,
        crossing_type: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> P {
        P { x, y, z: 10.0 }
    }

    /// Flat land at 10 m, read whole, with `blocked` cells built on.
    fn flat(a: P, b: P, blocked: impl Fn(f64, f64) -> bool) -> Land {
        let mut land = Land::around(a, b);
        for j in 0..land.h {
            for i in 0..land.w {
                let (x, y) = land.centre(i, j);
                land.z[j * land.w + i] = 10.0;
                if blocked(x, y) {
                    land.flags[j * land.w + i] = land::BUILT;
                }
            }
        }
        land.read.iter_mut().for_each(|r| *r = true);
        land
    }

    #[test]
    fn a_stations_track_ends_where_the_fixtures_do() {
        // The fixture's cargo station: modules -3..4, its ends at local
        // (10, -140) and (10, 180).
        let s = Station {
            origin: p(0.0, 0.0),
            heading: std::f64::consts::FRAC_PI_2,
            first: -3,
            last: 4,
        };
        let (inner, outer) = (s.inner(), s.outer());
        assert!(
            (inner.x - 10.0).abs() < 1e-9 && (inner.y - 180.0).abs() < 1e-9,
            "{inner:?}"
        );
        assert!(
            (outer.x - 10.0).abs() < 1e-9 && (outer.y + 140.0).abs() < 1e-9,
            "{outer:?}"
        );
        // Heading east: +y local is +x world, +x local is -y world.
        let e = Station { heading: 0.0, ..s };
        let inner = e.inner();
        assert!(
            (inner.x - 180.0).abs() < 1e-9 && (inner.y + 10.0).abs() < 1e-9,
            "{inner:?}"
        );
        let build = e.build(1923, 7, "Freight").unwrap();
        assert_eq!(build.transform.basis[..2], [0, -1_000_000]);
        let names: Vec<&str> = build.params.iter().map(|p| p.key.as_str()).collect();
        assert!(
            names.contains(&"modules[8401970].name") && names.contains(&"modules[8402040].name")
        );
        assert!(names.contains(&"modules[6399970].name"));
        assert!(
            build
                .params
                .iter()
                .any(|p| p.key.as_str() == "length" && p.value == ParamValue::Int(5))
        );
    }

    #[test]
    fn a_depot_joins_in_line_beyond_the_outer_end() {
        let s = Station::new(p(0.0, 0.0), 0.0);
        let (_, (end, out)) = s.ends_toward(p(1_000.0, 0.0));
        assert!(
            (end.x - s.outer().x).abs() < 1e-9,
            "the outer end is the depot's"
        );
        let (join, build) = depot(end, out, 1923, 3, "Depot").unwrap();
        let outer = s.outer();
        assert!((join.x - (outer.x - DEPOT_GAP)).abs() < 1e-9 && (join.y - outer.y).abs() < 1e-9);
        // Its own join, at DEPOT_JOIN on its axes, lands on `join`.
        let o = P::of(build.transform.origin);
        let (cos, sin) = (
            f64::from(build.transform.basis[0]) / 1e6,
            f64::from(build.transform.basis[1]) / 1e6,
        );
        let (jx, jy) = DEPOT_JOIN;
        let x = o.x + jx * cos - jy * sin;
        let y = o.y + jx * sin + jy * cos;
        assert!(
            (x - join.x).abs() < 0.01 && (y - join.y).abs() < 0.01,
            "{x} {y} {join:?}"
        );
    }

    #[test]
    fn a_route_goes_round_built_land_and_keeps_its_curves_wide() {
        let (a, b) = (p(0.0, 0.0), p(3_000.0, 0.0));
        // A town in the way, 600 m across, astride the straight line.
        let land = flat(a, b, |x, y| (x - 1_500.0).hypot(y) < 300.0);
        let cells = route(&land, a, 0.0, b, 0.0).expect("a route");
        assert!(
            cells.iter().all(|(x, y)| (x - 1_500.0).hypot(*y) >= 300.0),
            "{cells:?}"
        );
        let nodes = track(&land, a, 0.0, b, 0.0, &cells);
        assert_eq!(nodes.first().unwrap().at, a);
        assert_eq!(nodes.last().unwrap().at, b);
        assert!(tightest(&nodes) >= 150.0, "{}", tightest(&nodes));
        for w in nodes.windows(2) {
            let d = w[0].at.flat_distance(w[1].at);
            assert!((50.0..=160.0).contains(&d), "edge {d}");
        }
        assert!(nodes.iter().all(|n| !n.bridge));
    }

    #[test]
    fn a_line_climbs_within_its_grade_and_bridges_a_valley() {
        let (a, b) = (
            p(0.0, 0.0),
            P {
                x: 2_000.0,
                y: 0.0,
                z: 10.0,
            },
        );
        let mut land = flat(a, b, |_, _| false);
        // A valley 30 m deep, 400 m wide, across the line.
        for j in 0..land.h {
            for i in 0..land.w {
                let (x, _) = land.centre(i, j);
                if (x - 1_000.0).abs() < 200.0 {
                    land.z[j * land.w + i] = -20.0;
                }
            }
        }
        let cells = route(&land, a, 0.0, b, 0.0).expect("a route");
        let nodes = track(&land, a, 0.0, b, 0.0, &cells);
        for w in nodes.windows(2) {
            let grade = (w[1].at.z - w[0].at.z).abs() / w[0].at.flat_distance(w[1].at);
            assert!(grade <= MAX_GRADE + 1e-6, "grade {grade}");
        }
        assert!(nodes.iter().any(|n| n.bridge), "a bridge over the valley");
    }

    #[test]
    fn pieces_chain_from_station_to_station() {
        let (a, b) = (p(0.0, 0.0), p(2_000.0, 0.0));
        let land = flat(a, b, |_, _| false);
        let cells = route(&land, a, 0.0, b, 0.0).unwrap();
        let nodes = track(&land, a, 0.0, b, 0.0, &cells);
        let builds = pieces(&nodes, 1923);
        assert!(builds.len() >= 3);
        for (k, build) in builds.iter().enumerate() {
            let v = &build.polyline.vertices;
            assert!(
                matches!(v[0].resolve, Resolve::Node(_)),
                "piece {k} starts on a node"
            );
            assert!(build.polyline.links.len() <= PIECE_EDGES);
            if k + 1 < builds.len() {
                assert_eq!(v[v.len() - 1].resolve, Resolve::New);
                assert_eq!(v[v.len() - 1].pos, builds[k + 1].polyline.vertices[0].pos);
            } else {
                assert!(matches!(v[v.len() - 1].resolve, Resolve::Node(_)));
                assert_eq!(v[v.len() - 1].pos, b.pos());
            }
        }
        assert_eq!(builds[0].track.as_str(), track_template(1923));
    }

    #[test]
    fn sites_keep_off_built_land_and_face_the_target() {
        let (target, other) = (p(0.0, 0.0), p(2_000.0, 0.0));
        // Built land east of the target, towards the other end.
        let land = flat(target, other, |x, _| x > 100.0 && x < 600.0);
        let all = sites(&land, target, other, &[200.0]);
        assert!(!all.is_empty());
        for s in &all {
            assert!(
                s.footprint()
                    .iter()
                    .all(|(x, _)| *x <= 100.0 || *x >= 600.0)
            );
            // Its building's side, its own -x, faces the target.
            let building = s.local(-10.0, 0.0);
            assert!(building.flat_distance(target) < s.origin.flat_distance(target));
        }
        // The line leaves from the end nearer the other place.
        let ((end, _), (depot_end, _)) = all[0].ends_toward(other);
        assert!(end.flat_distance(other) <= depot_end.flat_distance(other));
    }

    #[test]
    fn no_route_where_the_land_is_closed() {
        let (a, b) = (p(0.0, 0.0), p(2_000.0, 0.0));
        // A wall from edge to edge across the line.
        let land = flat(a, b, |x, _| (x - 1_000.0).abs() < 60.0);
        assert!(route(&land, a, 0.0, b, 0.0).is_none());
    }
}
