//! Rail lines as plain geometry over what the game told the AI
//! (docs/AI_PLAYERS.md, "Track and trains"): a one-track cargo station and
//! where its track ends, a rail depot beyond a station's end, the land on a
//! grid, the cheapest route over it from one station's end to the other's
//! that a train can run, as the game prices track, earthworks, bridges and
//! tunnels ([`crate::costs`]), smoothed into track, its heights chosen for
//! the least cost within the line's grade, level crossings where it meets a
//! street, and that track cut into builds the game is asked about one at a
//! time. Every action made here is put to the game's verdict before it is
//! sent.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use tpf3mp_bridge::ai::{AiLand, AiStreet, land};
use tpf3mp_proto::{
    BoundedVec, Text,
    action::{
        ConstructionBuild, EdgeEnds, EdgeObjectKind, EdgeRef, Link, Network, Param, ParamValue,
        PlaceStop, Polyline, Pos, ResName, Resolve, Structure, Tangent, TrackBuild, Transform,
        UnitDir, Vertex,
    },
};

use crate::costs::{self, Prices};
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
/// The same for a passenger station: its 5 m platform in the first row,
/// its track in the second (`AddPassenger(0)`, `AddTrack(1)`).
pub const PASSENGER_TRACK_X: f64 = 5.0;
/// Where a rail depot's track meets the world's, on its own axes (the
/// fixture's depot: its boundary node at local (-2.2, -70)).
pub const DEPOT_JOIN: (f64, f64) = (-2.2, -70.0);
/// How far beyond a station's outer end a depot's track joins, in metres.
pub const DEPOT_GAP: f64 = 30.0;
/// Where a depot is tried beyond each of a line's stations, in order: how
/// far from the station's outer end its track joins, in metres, and how
/// far it turns from the station's line, in radians. Straight out first,
/// then turned 30 and 45 degrees either way, its track to the station one
/// edge curving from the station's heading to the depot's (about 195 m of
/// radius, over the line's least). Straight out met a player's railway at
/// one station, streets at another, and a line was given up with its
/// stations and track built (2026-10-31).
pub const DEPOT_PLACES: [(f64, f64); 7] = [
    (DEPOT_GAP, 0.0),
    (90.0, 0.0),
    (150.0, 0.0),
    (100.0, std::f64::consts::FRAC_PI_6),
    (100.0, -std::f64::consts::FRAC_PI_6),
    (150.0, std::f64::consts::FRAC_PI_4),
    (150.0, -std::f64::consts::FRAC_PI_4),
];
/// The grade a line keeps to where it can, the steepest it climbs at all
/// (a plant on a hill above its town, 89 m over 3.6 km, 2026-10-30), and
/// its tightest curve, in metres: well inside the simple track's limits
/// (8.5 % and 55 m to build), for early locomotives and a line's speed.
pub const MAX_GRADE: f64 = 0.025;
pub const STEEPEST: f64 = 0.04;
pub const MIN_RADIUS: f64 = 180.0;
/// What a metre climbed or fallen beyond [`MAX_GRADE`] costs: dear enough
/// that a line keeps to its grade unless it cannot, for a steep line slows
/// its trains, which no build price shows.
pub const STEEP: f64 = 100_000.0;
/// The land grid's cell, in metres, and a tile's cells a side: a tile is
/// one question, about 7 ms of the driving game's frame.
pub const STEP: f64 = 40.0;
/// How far a passenger station reaches people on foot, in metres from its
/// middle, about as the game's catchment overlay shows a station's.
pub const PEOPLE_REACH: f64 = 250.0;
/// The fewest town houses a passenger station must reach on foot, cells of
/// [`STEP`] built on by houses within [`PEOPLE_REACH`]: the AI's stations at
/// the towns' edges reached 4 and 5 people (the game's catchment), the
/// fixture player's 16 to 53, and every AI passenger train lost money.
pub const PEOPLE_LEAST: usize = 15;
/// How far straight out of a passenger station no street may lie, in
/// metres: a street met sooner, unless it takes a level crossing, the line
/// must clear by [`OVER`] or [`UNDER`], more than its grade climbs in that
/// space; the first stations among houses found no heights for their track
/// for the streets round them (run 53, 2026-10-31). It stands at the
/// built-up area's edge facing the line, as a player's first station, the
/// houses behind it in reach.
pub const EXIT_CLEAR: f64 = 200.0;
/// What a metre of track through town houses costs beyond the ground, the
/// houses it clears: about what a road depot among houses cost beyond one
/// on open ground (up to 800,000 for its 40 by 20 metres), over a track's
/// width of some 8 m.
pub const HOUSE_PER_M: f64 = 8_000.0;
pub const TILE: u8 = 16;
/// How far beyond the line's two ends the land is read, in metres.
pub const MARGIN: f64 = 480.0;
/// How far the line runs straight out of a station before the route
/// turns, in metres: two cells.
pub const LEAD: f64 = 2.0 * STEP;
/// How long a track edge is, about, and how many edges one build lays.
pub const EDGE: f64 = 100.0;
pub const PIECE_EDGES: usize = 6;
/// The heights a line's nodes are chosen among, apart, in metres.
pub const LEVEL: f64 = 0.5;
/// How far a line's heights may go above or below the ground it crosses,
/// in metres.
pub const REACH: f64 = 60.0;
/// What a metre climbed or fallen costs over its earth: trains are slower
/// up a grade and the line pays for it, so a level line is worth a little.
pub const CLIMB: f64 = 300.0;
/// A track over a street clears it by this, under one passes this deep, in
/// metres.
pub const OVER: f64 = 7.5;
pub const UNDER: f64 = 10.0;
/// The narrowest angle a level crossing takes (the game refuses one under
/// about 12°, docs/BUILDING.md), how near a street's ends one may be, and
/// how near another crossing or the line's ends, in metres.
pub const CROSSING_ANGLE: f64 = 35.0 * std::f64::consts::PI / 180.0;
pub const CROSSING_CLEAR: f64 = 8.0;
pub const CROSSING_APART: f64 = 30.0;
/// The steepest the track runs either side of a level crossing, and the
/// steepest street one is laid on: a crossing lies level.
pub const CROSSING_GRADE: f64 = 0.01;
pub const CROSSING_STREET_GRADE: f64 = 0.04;

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

/// The station modules' era for `year` (`getEra`).
fn era(year: i32) -> &'static str {
    if year >= 1990 {
        "c"
    } else if year >= 1950 {
        "b"
    } else {
        "a"
    }
}

/// A universal cargo platform of `year`'s era.
fn platform_module(year: i32) -> String {
    format!(
        "::/stations/rail/modular_station/platform_cargo_era_{}.module",
        era(year)
    )
}

/// A unit vector at `heading` radians.
fn unit(heading: f64) -> (f64, f64) {
    (heading.cos(), heading.sin())
}

/// A one-track station, for cargo or for passengers: its origin, the
/// heading of its track (its own +y axis), and its track modules from
/// `first` to `last` (0 its middle, each [`MODULE`] long).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Station {
    pub origin: P,
    pub heading: f64,
    pub first: i32,
    pub last: i32,
    pub passengers: bool,
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
            passengers: false,
        }
    }

    /// The same for passengers.
    pub fn for_passengers(origin: P, heading: f64) -> Self {
        Self {
            passengers: true,
            ..Self::new(origin, heading)
        }
    }

    /// How far across its own axes its track runs.
    fn track_x(&self) -> f64 {
        if self.passengers {
            PASSENGER_TRACK_X
        } else {
            STATION_TRACK_X
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
        self.local(self.track_x(), f64::from(self.last) * MODULE + MODULE / 2.0)
    }

    /// Its track's other end, where a depot joins.
    pub fn outer(&self) -> P {
        self.local(
            self.track_x(),
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
            for x in [-15.0, 0.0, self.track_x()] {
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
        ];
        // Cargo: universal platforms (specialization 1) in its first row,
        // the track in its third. Passengers: passenger platforms in its
        // first row (7400000), the track in its second.
        let (platform, platforms, tracks, building) = if self.passengers {
            (
                format!(
                    "::/stations/rail/modular_station/platform_passenger_era_{}.module",
                    era(year)
                ),
                7_400_000,
                8_401_000,
                format!(
                    "::/stations/rail/modular_station/main_building_1_era_{}.module",
                    era(year)
                ),
            )
        } else {
            params.push(int("specialization", 1));
            (
                platform_module(year),
                6_400_000,
                8_402_000,
                "::/stations/rail/modular_station/main_building_1_cargo.module".to_owned(),
            )
        };
        for j in self.first..=self.last {
            params.extend(module(platforms + 10 * i64::from(j), &platform));
            params.extend(module(tracks + 10 * i64::from(j), track_module(year)));
        }
        // A through station's building: its slot by whether its module
        // count is even.
        let k = if modules % 2 == 0 { 0 } else { 2 };
        params.extend(module(3_400_000 + 10 * k, &building));
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
    depot_at(end, out_heading, DEPOT_GAP, year, seed, name)
}

/// A rail depot at place place ([`DEPOT_PLACES`]) beyond a station's
/// end leaving at out: where its track joins, the heading its track
/// runs into the depot at (what [`depot_link`] arrives at), and the build.
/// The join lies at the end of an arc from the station's heading to the
/// depot's: along the chord, half the turn off the station's line.
pub fn depot_place(
    end: P,
    out: f64,
    place: usize,
    year: i32,
    seed: i64,
    name: &str,
) -> Option<(P, f64, ConstructionBuild)> {
    let &(gap, turn) = DEPOT_PLACES.get(place)?;
    let (cx, cy) = unit(out + turn / 2.0);
    let join = P {
        x: end.x + cx * gap,
        y: end.y + cy * gap,
        z: end.z,
    };
    let heading = out + turn;
    let (_, build) = depot_joined(join, heading, year, seed, name)?;
    Some((join, heading, build))
}

/// [`depot`], its track joining gap metres beyond the station's end.
pub fn depot_at(
    end: P,
    out_heading: f64,
    gap: f64,
    year: i32,
    seed: i64,
    name: &str,
) -> Option<(P, ConstructionBuild)> {
    let (ox, oy) = unit(out_heading);
    let join = P {
        x: end.x + ox * gap,
        y: end.y + oy * gap,
        z: end.z,
    };
    depot_joined(join, out_heading, year, seed, name)
}

/// A rail depot whose track joins at join, running into it at
/// out_heading: where it joins, and the build.
fn depot_joined(
    join: P,
    out_heading: f64,
    year: i32,
    seed: i64,
    name: &str,
) -> Option<(P, ConstructionBuild)> {
    let (ox, oy) = unit(out_heading);
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

/// Station placements around `target` for a line towards `other`, for
/// cargo or for `passengers`: on
/// rings of `radii` metres, 16 bearings round, each turned so its building
/// (on its own -x side) faces `target`, at the ground's height; those whose
/// ground the land shows built on, a street, a track or water left out.
/// The nearest ring first, and on each the bearings nearest `other` first.
/// A passenger station may stand on town houses, the game clearing them
/// for their price, as a player puts a station in town: those reaching
/// the most houses on foot ([`Land::people`]) first.
#[allow(clippy::cast_precision_loss)]
pub fn sites(land: &Land, target: P, other: P, radii: &[f64], passengers: bool) -> Vec<Station> {
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
                // A cargo station turned round its target, its building
                // facing it; a town's passenger station at its edge turned
                // to the other town, so its line runs straight out.
                let heading = if passengers {
                    (other.y - y).atan2(other.x - x)
                } else {
                    a + std::f64::consts::FRAC_PI_2
                };
                (
                    angle_between(a, towards),
                    Station {
                        passengers,
                        ..Station::new(origin, heading)
                    },
                )
            })
            .filter(|(_, s)| {
                let clear = |x: f64, y: f64, also: u8| {
                    land.flags_at(x, y).is_some_and(|f| {
                        let houses = passengers && f & land::HOUSE != 0;
                        (f & land::BUILT == 0 || houses)
                            && f & (land::TRACK | land::WATER | also) == 0
                    })
                };
                // Its ground free, and room for its line to leave it: the
                // route starts straight out, two cells beyond its lead; a
                // passenger station's, no street within [`EXIT_CLEAR`].
                let ((end, out), _) = s.ends_toward(other);
                let (ux, uy) = unit(out);
                let lead: Vec<(f64, u8)> = if passengers {
                    let cells = (EXIT_CLEAR / STEP).ceil() as u32;
                    (1..=cells)
                        .map(|k| (f64::from(k) * STEP, land::STREET))
                        .collect()
                } else {
                    vec![(LEAD, 0), (LEAD + STEP, 0), (LEAD + 2.0 * STEP, 0)]
                };
                s.footprint()
                    .iter()
                    .all(|(x, y)| clear(*x, *y, land::STREET))
                    && lead
                        .iter()
                        .all(|(d, also)| clear(end.x + ux * d, end.y + uy * d, *also))
            })
            .collect();
        ring.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.extend(ring.into_iter().map(|(_, s)| s));
    }
    if passengers {
        // Stable: the nearer ring first among those reaching as many.
        out.sort_by_cached_key(|s| std::cmp::Reverse(land.people(s)));
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

    /// The town houses a passenger station at `station` reaches on foot:
    /// cells built on by houses alone ([`land::HOUSE`]) within
    /// [`PEOPLE_REACH`] of its middle, those its own ground clears not
    /// counted.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn people(&self, station: &Station) -> usize {
        let middle = station.local(0.0, f64::from(station.first + station.last) * MODULE / 2.0);
        let under: Vec<(usize, usize)> = station
            .footprint()
            .iter()
            .filter_map(|(x, y)| self.cell(*x, *y))
            .collect();
        let span = (PEOPLE_REACH / STEP).ceil() as i64;
        let Some((ci, cj)) = self.cell(middle.x, middle.y) else {
            return 0;
        };
        let mut n = 0;
        for dj in -span..=span {
            for di in -span..=span {
                let (i, j) = (ci as i64 + di, cj as i64 + dj);
                if i < 0 || j < 0 || i >= self.w as i64 || j >= self.h as i64 {
                    continue;
                }
                let (i, j) = (i as usize, j as usize);
                let (x, y) = (self.x0 + i as f64 * STEP, self.y0 + j as f64 * STEP);
                if (x - middle.x).hypot(y - middle.y) <= PEOPLE_REACH
                    && self.flags[j * self.w + i] & land::HOUSE != 0
                    && !under.contains(&(i, j))
                {
                    n += 1;
                }
            }
        }
        n
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

/// What a step of route costs in money, or none where it may not go: over
/// built land but town houses, which it crosses at [`HOUSE_PER_M`]. Its track; a bridge over the water it crosses, and over
/// another railway, at the height that clears it; the cheapest of earth, bridge or tunnel for the height its
/// slope asks beyond the line's grade (half that rise, on average, over
/// the step); a level crossing where it meets a street, which it crosses
/// and never follows; and [`CLIMB`] for every metre the ground rises or
/// falls, so it keeps to the level where it can.
fn step_cost(land: &Land, prices: &Prices, a: (f64, f64), b: (f64, f64)) -> Option<f64> {
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    let (mut wet, mut rail, mut street, mut houses) = (0u8, 0u8, false, 0u8);
    for k in 1..=4 {
        let t = f64::from(k) / 4.0;
        let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        let f = land.flags_at(x, y)?;
        // Through town houses at what clearing them costs; never through
        // anything else built.
        if f & land::BUILT != 0 {
            if f & land::HOUSE == 0 {
                return None;
            }
            houses += 1;
        }
        street |= f & land::STREET != 0;
        wet += u8::from(f & land::WATER != 0);
        rail += u8::from(f & land::TRACK != 0);
    }
    let flat = prices.ground(0.0);
    let over_water = prices.bridge(8.0).unwrap_or(flat);
    let over_rail = prices.bridge(OVER + 2.0).unwrap_or(flat);
    let (wet, rail) = (f64::from(wet) / 4.0, f64::from(rail) / 4.0);
    let dry = (1.0 - wet - rail).max(0.0);
    let mut cost = len * (dry * flat + wet * over_water + rail * over_rail);
    cost += len * f64::from(houses) / 4.0 * HOUSE_PER_M;
    // The ramps up to clear a railway, as dear again as the bridge.
    if rail > 0.0 {
        cost += len * rail * over_rail;
    }
    let rise = (land.height(b.0, b.1) - land.height(a.0, a.1)).abs();
    let h = (rise - MAX_GRADE * len).max(0.0) / 2.0;
    if h > 0.0 {
        let fill = prices.ground(h).min(prices.ground(-h));
        let made = [prices.bridge(h), prices.tunnel(h)]
            .into_iter()
            .flatten()
            .fold(fill, f64::min);
        cost += len * (made - flat);
    }
    if street {
        cost += prices.crossing();
    }
    Some(cost + CLIMB * prices.scale * rise)
}

/// The most states a route search opens before it gives up: about 0.2 s
/// of the driver's agent (2026-10-30, a 3.6 km line routes in 5 to 25 ms;
/// one with no way through searches to this).
pub const SEARCH_MOST: usize = 200_000;

/// The cheapest route over `land` from `from`, leaving at `from_heading`,
/// to `to`, arriving at `to_heading`, at `prices` ([`step_cost`]): the
/// cells' centres it passes, from the first clear cell [`LEAD`] out of
/// `from` to the one [`LEAD`] short of `to`. It turns 22.5° at most a
/// step, never on two steps running, each turn costing a little, and keeps
/// off built land and other tracks.
#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
pub fn route(
    land: &Land,
    prices: &Prices,
    from: P,
    from_heading: f64,
    to: P,
    to_heading: f64,
) -> Option<Vec<(f64, f64)>> {
    let flat = prices.ground(0.0);
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
    // No metre costs less than flat track: the estimate never overshoots.
    let h = |i: usize, j: usize| {
        let (x, y) = land.centre(i, j);
        flat * (x - goal_x).hypot(y - goal_y)
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
            let Some(step) = step_cost(land, prices, here, there) else {
                continue;
            };
            let len = (there.0 - here.0).hypot(there.1 - here.1);
            let turn_cost = if turn == 0 { 0.0 } else { 0.25 * len * flat };
            let ng = g + step + turn_cost;
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

/// What a track edge is built as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ground,
    Bridge,
    Tunnel,
}

/// A track node of a line: where it is, the heading of the track through
/// it, what the edge from it to the next is built as and what that edge
/// costs, and, at a level crossing, the street edge it splits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub at: P,
    pub heading: f64,
    pub kind: Kind,
    pub cost: f64,
    pub crossing: Option<EdgeEnds>,
}

impl Node {
    /// A node on the ground, nothing yet built from it.
    pub fn on(at: P, heading: f64) -> Self {
        Self {
            at,
            heading,
            kind: Kind::Ground,
            cost: 0.0,
            crossing: None,
        }
    }
}

/// The line's track from `from` (leaving at `from_heading`) to `to`
/// (arriving at `to_heading`) through the route's cells, its heights the
/// cheapest within its grade at `prices` ([`profile`]); none if no heights
/// meet the grade. Both ends keep their stations' heights.
pub fn track(
    land: &Land,
    prices: &Prices,
    from: P,
    from_heading: f64,
    to: P,
    to_heading: f64,
    cells: &[(f64, f64)],
) -> Option<Vec<Node>> {
    let nodes = align(land, from, from_heading, to, to_heading, cells);
    let holds = ends_held(&nodes);
    profile(land, prices, &nodes, &holds)
}

/// The holds of a line with no crossing: its two ends at their heights.
pub fn ends_held(nodes: &[Node]) -> Vec<Hold> {
    let n = nodes.len();
    (0..n)
        .map(|k| {
            if k == 0 || k + 1 == n {
                Hold::At(nodes[k].at.z)
            } else {
                Hold::Free
            }
        })
        .collect()
}

/// The line's nodes from `from` (leaving at `from_heading`) to `to`
/// (arriving at `to_heading`) through the route's cells, on the ground:
/// about [`EDGE`] apart, smoothed so no curve is tighter than
/// [`MIN_RADIUS`] where the route allows. Both ends keep their stations'
/// heights.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn align(
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
    // On the ground, the ends at their stations' heights.
    (0..n)
        .map(|k| {
            let z = if k == 0 {
                from.z
            } else if k == n - 1 {
                to.z
            } else {
                land.height(points[k].0, points[k].1)
            };
            Node::on(
                P {
                    x: points[k].0,
                    y: points[k].1,
                    z,
                },
                headings[k],
            )
        })
        .collect()
}

/// How a node's height is held while a line's heights are chosen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hold {
    /// Wherever is cheapest.
    Free,
    /// At this height: a station's end. Its edges are on the ground or a
    /// bridge, never a tunnel.
    At(f64),
    /// At this height, a node already laid: its edge may be anything.
    Laid(f64),
    /// Where a street at this height crosses: at its height, a level
    /// crossing whose edges are on the ground; [`OVER`] above it on a
    /// bridge; or [`UNDER`] below it in a tunnel.
    Street(f64),
    /// Where a street at this height is met that no level crossing may be
    /// laid on: over it or under it, as for [`Hold::Street`].
    Clear(f64),
}

/// One of the points an edge is priced at: how far along it, the ground's
/// height there, and whether water or another railway lies there.
#[derive(Debug, Clone, Copy, Default)]
struct Sample {
    t: f64,
    ground: f64,
    wet: bool,
    rail: bool,
}

/// An edge's samples, at its eighths.
fn samples(land: &Land, a: P, b: P) -> [Sample; 4] {
    let mut out = [Sample::default(); 4];
    for (k, s) in out.iter_mut().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let t = (2 * k + 1) as f64 / 8.0;
        let (x, y) = (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        let f = land.flags_at(x, y).unwrap_or(0);
        *s = Sample {
            t,
            ground: land.height(x, y),
            wet: f & land::WATER != 0,
            rail: f & land::TRACK != 0,
        };
    }
    out
}

/// What an edge `len` long costs from height `z0` to `z1` over `ground`,
/// built as `kind`, if it can be: on the ground nowhere over water; a
/// bridge only [`costs::BRIDGE_LEAST`] above the ground, or over water; a
/// tunnel only [`costs::TUNNEL_LEAST`] under it, nowhere under water. Over
/// another railway only a bridge [`OVER`] above it or a tunnel [`UNDER`]
/// below it, the railway taken at the ground's height. What it
/// is weighed at, its climb and any grade beyond [`MAX_GRADE`] counted, and
/// what the game charges for it.
fn edge_cost(
    prices: &Prices,
    ground: &[Sample; 4],
    len: f64,
    z0: f64,
    z1: f64,
    kind: Kind,
) -> Option<(f64, f64)> {
    let mut sum = 0.0;
    for &Sample {
        t,
        ground: g,
        wet,
        rail,
    } in ground
    {
        let h = z0 + (z1 - z0) * t - g;
        let metre = match kind {
            Kind::Ground if wet || rail => None,
            Kind::Bridge if rail && h < OVER => None,
            Kind::Tunnel if rail && -h < UNDER => None,
            Kind::Ground => Some(prices.ground(h)),
            // Over water the ground is the bed: the bridge's height there
            // is priced as a middling one.
            Kind::Bridge if wet => (h >= 2.0)
                .then(|| prices.bridge(h.clamp(costs::BRIDGE_LEAST, 12.0)))
                .flatten(),
            Kind::Bridge => prices.bridge(h),
            Kind::Tunnel if wet => None,
            Kind::Tunnel => prices.tunnel(-h),
        }?;
        sum += metre * len / 4.0;
    }
    let rise = (z1 - z0).abs();
    let steep = (rise - MAX_GRADE * len).max(0.0);
    Some((sum + prices.scale * (CLIMB * rise + STEEP * steep), sum))
}

/// The kinds an edge from a node at `z` may be built as, by its hold.
fn kinds(hold: Hold, z: f64) -> &'static [Kind] {
    match hold {
        Hold::Free | Hold::Laid(_) => &[Kind::Ground, Kind::Bridge, Kind::Tunnel],
        // Out of a station on the ground, or over water by a bridge.
        Hold::At(_) => &[Kind::Ground, Kind::Bridge],
        Hold::Street(s) if (z - s).abs() < LEVEL => &[Kind::Ground],
        Hold::Street(s) | Hold::Clear(s) if z > s => &[Kind::Bridge],
        Hold::Street(_) | Hold::Clear(_) => &[Kind::Tunnel],
    }
}

/// The line's heights and what each edge is built as, for the least cost
/// at `prices`, keeping to [`MAX_GRADE`] where it can ([`STEEP`] a metre
/// beyond it) and never steeper than [`STEEPEST`]: every node's
/// height chosen among levels [`LEVEL`] apart, as deep or high as
/// [`REACH`] off the ground, by a search over the whole line, so a valley
/// is bridged and a hill tunnelled where that is cheaper than the earth
/// moved for an embankment or a cutting. Held nodes keep their heights
/// ([`Hold`]); a crossing taken over or under its street is a crossing no
/// more. None if no heights meet the grade.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn profile(land: &Land, prices: &Prices, nodes: &[Node], holds: &[Hold]) -> Option<Vec<Node>> {
    let n = nodes.len();
    if n < 2 || holds.len() != n {
        return None;
    }
    let grounds: Vec<[Sample; 4]> = nodes
        .windows(2)
        .map(|w| samples(land, w[0].at, w[1].at))
        .collect();
    let lengths: Vec<f64> = nodes
        .windows(2)
        .map(|w| w[0].at.flat_distance(w[1].at))
        .collect();
    // The levels: from the lowest ground (or hold) less REACH to the
    // highest more.
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for (k, node) in nodes.iter().enumerate() {
        let g = land.height(node.at.x, node.at.y);
        let held = match holds[k] {
            Hold::Free => g,
            Hold::At(z) | Hold::Laid(z) | Hold::Street(z) | Hold::Clear(z) => z,
        };
        lo = lo.min(g.min(held));
        hi = hi.max(g.max(held));
    }
    let base = lo - REACH;
    let count = (((hi + REACH) - base) / LEVEL).ceil() as usize + 1;
    let level = |i: usize| base + i as f64 * LEVEL;
    let nearest = |z: f64| ((z - base) / LEVEL).round().max(0.0) as usize;
    let allowed = |k: usize, i: usize| match holds[k] {
        Hold::Free => true,
        Hold::At(z) | Hold::Laid(z) => i == nearest(z),
        Hold::Street(s) => i == nearest(s) || level(i) >= s + OVER || level(i) <= s - UNDER,
        Hold::Clear(s) => level(i) >= s + OVER || level(i) <= s - UNDER,
    };
    // Held heights exactly, not their nearest level.
    let height = |k: usize, i: usize| match holds[k] {
        Hold::At(z) | Hold::Laid(z) => z,
        Hold::Street(s) if i == nearest(s) => s,
        _ => level(i),
    };
    let at_crossing =
        |k: usize, z: f64| matches!(holds[k], Hold::Street(s) if (z - s).abs() < LEVEL);
    let mut cost = vec![vec![f64::INFINITY; count]; n];
    let mut back = vec![vec![(usize::MAX, Kind::Ground, 0.0); count]; n];
    // Each back step: the node before, the edge's kind and its price.
    for (i, c) in cost[0].iter_mut().enumerate() {
        if allowed(0, i) {
            *c = 0.0;
        }
    }
    for k in 1..n {
        let len = lengths[k - 1];
        let reach = (STEEPEST * len / LEVEL).ceil() as usize;
        for i in 0..count {
            if !allowed(k, i) {
                continue;
            }
            let z1 = height(k, i);
            let into = kinds(holds[k], z1);
            for j in i.saturating_sub(reach)..=(i + reach).min(count - 1) {
                let before = cost[k - 1][j];
                if !before.is_finite() {
                    continue;
                }
                let z0 = height(k - 1, j);
                // Held heights lie off the levels by up to half of one.
                if (z1 - z0).abs() > STEEPEST * len + LEVEL / 2.0 {
                    continue;
                }
                // A level crossing lies level: the game refuses one on a
                // grade ("Too Much Incline", 2026-10-30).
                if (at_crossing(k - 1, z0) || at_crossing(k, z1))
                    && (z1 - z0).abs() > CROSSING_GRADE * len + LEVEL / 2.0
                {
                    continue;
                }
                for &kind in kinds(holds[k - 1], z0) {
                    if !into.contains(&kind) {
                        continue;
                    }
                    let Some((e, price)) = edge_cost(prices, &grounds[k - 1], len, z0, z1, kind)
                    else {
                        continue;
                    };
                    if before + e < cost[k][i] {
                        cost[k][i] = before + e;
                        back[k][i] = (j, kind, price);
                    }
                }
            }
        }
    }
    let (mut i, _) = cost[n - 1]
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is_finite())
        .min_by(|a, b| a.1.total_cmp(b.1))?;
    let mut out = nodes.to_vec();
    out[n - 1].at.z = height(n - 1, i);
    out[n - 1].kind = Kind::Ground;
    out[n - 1].cost = 0.0;
    for k in (1..n).rev() {
        let (j, kind, price) = back[k][i];
        out[k - 1].at.z = height(k - 1, j);
        out[k - 1].kind = kind;
        out[k - 1].cost = price;
        i = j;
    }
    // A crossing not taken at its street's height is no crossing; one
    // taken is priced with its edge.
    for k in 0..n {
        if let Hold::Street(s) = holds[k] {
            if (out[k].at.z - s).abs() >= LEVEL {
                out[k].crossing = None;
            } else if out[k].crossing.is_some() {
                out[k].cost += prices.crossing();
            }
        }
    }
    Some(out)
}

/// What a line's track costs, as planned.
pub fn cost(nodes: &[Node]) -> f64 {
    nodes.iter().map(|n| n.cost).sum()
}

/// Track in words, for the AI's log: each edge's kind, length and grade,
/// and where it crosses a street.
pub fn describe(nodes: &[Node]) -> String {
    nodes
        .windows(2)
        .map(|w| {
            let d = w[0].at.flat_distance(w[1].at).max(1e-6);
            let grade = 100.0 * (w[1].at.z - w[0].at.z) / d;
            let kind = match w[0].kind {
                Kind::Ground => "ground",
                Kind::Bridge => "bridge",
                Kind::Tunnel => "tunnel",
            };
            let crossing = if w[0].crossing.is_some() {
                "crossing, "
            } else {
                ""
            };
            format!("{crossing}{kind} {d:.0} m at {grade:+.1}%")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A point on a street edge's curve, `u` from its end `a` (0) to `b` (1):
/// the quadratic through its ends and its middle, which the game's own
/// curve keeps within a metre or two of.
fn street_at(a: P, mid: P, b: P, u: f64) -> P {
    let c = P {
        x: 2.0 * mid.x - (a.x + b.x) / 2.0,
        y: 2.0 * mid.y - (a.y + b.y) / 2.0,
        z: 2.0 * mid.z - (a.z + b.z) / 2.0,
    };
    let (p, q, r) = ((1.0 - u) * (1.0 - u), 2.0 * u * (1.0 - u), u * u);
    P {
        x: p * a.x + q * c.x + r * b.x,
        y: p * a.y + q * c.y + r * b.y,
        z: p * a.z + q * c.z + r * b.z,
    }
}

/// Where two segments in plan cross: how far along each, if they do.
fn segments_cross(a0: P, a1: P, b0: P, b1: P) -> Option<(f64, f64)> {
    let (rx, ry) = (a1.x - a0.x, a1.y - a0.y);
    let (sx, sy) = (b1.x - b0.x, b1.y - b0.y);
    let d = rx * sy - ry * sx;
    if d.abs() < 1e-9 {
        return None;
    }
    let (qx, qy) = (b0.x - a0.x, b0.y - a0.y);
    let t = (qx * sy - qy * sx) / d;
    let u = (qx * ry - qy * rx) / d;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((t, u))
}

/// A line's nodes with its crossings, and how each node's height is held.
pub type Crossed = (Vec<Node>, Vec<Hold>);

/// The pieces a street's curve is cut into when crossings are sought.
const STREET_PIECES: u32 = 16;

/// How near a node of the line a street may be met before the meeting
/// takes that node's place, in metres: a shorter edge would climb steeply
/// for its length (a 1 m edge at 15 %, refused "Too Much Incline",
/// 2026-10-30).
pub const MERGE: f64 = 20.0;

/// The line's nodes with a node where it meets each street, and how each
/// node's height is held ([`Hold`]): its ends at their heights; a level
/// crossing at its street's height, splitting the street's edge as the
/// track tool lays one (the track straight through it), where one may be
/// laid; else over or under the street. No level crossing at too narrow an
/// angle ([`CROSSING_ANGLE`]), on a steep street, by its edge's ends,
/// another crossing or a station, on a street with stops on it, a
/// company's, or by a junction whose settings a company's edge names (the
/// room refuses a split of those, D21). A street on a bridge or in a tunnel
/// is not met. A meeting within [`MERGE`] of a node takes its place.
pub fn cross(nodes: &[Node], streets: &[AiStreet]) -> Crossed {
    // Every meeting: its edge, how far along it, where, and the street's
    // edge where a level crossing may be laid.
    let mut found: Vec<(usize, f64, P, Option<EdgeEnds>)> = Vec::new();
    let mut seen: Vec<(EdgeEnds, P)> = Vec::new();
    for (k, w) in nodes.windows(2).enumerate() {
        let (a0, a1) = (w[0].at, w[1].at);
        let heading = (a1.y - a0.y).atan2(a1.x - a0.x);
        for street in streets {
            if street.structure {
                continue;
            }
            let (sa, sm, sb) = (P::of(street.a), P::of(street.mid), P::of(street.b));
            for piece in 0..STREET_PIECES {
                let (u0, u1) = (
                    f64::from(piece) / f64::from(STREET_PIECES),
                    f64::from(piece + 1) / f64::from(STREET_PIECES),
                );
                let (b0, b1) = (street_at(sa, sm, sb, u0), street_at(sa, sm, sb, u1));
                let Some((t, s)) = segments_cross(a0, a1, b0, b1) else {
                    continue;
                };
                let at = street_at(sa, sm, sb, u0 + (u1 - u0) * s);
                // Where two of its pieces meet, or a street the game named
                // twice: one meeting.
                let ends = EdgeEnds {
                    a: street.a,
                    b: street.b,
                };
                if seen
                    .iter()
                    .any(|(e, p)| *e == ends && p.flat_distance(at) < 2.0)
                {
                    continue;
                }
                seen.push((ends, at));
                let along = (b1.y - b0.y).atan2(b1.x - b0.x);
                let angle = angle_between(heading, along)
                    .min(angle_between(heading, along + std::f64::consts::PI));
                let near_end =
                    at.flat_distance(sa) < CROSSING_CLEAR || at.flat_distance(sb) < CROSSING_CLEAR;
                let steep = (b1.z - b0.z).abs() > CROSSING_STREET_GRADE * b0.flat_distance(b1);
                let level = angle >= CROSSING_ANGLE
                    && !steep
                    && !near_end
                    && street.objects == 0
                    && !street.owned
                    && !street.company_junction;
                found.push((k, t, at, level.then_some(ends)));
            }
        }
    }
    found.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)));
    // Level crossings apart, and away from the stations.
    let (first, last) = (nodes[0].at, nodes[nodes.len() - 1].at);
    for i in 0..found.len() {
        let at = found[i].2;
        let crowded = found
            .iter()
            .enumerate()
            .any(|(j, g)| j != i && g.2.flat_distance(at) < CROSSING_APART);
        if crowded
            || at.flat_distance(first) < CROSSING_APART
            || at.flat_distance(last) < CROSSING_APART
        {
            found[i].3 = None;
        }
    }
    let mut out = nodes.to_vec();
    let mut holds = ends_held(nodes);
    // From the last back, so the nodes before each stay where they are.
    for (k, _, at, ends) in found.into_iter().rev() {
        let (a, b) = (out[k].at, out[k + 1].at);
        let node = Node {
            crossing: ends,
            ..Node::on(at, (b.y - a.y).atan2(b.x - a.x))
        };
        let hold = match ends {
            Some(_) => Hold::Street(at.z),
            None => Hold::Clear(at.z),
        };
        let last = out.len() - 1;
        if a.flat_distance(at) < MERGE && k > 0 && holds[k] == Hold::Free {
            out[k] = node;
            holds[k] = hold;
        } else if b.flat_distance(at) < MERGE && k + 1 < last && holds[k + 1] == Hold::Free {
            out[k + 1] = node;
            holds[k + 1] = hold;
        } else {
            out.insert(k + 1, node);
            holds.insert(k + 1, hold);
        }
    }
    (out, holds)
}
/// Where the route crosses streets, bunched: a point and a radius around
/// each run of street cells it passes, for the game to name the streets
/// there.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn street_runs(land: &Land, cells: &[(f64, f64)]) -> Vec<(P, u32)> {
    let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut last: Option<(f64, f64)> = None;
    for w in cells.windows(2) {
        let (a, b) = (w[0], w[1]);
        for k in 0..=4 {
            let t = f64::from(k) / 4.0;
            let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            if land
                .flags_at(p.0, p.1)
                .is_some_and(|f| f & land::STREET != 0)
            {
                let joined = last.is_some_and(|q| (q.0 - p.0).hypot(q.1 - p.1) <= 3.0 * STEP);
                match runs.last_mut() {
                    Some(run) if joined => run.push(p),
                    _ => runs.push(vec![p]),
                }
                last = Some(p);
            }
        }
    }
    runs.into_iter()
        .map(|run| {
            let (mut x, mut y) = (0.0, 0.0);
            #[allow(clippy::cast_precision_loss)]
            let n = run.len() as f64;
            for p in &run {
                x += p.0 / n;
                y += p.1 / n;
            }
            let r = run
                .iter()
                .map(|p| (p.0 - x).hypot(p.1 - y))
                .fold(0.0, f64::max);
            (P { x, y, z: 0.0 }, (r + 2.0 * STEP).ceil() as u32)
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

/// One build of a line's track: the nodes it lays, from `first` to `last`,
/// and what it should cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub build: TrackBuild,
    pub first: usize,
    pub last: usize,
    pub cost: f64,
}

/// The track builds for `nodes`, about [`PIECE_EDGES`] edges each: every
/// piece starts on the track node the one before it laid (the first on the
/// station's end), the last ends on the other station's, and none ends on
/// a level crossing, which splits its street (a crossing's vertex
/// [`Resolve::Split`]s the street edge). Each edge's tangents follow the
/// track's heading, their length the edge's; a bridge is `prices`'s bridge.
pub fn pieces(nodes: &[Node], year: i32, prices: &Prices) -> Vec<Piece> {
    let mut out = Vec::new();
    let last = nodes.len().saturating_sub(1);
    let mut k = 0;
    while k < last {
        let mut end = (k + PIECE_EDGES).min(last);
        while end < last && nodes[end].crossing.is_some() {
            end += 1;
        }
        let vertices: Vec<Vertex> = (k..=end)
            .map(|i| Vertex {
                pos: nodes[i].at.pos(),
                resolve: match nodes[i].crossing {
                    _ if i == k || i == last => Resolve::Node(Network::Track),
                    Some(ends) => Resolve::Split(EdgeRef {
                        network: Network::Street,
                        ends,
                    }),
                    None => Resolve::New,
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
                    structure: match a.kind {
                        Kind::Ground => Structure::Ground,
                        Kind::Bridge => Structure::Bridge(res(prices.bridge_type())),
                        Kind::Tunnel => Structure::Tunnel(res(costs::TUNNEL)),
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
            out.push(Piece {
                build: TrackBuild {
                    track: res(track_template(year)),
                    style: None,
                    catenary: false,
                    polyline,
                    crossing_type: None,
                },
                first: k,
                last: end,
                cost: nodes[k..end].iter().map(|n| n.cost).sum(),
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

/// How far a passing loop's track runs beside the line's, centre to
/// centre, in metres: the standard track's spacing (`trackDistance`).
pub const LOOP_OFFSET: f64 = 5.0;
/// The edges of line a passing loop runs beside: about 300 m, room for a
/// train of six wagons to wait clear of the other's.
pub const LOOP_EDGES: usize = 3;
/// The most a passing loop's stretch of line turns, node to node.
pub const LOOP_TURN: f64 = 3.0 * std::f64::consts::PI / 180.0;
/// The shortest line given a passing loop, in metres.
pub const LOOP_FROM: f64 = 2_000.0;
/// How far along its edge from a passing loop's switch each of its
/// signals stands: where the loop's track has drawn clear of the line's.
/// Nearer, the game refuses one ("Construction Not Possible" within 30 %
/// of the switch, 2026-10-30, run 35); so a train waits 75 m short of the
/// switch, inside the loop.
pub const SIGNAL_AT: f64 = 0.75;

/// The signal a line's year offers (`signal_path_a.con.lua` until 1950,
/// then `signal_path_c`).
pub fn signal_model(year: i32) -> &'static str {
    if year >= 1950 {
        "infrastructure/signal/signal_path_c.con"
    } else {
        "infrastructure/signal/signal_path_a.con"
    }
}

/// A node's place `side` of the line (+1 left of its heading, -1 right),
/// [`LOOP_OFFSET`] off it.
fn beside(node: &Node, side: f64) -> P {
    let (ux, uy) = unit(node.heading);
    P {
        x: node.at.x - uy * side * LOOP_OFFSET,
        y: node.at.y + ux * side * LOOP_OFFSET,
        z: node.at.z,
    }
}

/// How long a stretch of line each passing loop serves, in metres: a line
/// twice as long gets two, and room for three trains.
pub const LOOP_SPACING: f64 = 2_500.0;

/// The side a passing loop beside the line's nodes from `k` may take, if
/// it fits there: [`LOOP_EDGES`] edges on the ground, straight (each node
/// turned [`LOOP_TURN`] at most), crossing no street, clear of the
/// stations, with free land on that side of them.
fn loop_fits(land: &Land, nodes: &[Node], k: usize) -> Option<f64> {
    if k == 0 || k + LOOP_EDGES + 1 >= nodes.len() {
        return None;
    }
    let span = &nodes[k..=k + LOOP_EDGES];
    let ground = span[..LOOP_EDGES].iter().all(|n| n.kind == Kind::Ground)
        && span.iter().all(|n| n.crossing.is_none());
    let straight = span
        .windows(2)
        .all(|w| angle_between(w[0].heading, w[1].heading) <= LOOP_TURN);
    if !ground || !straight {
        return None;
    }
    [1.0, -1.0].into_iter().find(|side| {
        span.iter().all(|n| {
            let p = beside(n, *side);
            land.flags_at(p.x, p.y)
                .is_some_and(|f| f & (land::BUILT | land::STREET | land::TRACK | land::WATER) == 0)
        })
    })
}

/// Where a line's passing loops go: none on a line shorter than
/// [`LOOP_FROM`], else one for each [`LOOP_SPACING`] of it (one at least),
/// each as near its even share of the line as one fits ([`loop_fits`]), no
/// two touching. Each loop's first node and side, along the line.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn loop_sites(land: &Land, nodes: &[Node]) -> Vec<(usize, f64)> {
    let mut along = vec![0.0];
    for w in nodes.windows(2) {
        along.push(along.last().copied().unwrap_or(0.0) + w[0].at.flat_distance(w[1].at));
    }
    let length = along.last().copied().unwrap_or(0.0);
    if nodes.len() < LOOP_EDGES + 3 || length < LOOP_FROM {
        return Vec::new();
    }
    let count = ((length / LOOP_SPACING).floor() as usize).max(1);
    let mut out: Vec<(usize, f64)> = Vec::new();
    for i in 1..=count {
        let target = length * i as f64 / (count + 1) as f64;
        let middle = |k: usize| (along[k] + along[k + LOOP_EDGES]) / 2.0;
        let mut firsts: Vec<usize> = (1..nodes.len().saturating_sub(LOOP_EDGES + 1)).collect();
        firsts.sort_by(|a, b| {
            (middle(*a) - target)
                .abs()
                .total_cmp(&(middle(*b) - target).abs())
        });
        let site = firsts.into_iter().find_map(|k| {
            // Apart from the loops chosen: a node of single track between.
            let apart = out
                .iter()
                .all(|(o, _)| k > o + LOOP_EDGES + 1 || k + LOOP_EDGES + 1 < *o);
            if !apart {
                return None;
            }
            loop_fits(land, nodes, k).map(|side| (k, side))
        });
        out.extend(site);
    }
    out.sort_by_key(|(k, _)| *k);
    out
}
/// The passing loop's track beside the line's nodes from `k`: from node
/// `k`, where it leaves the line as a switch, `side` of it, back onto it at
/// node `k` + [`LOOP_EDGES`]; each edge's tangents along the line's heading,
/// its inner nodes at the line's heights.
pub fn passing_loop(nodes: &[Node], k: usize, side: f64, year: i32) -> Option<TrackBuild> {
    let m = k + LOOP_EDGES;
    let at = |i: usize| {
        if i == k || i == m {
            nodes[i].at
        } else {
            beside(&nodes[i], side)
        }
    };
    let vertices: Vec<Vertex> = (k..=m)
        .map(|i| Vertex {
            pos: at(i).pos(),
            resolve: if i == k || i == m {
                Resolve::Node(Network::Track)
            } else {
                Resolve::New
            },
        })
        .collect();
    let links: Vec<Link> = (k..m)
        .map(|i| {
            let (a, b) = (at(i), at(i + 1));
            let len = a.flat_distance(b);
            let tangent = |heading: f64| {
                let (ux, uy) = unit(heading);
                mm_tangent(ux * len, uy * len, b.z - a.z)
            };
            Link {
                from: u16::try_from(i - k).unwrap_or(0),
                to: u16::try_from(i + 1 - k).unwrap_or(0),
                tangent0: tangent(nodes[i].heading),
                tangent1: tangent(nodes[i + 1].heading),
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
            }
        })
        .collect();
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

/// The engine's side flag that makes a one-way signal pass trains running
/// the way its `direction` points: the right-hand side, as a signal stands
/// beside the trains it controls on a right-hand railway (as on TPF2).
pub const FORWARD_LEFT: bool = false;

/// A point `t` along the cubic Hermite curve from `a` (tangent `ta`) to
/// `b` (tangent `tb`), and its unit direction there, as the game draws an
/// edge.
fn hermite(a: P, ta: (f64, f64, f64), b: P, tb: (f64, f64, f64), t: f64) -> (P, (f64, f64, f64)) {
    let (t2, t3) = (t * t, t * t * t);
    let (h00, h10, h01, h11) = (
        2.0 * t3 - 3.0 * t2 + 1.0,
        t3 - 2.0 * t2 + t,
        -2.0 * t3 + 3.0 * t2,
        t3 - t2,
    );
    let at = P {
        x: h00 * a.x + h10 * ta.0 + h01 * b.x + h11 * tb.0,
        y: h00 * a.y + h10 * ta.1 + h01 * b.y + h11 * tb.1,
        z: h00 * a.z + h10 * ta.2 + h01 * b.z + h11 * tb.2,
    };
    let (d00, d10, d01, d11) = (
        6.0 * t2 - 6.0 * t,
        3.0 * t2 - 4.0 * t + 1.0,
        -6.0 * t2 + 6.0 * t,
        3.0 * t2 - 2.0 * t,
    );
    let d = (
        d00 * a.x + d10 * ta.0 + d01 * b.x + d11 * tb.0,
        d00 * a.y + d10 * ta.1 + d01 * b.y + d11 * tb.1,
        d00 * a.z + d10 * ta.2 + d01 * b.z + d11 * tb.2,
    );
    let n = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt().max(1e-9);
    (at, (d.0 / n, d.1 / n, d.2 / n))
}

/// A one-way signal on the track edge from `a` (heading `ha`) to `b`
/// (heading `hb`), `t` along it, passing trains towards `b` (`forward`) or
/// towards `a`: the signal tool's placement, its place on the edge's curve
/// as the edge was laid (its tangents along the headings, as long as the
/// edge).
#[allow(clippy::cast_possible_truncation)]
fn one_way_signal(a: P, ha: f64, b: P, hb: f64, t: f64, forward: bool, year: i32) -> PlaceStop {
    let len = a.flat_distance(b);
    let tangent = |h: f64| {
        let (ux, uy) = unit(h);
        (ux * len, uy * len, b.z - a.z)
    };
    let (at, d) = hermite(a, tangent(ha), b, tangent(hb), t);
    let micro = |v: f64| (v * 1_000_000.0).round() as i32;
    PlaceStop {
        edge: EdgeRef {
            network: Network::Track,
            ends: EdgeEnds {
                a: a.pos(),
                b: b.pos(),
            },
        },
        at: at.pos(),
        left: if forward { FORWARD_LEFT } else { !FORWARD_LEFT },
        direction: UnitDir {
            x: micro(d.0),
            y: micro(d.1),
            z: micro(d.2),
        },
        model: res(signal_model(year)),
        two_sided: false,
        object: EdgeObjectKind::Signal,
        one_way: true,
        name: None,
        params: BoundedVec::default(),
    }
}

/// The two one-way signals of a passing loop beside nodes from `k`: on the
/// line's own track by the loop's far end, passing trains towards the
/// line's far end; on the loop's track by its near end, passing trains
/// back. So each direction keeps to its own track through the loop and
/// waits at its exit, clear of the other, for the single track ahead.
pub fn loop_signals(nodes: &[Node], k: usize, side: f64, year: i32) -> [PlaceStop; 2] {
    let m = k + LOOP_EDGES;
    let main = one_way_signal(
        nodes[m - 1].at,
        nodes[m - 1].heading,
        nodes[m].at,
        nodes[m].heading,
        1.0 - SIGNAL_AT,
        true,
        year,
    );
    let back = one_way_signal(
        nodes[k].at,
        nodes[k].heading,
        beside(&nodes[k + 1], side),
        nodes[k + 1].heading,
        SIGNAL_AT,
        false,
        year,
    );
    [main, back]
}
/// How near a new station's free end a rail depot of its own must join
/// the track for the new line to share it, in metres.
pub const DEPOT_SHARE: f64 = 400.0;

/// The track from a station's free `end`, leaving at `out`, to the join
/// of a depot another of its lines built, arriving at `into` (towards the
/// depot): one edge whose tangents follow both, so a new line shares the
/// depot. Both its ends are nodes laid already.
pub fn depot_link(end: P, out: f64, join: P, into: f64, year: i32) -> Option<TrackBuild> {
    let len = end.flat_distance(join);
    let tangent = |h: f64| {
        let (ux, uy) = unit(h);
        mm_tangent(ux * len, uy * len, join.z - end.z)
    };
    let vertices = vec![
        Vertex {
            pos: end.pos(),
            resolve: Resolve::Node(Network::Track),
        },
        Vertex {
            pos: join.pos(),
            resolve: Resolve::Node(Network::Track),
        },
    ];
    let links = vec![Link {
        from: 0,
        to: 1,
        tangent0: tangent(out),
        tangent1: tangent(into),
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

    #[test]
    fn a_depot_turned_aside_joins_on_the_arc_and_runs_in_at_its_turn() {
        let end = P {
            x: 100.0,
            y: 0.0,
            z: 10.0,
        };
        // Straight out first: its join 30 m beyond the end, in line.
        let (join, into, _) = depot_place(end, 0.0, 0, 1925, 1, "D").unwrap();
        assert!((join.x - 130.0).abs() < 1e-9 && join.y.abs() < 1e-9);
        assert!(into.abs() < 1e-12);
        // Turned 30 degrees: its join 100 m along the chord, 15 degrees off
        // the line; its track runs in at 30.
        let (join, into, build) = depot_place(end, 0.0, 3, 1925, 1, "D").unwrap();
        let half = std::f64::consts::FRAC_PI_6 / 2.0;
        assert!((join.x - (100.0 + 100.0 * half.cos())).abs() < 1e-6);
        assert!((join.y - 100.0 * half.sin()).abs() < 1e-6);
        assert!((into - std::f64::consts::FRAC_PI_6).abs() < 1e-12);
        // The arc's radius over the line's least.
        assert!(100.0 / (2.0 * half.sin()) > MIN_RADIUS);
        // The depot faces along its turn: its own +y axis (-sin, cos) of
        // its basis is the heading it runs in at.
        let [cos, sin, ..] = build.transform.basis;
        let (ux, uy) = (-f64::from(sin) / 1e6, f64::from(cos) / 1e6);
        assert!((ux - into.cos()).abs() < 1e-5 && (uy - into.sin()).abs() < 1e-5);
        assert!(depot_place(end, 0.0, DEPOT_PLACES.len(), 1925, 1, "D").is_none());
        // Its track to the station: one edge from the end's heading to the
        // depot's.
        let link = depot_link(end, 0.0, join, into, 1925).unwrap();
        assert_eq!(link.polyline.links.len(), 1);
    }
    fn p(x: f64, y: f64) -> P {
        P { x, y, z: 10.0 }
    }

    /// Flat land at 10 m with town houses wherever `houses` says, and
    /// another construction wherever `other` says.
    fn town(
        a: P,
        b: P,
        houses: impl Fn(f64, f64) -> bool,
        other: impl Fn(f64, f64) -> bool,
    ) -> Land {
        let mut land = flat(a, b, |_, _| false);
        for j in 0..land.h {
            for i in 0..land.w {
                let (x, y) = land.centre(i, j);
                if other(x, y) {
                    land.flags[j * land.w + i] = land::BUILT;
                } else if houses(x, y) {
                    land.flags[j * land.w + i] = land::BUILT | land::HOUSE;
                }
            }
        }
        land
    }

    #[test]
    fn a_passenger_station_stands_among_a_towns_houses_reaching_the_most() {
        // A town of houses 400 m round its middle at the origin, the other
        // town 5 km east: the AI's stations stood at its edge on free
        // ground, reaching 4 and 5 people (2026-10-31).
        let (centre, other) = (p(0.0, 0.0), p(5_000.0, 0.0));
        let land = town(centre, other, |x, y| x.hypot(y) <= 400.0, |_, _| false);
        let rings = [60.0, 120.0, 180.0, 260.0, 340.0, 420.0, 500.0, 600.0];
        let sites = sites(&land, centre, other, &rings, true);
        let best = sites.first().expect("a place");
        let reached = land.people(best);
        assert!(reached >= PEOPLE_LEAST, "{reached}");
        assert!(
            best.origin.flat_distance(centre) <= 200.0,
            "{:?}",
            best.origin
        );
        // Those reaching the most first; an edge place reaches fewer.
        let edge = sites
            .iter()
            .find(|s| s.origin.flat_distance(centre) >= 500.0)
            .expect("an edge place");
        assert!(land.people(edge) < reached);
        for w in sites.windows(2) {
            assert!(land.people(&w[0]) >= land.people(&w[1]));
        }
        // A cargo station keeps off the houses.
        assert!(sites_free_of_houses(&land, centre, other));
    }

    fn sites_free_of_houses(land: &Land, centre: P, other: P) -> bool {
        sites(land, centre, other, &[60.0, 120.0, 180.0], false).is_empty()
    }

    #[test]
    fn a_passenger_station_never_stands_on_anything_built_but_houses() {
        // Houses all round, an industry's ground through the middle 200 m
        // east and west: no place over it.
        let (centre, other) = (p(0.0, 0.0), p(5_000.0, 0.0));
        let land = town(
            centre,
            other,
            |x, y| x.hypot(y) <= 600.0,
            |_, y| y.abs() <= 120.0,
        );
        for s in sites(&land, centre, other, &[60.0, 120.0, 180.0, 260.0], true) {
            for (x, y) in s.footprint() {
                assert!(y.abs() > 100.0, "over the industry at {x},{y}");
            }
        }
    }

    #[test]
    fn a_houses_reach_is_counted_round_the_station_its_own_ground_left_out() {
        let (centre, other) = (p(0.0, 0.0), p(5_000.0, 0.0));
        let land = town(centre, other, |_, _| true, |_, _| false);
        let station = Station::for_passengers(centre, 0.0);
        let all = land.people(&station);
        // About the cells within 250 m, less the 6 or so under it.
        let circle = std::f64::consts::PI * PEOPLE_REACH * PEOPLE_REACH / (STEP * STEP);
        #[allow(clippy::cast_precision_loss)]
        let n = all as f64;
        assert!(
            n > circle * 0.8 && n < circle * 1.1,
            "{all} of about {circle:.0}"
        );
        let bare = flat(centre, other, |_, _| false);
        assert_eq!(bare.people(&station), 0);
    }

    #[test]
    fn a_passenger_station_has_no_street_within_its_first_run_out() {
        // A town of houses 400 m round, streets every 100 m through it: run
        // 53's stations among them found no heights for their track. Its
        // place leaves a clear run out towards the other town.
        let (centre, other) = (p(0.0, 0.0), p(5_000.0, 0.0));
        let street = |x: f64, y: f64| {
            (x.hypot(y) <= 400.0) && ((x.rem_euclid(100.0) < 20.0) || (y.rem_euclid(100.0) < 20.0))
        };
        let mut land = town(centre, other, |x, y| x.hypot(y) <= 400.0, |_, _| false);
        for j in 0..land.h {
            for i in 0..land.w {
                let (x, y) = land.centre(i, j);
                if street(x, y) {
                    land.flags[j * land.w + i] = land::STREET;
                }
            }
        }
        let rings = [60.0, 120.0, 180.0, 260.0, 340.0, 420.0, 500.0, 600.0];
        let found = sites(&land, centre, other, &rings, true);
        let best = found.first().expect("a place");
        let ((end, out), _) = best.ends_toward(other);
        let (ux, uy) = unit(out);
        let mut d = STEP;
        while d <= EXIT_CLEAR {
            assert!(
                !street(end.x + ux * d, end.y + uy * d),
                "a street {d} m out"
            );
            d += STEP;
        }
        assert!(land.people(best) >= PEOPLE_LEAST, "{}", land.people(best));
    }
    #[test]
    fn track_crosses_town_houses_at_their_price_and_nothing_else_built() {
        let (a, b) = (p(0.0, 0.0), p(4_000.0, 0.0));
        let prices = Prices::default();
        let open = flat(a, b, |_, _| false);
        let houses = town(a, b, |_, _| true, |_, _| false);
        let built = flat(a, b, |_, _| true);
        let step = ((100.0, 0.0), (140.0, 0.0));
        let free = step_cost(&open, &prices, step.0, step.1).unwrap();
        let through = step_cost(&houses, &prices, step.0, step.1).unwrap();
        assert!(
            (through - free - 40.0 * HOUSE_PER_M).abs() < 1.0,
            "{free} {through}"
        );
        assert!(step_cost(&built, &prices, step.0, step.1).is_none());
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
            passengers: false,
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

    const PRICES: Prices = Prices {
        scale: 1.0,
        trestle: false,
    };

    /// Sets the ground under every cell `f` names to the height it gives.
    fn shape(land: &mut Land, f: impl Fn(f64, f64) -> Option<f64>) {
        for j in 0..land.h {
            for i in 0..land.w {
                let (x, y) = land.centre(i, j);
                if let Some(z) = f(x, y) {
                    land.z[j * land.w + i] = z;
                }
            }
        }
    }

    /// Marks every cell `f` names with `flag`.
    fn mark(land: &mut Land, flag: u8, f: impl Fn(f64, f64) -> bool) {
        for j in 0..land.h {
            for i in 0..land.w {
                let (x, y) = land.centre(i, j);
                if f(x, y) {
                    land.flags[j * land.w + i] |= flag;
                }
            }
        }
    }

    fn grades_hold(nodes: &[Node]) {
        for w in nodes.windows(2) {
            let d = w[0].at.flat_distance(w[1].at);
            let grade = (w[1].at.z - w[0].at.z).abs() / d;
            assert!(
                grade <= MAX_GRADE + LEVEL / d + 1e-6,
                "grade {grade} over {d} m"
            );
        }
    }

    #[test]
    fn a_route_goes_round_built_land_and_keeps_its_curves_wide() {
        let (a, b) = (p(0.0, 0.0), p(3_000.0, 0.0));
        // A town in the way, 600 m across, astride the straight line.
        let land = flat(a, b, |x, y| (x - 1_500.0).hypot(y) < 300.0);
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).expect("a route");
        assert!(
            cells.iter().all(|(x, y)| (x - 1_500.0).hypot(*y) >= 300.0),
            "{cells:?}"
        );
        let nodes = track(&land, &PRICES, a, 0.0, b, 0.0, &cells).expect("heights");
        assert_eq!(nodes.first().unwrap().at, a);
        assert_eq!(nodes.last().unwrap().at, b);
        assert!(tightest(&nodes) >= 150.0, "{}", tightest(&nodes));
        for w in nodes.windows(2) {
            let d = w[0].at.flat_distance(w[1].at);
            assert!((50.0..=160.0).contains(&d), "edge {d}");
        }
        // On flat land it keeps to the ground, priced as plain track.
        assert!(nodes.iter().all(|n| n.kind == Kind::Ground));
        assert!(nodes.iter().all(|n| (n.at.z - 10.0).abs() < 1e-6));
        let length: f64 = nodes
            .windows(2)
            .map(|w| w[0].at.flat_distance(w[1].at))
            .sum();
        assert!((cost(&nodes) - costs::TRACK * length).abs() < 1.0);
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
        shape(&mut land, |x, _| {
            ((x - 1_000.0).abs() < 200.0).then_some(-20.0)
        });
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).expect("a route");
        let nodes = track(&land, &PRICES, a, 0.0, b, 0.0, &cells).expect("heights");
        grades_hold(&nodes);
        assert!(
            nodes.iter().any(|n| n.kind == Kind::Bridge),
            "a bridge over the valley"
        );
        // Cheaper than an embankment across it, and every bridge high.
        let fill = 400.0 * PRICES.ground(30.0);
        assert!(cost(&nodes) < fill, "{} >= {fill}", cost(&nodes));
        for w in nodes.windows(2) {
            if w[0].kind == Kind::Bridge {
                let mid = (w[0].at.z + w[1].at.z) / 2.0;
                let x = (w[0].at.x + w[1].at.x) / 2.0;
                assert!(mid - land.height(x, 0.0) >= 2.0, "a bridge at {x}");
            }
        }
    }

    #[test]
    fn a_ridge_is_tunnelled_where_a_cutting_would_cost_more() {
        let (a, b) = (p(0.0, 0.0), p(3_000.0, 0.0));
        let mut land = flat(a, b, |_, _| false);
        // A ridge 60 m high and 600 m wide, from edge to edge of the land.
        shape(&mut land, |x, _| {
            let d = (x - 1_500.0).abs();
            (d < 300.0).then(|| 10.0 + 60.0 * (1.0 - d / 300.0))
        });
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).expect("a route");
        let nodes = track(&land, &PRICES, a, 0.0, b, 0.0, &cells).expect("heights");
        grades_hold(&nodes);
        assert!(
            nodes.iter().any(|n| n.kind == Kind::Tunnel),
            "a tunnel through the ridge"
        );
        for w in nodes.windows(2) {
            if w[0].kind == Kind::Tunnel {
                let x = (w[0].at.x + w[1].at.x) / 2.0;
                let mid = (w[0].at.z + w[1].at.z) / 2.0;
                assert!(land.height(x, 0.0) - mid >= costs::TUNNEL_LEAST - LEVEL);
            }
        }
    }

    #[test]
    fn a_pond_is_gone_round_but_a_river_bridged() {
        let (a, b) = (p(0.0, 0.0), p(3_000.0, 0.0));
        // A pond 200 m across on the straight line: round it is cheaper.
        let mut pond = flat(a, b, |_, _| false);
        mark(&mut pond, land::WATER, |x, y| {
            (x - 1_500.0).hypot(y) < 100.0
        });
        let cells = route(&pond, &PRICES, a, 0.0, b, 0.0).expect("a route");
        assert!(
            cells
                .iter()
                .all(|(x, y)| pond.flags_at(*x, *y).unwrap() & land::WATER == 0),
            "{cells:?}"
        );
        // A river from edge to edge, its bed 4 m down: bridged, as short a
        // bridge as it can be.
        let mut river = flat(a, b, |_, _| false);
        mark(&mut river, land::WATER, |x, _| (x - 1_500.0).abs() < 60.0);
        shape(&mut river, |x, _| {
            ((x - 1_500.0).abs() < 60.0).then_some(6.0)
        });
        let cells = route(&river, &PRICES, a, 0.0, b, 0.0).expect("a route");
        let nodes = track(&river, &PRICES, a, 0.0, b, 0.0, &cells).expect("heights");
        grades_hold(&nodes);
        for w in nodes.windows(2) {
            let wet = samples(&river, w[0].at, w[1].at).iter().any(|s| s.wet);
            if wet {
                assert_eq!(w[0].kind, Kind::Bridge, "over the river at {:?}", w[0].at);
            }
        }
        let bridged = nodes.iter().filter(|n| n.kind == Kind::Bridge).count();
        assert!((1..=3).contains(&bridged), "{bridged} bridge edges");
    }

    /// A street from (x, -400) to (x, 400) at height `z`, turned `turn`
    /// radians about its middle.
    fn street(x: f64, z: f64, turn: f64) -> AiStreet {
        let (s, c) = turn.sin_cos();
        let at = |d: f64| {
            P {
                x: x - d * s,
                y: d * c,
                z,
            }
            .pos()
        };
        AiStreet {
            a: at(-400.0),
            b: at(400.0),
            mid: at(0.0),
            template: Text::lossy("street/country_old_small.lua"),
            length: 800,
            owned: false,
            objects: 0,
            structure: false,
            company_junction: false,
        }
    }

    #[test]
    fn a_level_crossing_splits_the_street_at_its_height() {
        let (a, b) = (p(0.0, 0.0), p(2_000.0, 0.0));
        let mut land = flat(a, b, |_, _| false);
        mark(&mut land, land::STREET, |x, _| (x - 1_000.0).abs() < 30.0);
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).expect("a route");
        // The route's street cells, bunched, for the game to name.
        let runs = street_runs(&land, &cells);
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert!((runs[0].0.x - 1_000.0).abs() < 60.0);
        let crossing_street = street(1_000.0, 11.0, 0.0);
        let nodes = align(&land, a, 0.0, b, 0.0, &cells);
        let (crossed, holds) = cross(&nodes, std::slice::from_ref(&crossing_street));
        // A node added, or one near the street moved onto it.
        assert!((nodes.len()..=nodes.len() + 1).contains(&crossed.len()));
        let k = crossed.iter().position(|n| n.crossing.is_some()).unwrap();
        assert!((crossed[k].at.x - 1_000.0).abs() < 0.5 && crossed[k].at.y.abs() < 40.0);
        assert_eq!(holds[k], Hold::Street(11.0));
        let planned = profile(&land, &PRICES, &crossed, &holds).expect("heights");
        grades_hold(&planned);
        // At the street's height, on the ground either side, priced with
        // its crossing.
        assert!((planned[k].at.z - 11.0).abs() < 1e-9);
        assert!(planned[k].crossing.is_some());
        assert_eq!(planned[k - 1].kind, Kind::Ground);
        assert_eq!(planned[k].kind, Kind::Ground);
        assert!(
            cost(&planned) > cost(&profile(&land, &PRICES, &nodes, &ends_held(&nodes)).unwrap())
        );
        // Its vertex splits the street, inside a piece, never at its ends.
        let built = pieces(&planned, 1923, &PRICES);
        let split = built
            .iter()
            .flat_map(|p| {
                p.build
                    .polyline
                    .vertices
                    .iter()
                    .enumerate()
                    .map(move |(i, v)| (p, i, v))
            })
            .find(|(_, _, v)| matches!(v.resolve, Resolve::Split(_)))
            .expect("a split vertex");
        let (piece, i, vertex) = split;
        assert!(i > 0 && i + 1 < piece.build.polyline.vertices.len());
        assert_eq!(
            vertex.resolve,
            Resolve::Split(EdgeRef {
                network: Network::Street,
                ends: EdgeEnds {
                    a: crossing_street.a,
                    b: crossing_street.b,
                },
            })
        );
        // The pieces still chain.
        for w in built.windows(2) {
            assert_eq!(w[0].last, w[1].first);
        }
    }

    #[test]
    fn where_no_level_crossing_may_be_laid_the_line_goes_over_or_under() {
        let (a, b) = (p(0.0, 0.0), p(2_000.0, 0.0));
        let land = flat(a, b, |_, _| false);
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).expect("a route");
        let nodes = align(&land, a, 0.0, b, 0.0, &cells);
        let narrow = street(1_000.0, 10.0, 70f64.to_radians());
        let mut stops = street(1_000.0, 10.0, 0.0);
        stops.objects = 1;
        let mut owned = street(1_000.0, 10.0, 0.0);
        owned.owned = true;
        let mut junction = street(1_000.0, 10.0, 0.0);
        junction.company_junction = true;
        // Ending by the line.
        let mut short = street(1_000.0, 10.0, 0.0);
        short.a = P {
            x: 1_000.0,
            y: 15.0,
            z: 10.0,
        }
        .pos();
        short.mid = P {
            x: 1_000.0,
            y: 207.5,
            z: 10.0,
        }
        .pos();
        // Up a steep hill.
        let mut hill = street(1_000.0, 10.0, 0.0);
        hill.a.z = -30_000;
        hill.b.z = 50_000;
        hill.mid.z = 10_000;
        for (what, s) in [
            ("20° off the line", narrow),
            ("stops on it", stops),
            ("a company's", owned),
            ("by a company's junction", junction),
            ("ending by the line", short),
            ("steep", hill),
        ] {
            let (crossed, holds) = cross(&nodes, &[s]);
            assert!(crossed.iter().all(|n| n.crossing.is_none()), "{what}");
            let k = holds
                .iter()
                .position(|h| matches!(h, Hold::Clear(_)))
                .unwrap_or_else(|| panic!("{what}: met"));
            let Hold::Clear(z) = holds[k] else {
                unreachable!()
            };
            let planned = profile(&land, &PRICES, &crossed, &holds).expect(what);
            let over = planned[k].at.z >= z + OVER - 1e-9;
            let under = planned[k].at.z <= z - UNDER + 1e-9;
            assert!(over || under, "{what}: {} by {z}", planned[k].at.z);
            let kind = if over { Kind::Bridge } else { Kind::Tunnel };
            assert_eq!(planned[k - 1].kind, kind, "{what}");
            assert_eq!(planned[k].kind, kind, "{what}");
        }
        // A street on a bridge is not met; one named twice, once.
        let mut high = street(1_000.0, 10.0, 0.0);
        high.structure = true;
        let (crossed, holds) = cross(&nodes, &[high]);
        assert_eq!(crossed.len(), nodes.len());
        assert!(holds.iter().all(|h| matches!(h, Hold::Free | Hold::At(_))));
        let twice = street(1_000.0, 10.0, 0.0);
        let (_, holds) = cross(&nodes, &[twice.clone(), twice]);
        assert_eq!(
            holds
                .iter()
                .filter(|h| matches!(h, Hold::Street(_)))
                .count(),
            1
        );
    }

    #[test]
    fn a_street_met_by_a_node_takes_its_place() {
        let (a, b) = (p(0.0, 0.0), p(2_000.0, 0.0));
        let land = flat(a, b, |_, _| false);
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).expect("a route");
        let nodes = align(&land, a, 0.0, b, 0.0, &cells);
        // A street 5 m past an inner node.
        let k = nodes.len() / 2;
        let x = nodes[k].at.x + 5.0;
        let (crossed, holds) = cross(&nodes, &[street(x, 10.0, 0.0)]);
        assert_eq!(crossed.len(), nodes.len(), "moved, not added");
        assert!(crossed[k].crossing.is_some());
        assert!((crossed[k].at.x - x).abs() < 1.0);
        assert!(matches!(holds[k], Hold::Street(_)));
        // No edge shorter than the merge distance.
        for w in crossed.windows(2) {
            assert!(w[0].at.flat_distance(w[1].at) >= MERGE - 5.0);
        }
    }
    #[test]
    fn a_street_far_below_the_line_is_bridged_not_crossed() {
        let (a, b) = (p(0.0, 0.0), p(2_000.0, 0.0));
        let mut land = flat(a, b, |_, _| false);
        // A cutting 12 m deep for a street, 60 m wide, across the line.
        shape(&mut land, |x, _| {
            ((x - 1_000.0).abs() < 30.0).then_some(-2.0)
        });
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).expect("a route");
        let nodes = align(&land, a, 0.0, b, 0.0, &cells);
        let (crossed, holds) = cross(&nodes, &[street(1_000.0, -2.0, 0.0)]);
        let planned = profile(&land, &PRICES, &crossed, &holds).expect("heights");
        grades_hold(&planned);
        let k = holds
            .iter()
            .position(|h| matches!(h, Hold::Street(_)))
            .unwrap();
        assert!(planned[k].at.z >= -2.0 + OVER, "{}", planned[k].at.z);
        assert!(planned[k].crossing.is_none(), "no crossing over it");
        assert_eq!(planned[k - 1].kind, Kind::Bridge);
        assert_eq!(planned[k].kind, Kind::Bridge);
    }

    #[test]
    fn pieces_chain_from_station_to_station() {
        let (a, b) = (p(0.0, 0.0), p(2_000.0, 0.0));
        let land = flat(a, b, |_, _| false);
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).unwrap();
        let nodes = track(&land, &PRICES, a, 0.0, b, 0.0, &cells).unwrap();
        let built = pieces(&nodes, 1923, &PRICES);
        assert!(built.len() >= 3);
        for (k, piece) in built.iter().enumerate() {
            let v = &piece.build.polyline.vertices;
            assert!(
                matches!(v[0].resolve, Resolve::Node(_)),
                "piece {k} starts on a node"
            );
            assert!(piece.build.polyline.links.len() <= PIECE_EDGES);
            assert_eq!(v.len(), piece.last - piece.first + 1);
            if k + 1 < built.len() {
                assert_eq!(v[v.len() - 1].resolve, Resolve::New);
                assert_eq!(
                    v[v.len() - 1].pos,
                    built[k + 1].build.polyline.vertices[0].pos
                );
            } else {
                assert!(matches!(v[v.len() - 1].resolve, Resolve::Node(_)));
                assert_eq!(v[v.len() - 1].pos, b.pos());
            }
        }
        assert_eq!(built[0].build.track.as_str(), track_template(1923));
        let priced: f64 = built.iter().map(|p| p.cost).sum();
        assert!((priced - cost(&nodes)).abs() < 1e-6);
    }

    #[test]
    fn sites_keep_off_built_land_and_face_the_target() {
        let (target, other) = (p(0.0, 0.0), p(2_000.0, 0.0));
        // Built land east of the target, towards the other end.
        let land = flat(target, other, |x, _| x > 100.0 && x < 600.0);
        let all = sites(&land, target, other, &[200.0], false);
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
        assert!(route(&land, &PRICES, a, 0.0, b, 0.0).is_none());
    }

    #[test]
    fn a_level_crossing_lies_level_on_a_slope() {
        // The land climbs 3 % east; a street across the line at x = 1 000.
        let (a, b) = (
            p(0.0, 0.0),
            P {
                x: 2_000.0,
                y: 0.0,
                z: 70.0,
            },
        );
        let mut land = flat(a, b, |_, _| false);
        shape(&mut land, |x, _| Some(10.0 + 0.03 * x));
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).expect("a route");
        let nodes = align(&land, a, 0.0, b, 0.0, &cells);
        let (crossed, holds) = cross(&nodes, &[street(1_000.0, 40.0, 0.0)]);
        let planned = profile(&land, &PRICES, &crossed, &holds).expect("heights");
        let k = planned
            .iter()
            .position(|n| n.crossing.is_some())
            .expect("a crossing");
        for w in [&planned[k - 1..=k], &planned[k..=k + 1]] {
            let d = w[0].at.flat_distance(w[1].at);
            let grade = (w[1].at.z - w[0].at.z).abs() / d;
            assert!(
                grade <= CROSSING_GRADE + LEVEL / 2.0 / d + 1e-9,
                "{grade} over {d} m"
            );
        }
        assert!(planned.windows(2).all(|w| {
            let d = w[0].at.flat_distance(w[1].at);
            (w[1].at.z - w[0].at.z).abs() <= STEEPEST * d + LEVEL
        }));
    }

    #[test]
    fn a_long_line_gets_a_passing_loop_with_a_signal_at_each_exit() {
        let (a, b) = (p(0.0, 0.0), p(3_000.0, 0.0));
        let land = flat(a, b, |_, _| false);
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).unwrap();
        let nodes = track(&land, &PRICES, a, 0.0, b, 0.0, &cells).unwrap();
        let sites = loop_sites(&land, &nodes);
        assert_eq!(sites.len(), 1, "one loop on 3 km");
        let (k, side) = sites[0];
        // Near the middle, clear of the stations.
        let m = k + LOOP_EDGES;
        assert!(k > 0 && m < nodes.len() - 1);
        assert!(nodes[k].at.x > 800.0 && nodes[m].at.x < 2_200.0, "{k}");
        let build = passing_loop(&nodes, k, side, 1923).unwrap();
        let v = &build.polyline.vertices;
        assert_eq!(v.len(), LOOP_EDGES + 1);
        // Off the line's nodes at its ends, as switches; beside it between.
        assert_eq!(v[0].pos, nodes[k].at.pos());
        assert_eq!(v[LOOP_EDGES].pos, nodes[m].at.pos());
        assert!(matches!(v[0].resolve, Resolve::Node(Network::Track)));
        assert!(matches!(
            v[LOOP_EDGES].resolve,
            Resolve::Node(Network::Track)
        ));
        for (i, vertex) in v.iter().enumerate().skip(1).take(LOOP_EDGES - 1) {
            let d = P::of(vertex.pos).flat_distance(nodes[k + i].at);
            assert!((d - LOOP_OFFSET).abs() < 0.01, "{d}");
            assert_eq!(vertex.resolve, Resolve::New);
        }
        let [main, back] = loop_signals(&nodes, k, side, 1923);
        for s in [&main, &back] {
            assert_eq!(s.object, EdgeObjectKind::Signal);
            assert!(s.one_way);
            assert_eq!(s.edge.network, Network::Track);
            assert_eq!(s.model.as_str(), "infrastructure/signal/signal_path_a.con");
        }
        // On the line's own track by the loop's far end, passing trains on.
        assert_eq!(main.edge.ends.a, nodes[m - 1].at.pos());
        assert_eq!(main.edge.ends.b, nodes[m].at.pos());
        assert_eq!(main.left, FORWARD_LEFT);
        // Clear of the switches, where the game takes a signal.
        let at = P::of(main.at);
        assert!(at.flat_distance(nodes[m].at) > 60.0, "{at:?}");
        assert!(at.flat_distance(nodes[m].at) > at.flat_distance(nodes[m - 1].at));
        assert!(main.direction.x > 900_000, "{:?}", main.direction);
        // On the loop's by its near end, passing trains back.
        assert_eq!(back.edge.ends.a, nodes[k].at.pos());
        assert_eq!(back.edge.ends.b, v[1].pos);
        assert_eq!(back.left, !FORWARD_LEFT);
        assert!(P::of(back.at).flat_distance(nodes[k].at) > 60.0);
        // A short line has none, nor one whose middle is all built about.
        let (c, d) = (p(0.0, 0.0), p(1_500.0, 0.0));
        let short = flat(c, d, |_, _| false);
        let cells = route(&short, &PRICES, c, 0.0, d, 0.0).unwrap();
        let nodes = track(&short, &PRICES, c, 0.0, d, 0.0, &cells).unwrap();
        assert!(loop_sites(&short, &nodes).is_empty());
        let crowded = flat(a, b, |_, y| y.abs() > 0.0 && y.abs() < 60.0);
        let cells: Vec<(f64, f64)> = (0..=75).map(|i| (f64::from(i) * 40.0, 0.0)).collect();
        let nodes = align(&crowded, a, 0.0, b, 0.0, &cells);
        assert!(loop_sites(&crowded, &nodes).is_empty());
    }

    #[test]
    fn a_longer_line_gets_a_loop_for_each_stretch_set_apart() {
        let (a, b) = (p(0.0, 0.0), p(6_000.0, 0.0));
        let land = flat(a, b, |_, _| false);
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).unwrap();
        let nodes = track(&land, &PRICES, a, 0.0, b, 0.0, &cells).unwrap();
        let sites = loop_sites(&land, &nodes);
        assert_eq!(sites.len(), 2, "{sites:?}");
        // Near a third and two thirds of the line, single track between.
        let (k0, k1) = (sites[0].0, sites[1].0);
        assert!(k0 + LOOP_EDGES + 1 < k1);
        assert!(
            (nodes[k0].at.x - 2_000.0).abs() < 400.0,
            "{}",
            nodes[k0].at.x
        );
        assert!(
            (nodes[k1].at.x - 4_000.0).abs() < 400.0,
            "{}",
            nodes[k1].at.x
        );
    }

    #[test]
    fn another_railway_is_bridged_over_never_crossed_on_the_ground() {
        let (a, b) = (p(0.0, 0.0), p(3_000.0, 0.0));
        // A railway from edge to edge across the line.
        let mut land = flat(a, b, |_, _| false);
        mark(&mut land, land::TRACK, |x, _| (x - 1_500.0).abs() < 30.0);
        let cells = route(&land, &PRICES, a, 0.0, b, 0.0).expect("a route over it");
        let nodes = track(&land, &PRICES, a, 0.0, b, 0.0, &cells).expect("heights");
        grades_hold(&nodes);
        for w in nodes.windows(2) {
            for s in samples(&land, w[0].at, w[1].at) {
                if s.rail {
                    let h = w[0].at.z + (w[1].at.z - w[0].at.z) * s.t - s.ground;
                    assert!(
                        (w[0].kind == Kind::Bridge && h >= OVER - 1e-9)
                            || (w[0].kind == Kind::Tunnel && -h >= UNDER - 1e-9),
                        "{:?} at {h} over the railway",
                        w[0].kind
                    );
                }
            }
        }
    }
}
