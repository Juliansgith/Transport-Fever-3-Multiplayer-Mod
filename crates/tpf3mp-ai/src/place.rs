//! Where the AI builds, as plain geometry over what the game told it
//! (docs/AI_PLAYERS.md, "Building, mode by mode"): a stop on an existing
//! street, and a road depot facing a street, joined to it as the game's
//! construction tool joins one. Every action made here is put to the
//! game's verdict before it is sent.

use tpf3mp_bridge::ai::AiStreet;
use tpf3mp_proto::{
    BoundedVec, Text,
    action::{
        ConstructionBuild, EdgeEnds, EdgeKind, EdgeObjectKind, EdgeRef, Link, Network, Param,
        ParamValue, PlaceStop, Polyline, Pos, ResName, Resolve, Structure, Tangent, Transform,
        UnitDir, Vertex,
    },
};

/// The road depot TF3 builds road vehicles in.
pub const ROAD_DEPOT: &str = "::/depots/road/road_depot/road_depot.con";
/// Where a road depot's entrance node stands from its origin, along its
/// own -y axis, in metres: its apron's first point
/// (`depots/road/road_depot/road_depot.script.lua`, curve `lane1`).
pub const DEPOT_ENTRANCE: f64 = 35.0;
/// How far short of the street's centre line a depot's entrance ends, in
/// metres: as the construction tool leaves it, about 2 m short of the
/// junction it splits into the street, which the game's refresh of the
/// construction then snaps it onto (docs/HOOKS.md, "The construction
/// tool"). A separate road onto the entrance the game refuses: the
/// entrance is the construction's own frozen node.
pub const DEPOT_GAP: f64 = 2.0;

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

/// The middle of a street edge's curve.
pub fn middle(street: &AiStreet) -> P {
    P::of(street.mid)
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
        at: street.mid,
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
/// of a to b, -1 its right), facing it, its entrance [`DEPOT_GAP`] short of
/// the street's middle: where its entrance is, and the build. The build's
/// connection splits the street there through a new junction and runs the
/// entrance edge to it, as the construction tool's proposal does: every
/// game builds the split with the depot (the entrance edge is the depot's
/// own, which it makes itself), and its refresh of the depot snaps the
/// entrance onto the junction (tpf3mp/apply.lua, BuildConstruction).
#[allow(clippy::cast_possible_truncation)]
pub fn depot(
    street: &AiStreet,
    side: f64,
    year: i32,
    seed: i64,
    name: &str,
) -> Option<(P, ConstructionBuild)> {
    let (ux, uy) = along(street);
    // The left normal of a to b, then the side asked for.
    let (nx, ny) = (-uy * side, ux * side);
    let foot = middle(street);
    let entrance = foot.plus(nx * DEPOT_GAP, ny * DEPOT_GAP);
    // The depot's -y axis points from its entrance to the street: towards
    // -n. With its basis [cos, sin; -sin, cos], -y is (sin, -cos).
    let (sin, cos) = (-nx, ny);
    let origin = entrance.plus(nx * DEPOT_ENTRANCE, ny * DEPOT_ENTRANCE);
    let micro = |v: f64| (v * 1_000_000.0).round() as i32;
    let (from, to) = (foot.pos(), entrance.pos());
    let tangent = Tangent {
        x: to.x - from.x,
        y: to.y - from.y,
        z: to.z - from.z,
    };
    let connection = Polyline::new(
        BoundedVec::new(vec![
            Vertex {
                pos: from,
                resolve: Resolve::Split(edge_ref(street)),
            },
            Vertex {
                pos: to,
                resolve: Resolve::New,
            },
        ])
        .ok()?,
        BoundedVec::new(vec![Link {
            from: 0,
            to: 1,
            tangent0: tangent,
            tangent1: tangent,
            structure: Structure::Ground,
            kind: Some(EdgeKind {
                network: Network::Street,
                template: street.template.clone(),
                style: None,
            }),
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
        connection: Some(Box::new(connection)),
    };
    Some((entrance, build))
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
            mid: Pos {
                x: (ax + bx) / 2,
                y: (ay + by) / 2,
                z: 5_000,
            },
            template: text("::/street/town_small.street_template"),
            length: 100,
            owned: false,
            objects: 0,
            structure: false,
            company_junction: false,
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
    fn a_depot_faces_the_street_its_entrance_short_of_a_split_in_its_middle() {
        // A street along x: its left is +y.
        let s = street(0, 0, 100_000, 0);
        let (entrance, build) = depot(&s, 1.0, 1925, 7, "Rossi Depot").unwrap();
        assert_eq!(
            entrance.pos(),
            Pos {
                x: 50_000,
                y: 2_000,
                z: 5_000
            }
        );
        // The origin 35 m further from the street.
        assert_eq!(
            build.transform.origin,
            Pos {
                x: 50_000,
                y: 37_000,
                z: 5_000
            }
        );
        // Its -y axis, (sin, -cos), points at the street: (0, -1).
        let [cos, sin, ..] = build.transform.basis;
        assert_eq!((sin, -cos), (0, -1_000_000));
        // And its entrance, origin + 35 * (sin, -cos), is where we said.
        let x = f64::from(build.transform.origin.x) / 1000.0 + 35.0 * f64::from(sin) / 1e6;
        let y = f64::from(build.transform.origin.y) / 1000.0 - 35.0 * f64::from(cos) / 1e6;
        assert!((x - 50.0).abs() < 1e-6 && (y - 2.0).abs() < 1e-6);
        assert!(build.params.iter().any(|p| p.key.as_str() == "seed"));
        // Its connection: the street split in its middle, and the entrance
        // edge from that junction to the entrance, which the depot makes
        // itself (the mod peels it off) and its refresh snaps.
        let connection = build.connection.as_ref().unwrap();
        let vertices = &connection.vertices;
        assert!(matches!(&vertices[0].resolve, Resolve::Split(e) if e.ends.a == s.a));
        assert_eq!(vertices[0].pos, s.mid);
        assert_eq!(vertices[1].resolve, Resolve::New);
        assert_eq!(vertices[1].pos, entrance.pos());
        let link = &connection.links[0];
        assert_eq!(
            link.kind.as_ref().map(|k| (k.network, k.template.clone())),
            Some((Network::Street, s.template.clone()))
        );
        assert!(link.owned && connection.removals.is_empty());
        let action = tpf3mp_proto::action::Action::BuildConstruction(build);
        assert!(action.to_payload().is_ok(), "a valid action");
        // The other side, of a street along y.
        let (entrance, _) = depot(&street(0, 0, 0, 100_000), -1.0, 1925, 7, "D").unwrap();
        assert_eq!(
            entrance.pos(),
            Pos {
                x: 2_000,
                y: 50_000,
                z: 5_000
            }
        );
    }

    #[test]
    fn a_stop_and_a_split_take_the_curves_middle_not_the_chords() {
        let mut s = street(0, 0, 100_000, 0);
        s.mid = Pos {
            x: 50_000,
            y: 3_000,
            z: 5_000,
        };
        assert_eq!(stop(&s, 1925).at, s.mid);
        let (_, build) = depot(&s, 1.0, 1925, 7, "D").unwrap();
        assert_eq!(build.connection.unwrap().vertices[0].pos, s.mid);
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
