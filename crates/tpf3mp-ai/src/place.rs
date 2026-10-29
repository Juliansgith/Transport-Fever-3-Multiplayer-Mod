//! Where the AI builds, as plain geometry over what the game told it
//! (docs/AI_PLAYERS.md, "Building, mode by mode"): a stop on an existing
//! street, a road depot facing a street, and the road joining the two.
//! Every action made here is put to the game's verdict before it is sent.

use tpf3mp_bridge::ai::AiStreet;
use tpf3mp_proto::{
    BoundedVec, Text,
    action::{
        ConstructionBuild, EdgeEnds, EdgeObjectKind, EdgeRef, Link, Network, Param, ParamValue,
        PlaceStop, Polyline, Pos, ResName, Resolve, RoadBuild, Structure, Tangent, Tram, Transform,
        UnitDir, Vertex,
    },
};

/// The road depot TF3 builds road vehicles in.
pub const ROAD_DEPOT: &str = "::/depots/road/road_depot/road_depot.con";
/// Where a road depot's entrance node stands from its origin, along its
/// own -y axis, in metres: its apron's first point
/// (`depots/road/road_depot/road_depot.script.lua`, curve `lane1`).
pub const DEPOT_ENTRANCE: f64 = 35.0;
/// How far from the street's centre line the AI puts a depot's entrance,
/// in metres: the join between them is that long.
pub const DEPOT_SETBACK: f64 = 14.0;

/// A street edge's length the AI stops buses on at least, in metres.
pub const STOP_EDGE_MIN: u32 = 24;

/// The two-sided bus stop TF3 offers in `year`: old before 1920, mid until
/// 1980, new after (`stations/street/small_stops/*.con`, `availability`).
pub fn stop_model(year: i32) -> &'static str {
    if year < 1920 {
        "stations/street/small_stops/small_old_twosided.con"
    } else if year < 1980 {
        "stations/street/small_stops/small_mid_twosided.con"
    } else {
        "stations/street/small_stops/small_new_twosided.con"
    }
}

fn text<const N: usize>(s: &str) -> Text<N> {
    Text::lossy(s)
}

fn res(s: &str) -> ResName {
    text(s)
}

/// A point in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct P {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl P {
    pub fn of(pos: Pos) -> Self {
        Self {
            x: f64::from(pos.x) / 1000.0,
            y: f64::from(pos.y) / 1000.0,
            z: f64::from(pos.z) / 1000.0,
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    pub fn pos(self) -> Pos {
        Pos {
            x: (self.x * 1000.0).round() as i32,
            y: (self.y * 1000.0).round() as i32,
            z: (self.z * 1000.0).round() as i32,
        }
    }

    pub fn flat_distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    fn plus(self, dx: f64, dy: f64) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            z: self.z,
        }
    }
}

/// The midpoint of a street edge.
pub fn middle(street: &AiStreet) -> P {
    let (a, b) = (P::of(street.a), P::of(street.b));
    P {
        x: (a.x + b.x) / 2.0,
        y: (a.y + b.y) / 2.0,
        z: (a.z + b.z) / 2.0,
    }
}

/// The edge's direction on the ground, a to b, as a unit vector.
fn along(street: &AiStreet) -> (f64, f64) {
    let (a, b) = (P::of(street.a), P::of(street.b));
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx.hypot(dy).max(1e-6);
    (dx / length, dy / length)
}

fn edge_ref(street: &AiStreet) -> EdgeRef {
    EdgeRef {
        network: Network::Street,
        ends: EdgeEnds {
            a: street.a,
            b: street.b,
        },
    }
}

/// Whether the AI may stop buses on this edge, or split it for a depot: the
/// town's or anyone's street on the ground, empty, long enough.
pub fn usable(street: &AiStreet) -> bool {
    !street.structure && street.objects == 0 && street.length >= STOP_EDGE_MIN
}

/// A two-sided bus stop in the middle of `street`.
#[allow(clippy::cast_possible_truncation)]
pub fn stop(street: &AiStreet, year: i32) -> PlaceStop {
    let (ux, uy) = along(street);
    PlaceStop {
        edge: edge_ref(street),
        at: middle(street).pos(),
        left: false,
        direction: UnitDir {
            x: (ux * 1_000_000.0).round() as i32,
            y: (uy * 1_000_000.0).round() as i32,
            z: 0,
        },
        model: res(stop_model(year)),
        two_sided: true,
        object: EdgeObjectKind::default(),
        one_way: false,
        name: None,
        params: BoundedVec::default(),
    }
}

/// A road depot beside the middle of `street`, on side `side` (+1 its left
/// of a to b, -1 its right), its entrance [`DEPOT_SETBACK`] metres from the
/// street and facing it: where its entrance is, and the build.
#[allow(clippy::cast_possible_truncation)]
pub fn depot(
    street: &AiStreet,
    side: f64,
    year: i32,
    seed: i64,
    name: &str,
) -> (P, ConstructionBuild) {
    let (ux, uy) = along(street);
    // The left normal of a to b, then the side asked for.
    let (nx, ny) = (-uy * side, ux * side);
    let foot = middle(street);
    let entrance = foot.plus(nx * DEPOT_SETBACK, ny * DEPOT_SETBACK);
    // The depot's -y axis points from its entrance to the street: towards
    // -n. With its basis [cos, sin; -sin, cos], -y is (sin, -cos).
    let (sin, cos) = (-nx, ny);
    let origin = entrance.plus(nx * DEPOT_ENTRANCE, ny * DEPOT_ENTRANCE);
    let micro = |v: f64| (v * 1_000_000.0).round() as i32;
    let build = ConstructionBuild {
        file: res(ROAD_DEPOT),
        transform: Transform {
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
        },
        params: BoundedVec::new(vec![
            Param {
                key: text("seed"),
                value: ParamValue::Int(seed),
            },
            Param {
                key: text("year"),
                value: ParamValue::Int(i64::from(year)),
            },
        ])
        .unwrap_or_default(),
        name: text(name),
        replaces: None,
        connection: None,
    };
    (entrance, build)
}

/// The road from a depot's entrance node at `entrance` to the middle of
/// `street`, splitting it there: the company's own, in the street's
/// template.
pub fn join(entrance: Pos, street: &AiStreet) -> Option<RoadBuild> {
    let foot = middle(street).pos();
    let (from, to) = (P::of(entrance), P::of(foot));
    let tangent = Tangent {
        x: foot.x - entrance.x,
        y: foot.y - entrance.y,
        z: foot.z - entrance.z,
    };
    if from.flat_distance(to) < 1.0 {
        return None;
    }
    let polyline = Polyline::new(
        BoundedVec::new(vec![
            Vertex {
                pos: entrance,
                resolve: Resolve::Node(Network::Street),
            },
            Vertex {
                pos: foot,
                resolve: Resolve::Split(edge_ref(street)),
            },
        ])
        .ok()?,
        BoundedVec::new(vec![Link {
            from: 0,
            to: 1,
            tangent0: tangent,
            tangent1: tangent,
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
        }])
        .ok()?,
        BoundedVec::default(),
    )
    .ok()?;
    Some(RoadBuild {
        street: street.template.clone(),
        style: None,
        bus_lane: false,
        tram: Tram::None,
        polyline,
        crossing_type: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn street(ax: i32, ay: i32, bx: i32, by: i32) -> AiStreet {
        AiStreet {
            a: Pos {
                x: ax,
                y: ay,
                z: 5_000,
            },
            b: Pos {
                x: bx,
                y: by,
                z: 5_000,
            },
            template: text("::/street/town_small.street_template"),
            length: 100,
            owned: false,
            objects: 0,
            structure: false,
        }
    }

    #[test]
    fn the_stop_model_follows_the_years_the_game_offers_it() {
        assert!(stop_model(1900).contains("small_old"));
        assert!(stop_model(1920).contains("small_mid"));
        assert!(stop_model(1979).contains("small_mid"));
        assert!(stop_model(1980).contains("small_new"));
    }

    #[test]
    fn a_stop_stands_in_the_middle_facing_along_the_street() {
        let s = stop(&street(0, 0, 100_000, 0), 1925);
        assert_eq!(
            s.at,
            Pos {
                x: 50_000,
                y: 0,
                z: 5_000
            }
        );
        assert_eq!(
            s.direction,
            UnitDir {
                x: 1_000_000,
                y: 0,
                z: 0
            }
        );
        assert!(s.two_sided);
        assert!(s.model.as_str().contains("small_mid"));
    }

    #[test]
    fn a_depot_faces_the_street_with_its_entrance_set_back() {
        // A street along x: its left is +y.
        let (entrance, build) = depot(&street(0, 0, 100_000, 0), 1.0, 1925, 7, "Rossi Depot");
        assert_eq!(
            entrance.pos(),
            Pos {
                x: 50_000,
                y: 14_000,
                z: 5_000
            }
        );
        // The origin 35 m further from the street.
        assert_eq!(
            build.transform.origin,
            Pos {
                x: 50_000,
                y: 49_000,
                z: 5_000
            }
        );
        // Its -y axis, (sin, -cos), points at the street: (0, -1).
        let [cos, sin, ..] = build.transform.basis;
        assert_eq!((sin, -cos), (0, -1_000_000));
        // And its entrance, origin + 35 * (sin, -cos), is where we said.
        let x = f64::from(build.transform.origin.x) / 1000.0 + 35.0 * f64::from(sin) / 1e6;
        let y = f64::from(build.transform.origin.y) / 1000.0 - 35.0 * f64::from(cos) / 1e6;
        assert!((x - 50.0).abs() < 1e-6 && (y - 14.0).abs() < 1e-6);
        assert!(build.params.iter().any(|p| p.key.as_str() == "seed"));
        // The other side, of a street along y.
        let (entrance, _) = depot(&street(0, 0, 0, 100_000), -1.0, 1925, 7, "D");
        assert_eq!(
            entrance.pos(),
            Pos {
                x: 14_000,
                y: 50_000,
                z: 5_000
            }
        );
    }

    #[test]
    fn the_join_runs_from_the_entrance_to_a_split_in_the_middle() {
        let s = street(0, 0, 100_000, 0);
        let road = join(
            Pos {
                x: 50_000,
                y: 14_000,
                z: 5_000,
            },
            &s,
        )
        .unwrap();
        let vertices = &road.polyline.vertices;
        assert_eq!(vertices[0].resolve, Resolve::Node(Network::Street));
        assert!(matches!(vertices[1].resolve, Resolve::Split(_)));
        assert_eq!(
            vertices[1].pos,
            Pos {
                x: 50_000,
                y: 0,
                z: 5_000
            }
        );
        assert!(road.polyline.links[0].owned);
        assert_eq!(road.street, s.template);
        let action = tpf3mp_proto::action::Action::BuildRoad(road);
        assert!(action.to_payload().is_ok(), "a valid action");
        assert!(
            join(
                Pos {
                    x: 50_000,
                    y: 0,
                    z: 5_000
                },
                &s
            )
            .is_none(),
            "nothing to join"
        );
    }

    #[test]
    fn only_an_empty_street_on_the_ground_takes_a_stop() {
        let mut s = street(0, 0, 100_000, 0);
        assert!(usable(&s));
        s.objects = 1;
        assert!(!usable(&s));
        s.objects = 0;
        s.structure = true;
        assert!(!usable(&s));
        s.structure = false;
        s.length = 10;
        assert!(!usable(&s));
    }
}
