//! Every action round-trips through postcard and through an intent's
//! payload, and whatever bytes a peer sends as an action, decoding gives an
//! error or a well-formed action, never a panic. Same method as
//! `hostile_bytes.rs`: valid samples of every variant, corrupted.

#![allow(clippy::unwrap_used)]

use proptest::{collection::vec, prelude::*, sample::Index};
use tpf3mp_proto::{
    BoundedVec, MAX_PAYLOAD, Payload, Text,
    action::{
        ACTION_SCHEMA_VERSION, Action, AssignLine, Bulldoze, BuyVehicle, CompanyId, CompanyOp,
        ConstructionBuild, ConstructionRef, CreateLine, EdgeEnds, EdgeRef, EditLine, LineChange,
        LineId, LineStop, Link, MAX_EDGES, MAX_VERTICES, Network, Param, ParamValue, PlaceStop,
        Polyline, Pos, Pos2, Resolve, Rgb, RoadBuild, StationId, Structure, Tangent, Terraform,
        TerrainCell, TrackBuild, Tram, Transform, UnitDir, VehicleId, Vertex,
    },
};

fn text<const N: usize>(value: &str) -> Text<N> {
    Text::new(value).unwrap()
}

fn list<T, const N: usize>(items: Vec<T>) -> BoundedVec<T, N> {
    BoundedVec::new(items).unwrap()
}

fn pos(x: i32, y: i32, z: i32) -> Pos {
    Pos { x, y, z }
}

fn ends(a: Pos, b: Pos) -> EdgeEnds {
    EdgeEnds { a, b }
}

fn polyline() -> Polyline {
    Polyline::new(
        list(vec![
            Vertex {
                pos: pos(1_204_500, -88_250, 31_400),
                resolve: Resolve::Node(Network::Street),
            },
            Vertex {
                pos: pos(1_264_500, -88_250, 33_100),
                resolve: Resolve::Split(EdgeRef {
                    network: Network::Track,
                    ends: ends(
                        pos(1_260_000, -120_000, 33_000),
                        pos(1_270_000, -40_000, 33_200),
                    ),
                }),
            },
            Vertex {
                pos: pos(1_324_500, -60_000, 40_000),
                resolve: Resolve::New,
            },
        ]),
        list(vec![
            Link {
                from: 0,
                to: 1,
                tangent0: Tangent {
                    x: 60_000,
                    y: 0,
                    z: 1_700,
                },
                tangent1: Tangent {
                    x: 60_000,
                    y: 0,
                    z: 1_700,
                },
                structure: Structure::Ground,
            },
            Link {
                from: 1,
                to: 2,
                tangent0: Tangent {
                    x: 55_000,
                    y: 20_000,
                    z: 0,
                },
                tangent1: Tangent {
                    x: 60_000,
                    y: 28_250,
                    z: -2_000,
                },
                structure: Structure::Bridge(text("bridge/cement.lua")),
            },
        ]),
        list(vec![ends(pos(0, 0, 0), pos(-5_000, 12_000, 300))]),
    )
    .unwrap()
}

fn depot() -> ConstructionRef {
    ConstructionRef {
        file: text("depot/train_depot_era_a.con"),
        at: pos(900_000, 450_000, 12_000),
    }
}

fn stops() -> BoundedVec<LineStop, 256> {
    list(vec![
        LineStop {
            station: StationId(4),
            terminal: Some(1),
        },
        LineStop {
            station: StationId(9),
            terminal: None,
        },
    ])
}

/// One of every variant, and of every inner variant.
fn samples() -> Vec<Action> {
    let color = Rgb {
        r: 200,
        g: 30,
        b: 0,
    };
    vec![
        Action::BuildRoad(RoadBuild {
            street: text("street/standard/town_medium_new.lua"),
            bus_lane: true,
            tram: Tram::Electric,
            polyline: polyline(),
        }),
        Action::BuildTrack(TrackBuild {
            track: text("high_speed.lua"),
            catenary: true,
            polyline: polyline(),
        }),
        Action::Bulldoze(Bulldoze::Edges {
            network: Network::Track,
            edges: list(vec![ends(pos(1, 2, 3), pos(4, 5, 6))]),
        }),
        Action::Bulldoze(Bulldoze::Construction(depot())),
        Action::Bulldoze(Bulldoze::EdgeObject {
            edge: EdgeRef {
                network: Network::Street,
                ends: ends(pos(10, 0, 0), pos(90_000, 0, 0)),
            },
            at: pos(45_000, 3_000, 0),
            model: text("station/street/bus_stop.mdl"),
        }),
        Action::BuildConstruction(ConstructionBuild {
            file: text("station/rail/modular_station/modular_station.con"),
            transform: Transform {
                basis: [0, 1_000_000, 0, -1_000_000, 0, 0, 0, 0, 1_000_000],
                origin: pos(500_000, 500_000, 20_000),
            },
            params: list(vec![
                Param {
                    key: text("seed"),
                    value: ParamValue::Int(-4_094_223_111),
                },
                Param {
                    key: text("modules[3801].name"),
                    value: ParamValue::Text(text("station/rail/modules/platform_track.module")),
                },
                Param {
                    key: text("modules[3801].variant"),
                    value: ParamValue::Fixed(2_500_000),
                },
                Param {
                    key: text("paramX"),
                    value: ParamValue::Bool(true),
                },
            ]),
            name: text("Hauptbahnhof"),
            replaces: Some(depot()),
        }),
        Action::BuyVehicle(BuyVehicle {
            depot: depot(),
            consist: list(vec![
                text("vehicle/train/br_101.mdl"),
                text("vehicle/waggon/ic_2nd.mdl"),
            ]),
        }),
        Action::SellVehicle {
            vehicles: list(vec![VehicleId(1), VehicleId(70_000)]),
        },
        Action::CreateLine(CreateLine {
            name: text("Line 1"),
            color,
            stops: stops(),
        }),
        Action::EditLine(EditLine {
            line: LineId(3),
            change: LineChange::Rename(text("Airport express")),
        }),
        Action::EditLine(EditLine {
            line: LineId(3),
            change: LineChange::Recolor(color),
        }),
        Action::EditLine(EditLine {
            line: LineId(3),
            change: LineChange::SetStops(stops()),
        }),
        Action::EditLine(EditLine {
            line: LineId(3),
            change: LineChange::Delete,
        }),
        Action::AssignLine(AssignLine {
            vehicles: list(vec![VehicleId(1)]),
            line: Some(LineId(3)),
            first_stop: 1,
        }),
        Action::AssignLine(AssignLine {
            vehicles: list(vec![VehicleId(1), VehicleId(2)]),
            line: None,
            first_stop: 0,
        }),
        Action::PlaceStop(PlaceStop {
            edge: EdgeRef {
                network: Network::Street,
                ends: ends(pos(10, 0, 0), pos(90_000, 0, 0)),
            },
            at: pos(45_000, 3_000, 0),
            left: true,
            direction: UnitDir {
                x: 1_000_000,
                y: 0,
                z: 0,
            },
            model: text("station/street/bus_stop.mdl"),
        }),
        Action::Terraform(
            Terraform::new(
                Pos2 {
                    x: -8_000,
                    y: 16_000,
                },
                4_000,
                2,
                list(vec![
                    TerrainCell {
                        target: 5_000,
                        before: 4_200,
                    },
                    TerrainCell {
                        target: 5_000,
                        before: 4_900,
                    },
                    TerrainCell {
                        target: 5_000,
                        before: 5_300,
                    },
                    TerrainCell {
                        target: 5_000,
                        before: -100,
                    },
                ]),
            )
            .unwrap(),
        ),
        Action::CompanyOp(CompanyOp::Create {
            name: text("Rail & Sons"),
        }),
        Action::CompanyOp(CompanyOp::Join(CompanyId(2))),
        Action::CompanyOp(CompanyOp::Rename {
            company: CompanyId(2),
            name: text("Rail & Daughters"),
        }),
        Action::CompanyOp(CompanyOp::Delete(CompanyId(2))),
    ]
}

/// Decodes `bytes` as an action payload: an error, or an action that
/// survives a round trip.
fn check(bytes: &[u8]) {
    let Ok(payload) = Payload::new(bytes.to_vec()) else {
        return;
    };
    if let Ok(action) = Action::from_payload(&payload) {
        let again = action.to_payload().unwrap();
        assert_eq!(Action::from_payload(&again).unwrap(), action);
    }
}

#[test]
fn every_variant_round_trips() {
    let samples = samples();
    // Every top-level variant is sampled: postcard tags them 0..=11.
    let mut tags: Vec<u8> = samples
        .iter()
        .map(|action| postcard::to_stdvec(action).unwrap()[0])
        .collect();
    tags.dedup();
    assert_eq!(tags, (0..=11).collect::<Vec<u8>>());

    for action in samples {
        let bytes = postcard::to_stdvec(&action).unwrap();
        assert_eq!(postcard::from_bytes::<Action>(&bytes).unwrap(), action);

        let payload = action.to_payload().unwrap();
        assert_eq!(payload.as_bytes()[0], ACTION_SCHEMA_VERSION as u8);
        assert_eq!(Action::from_payload(&payload).unwrap(), action);
        // An intent's payload is how an action travels; it must survive the
        // payload's own encoding too.
        let wire = postcard::to_stdvec(&payload).unwrap();
        let back: Payload = postcard::from_bytes(&wire).unwrap();
        assert_eq!(Action::from_payload(&back).unwrap(), action);
    }
}

#[test]
fn oversized_lists_are_refused() {
    // A polyline claiming one vertex past the limit, then nothing: refused
    // on the length, before any vertex is read.
    let mut bytes = vec![ACTION_SCHEMA_VERSION as u8, 1];
    bytes.extend(postcard::to_stdvec(&Text::<8>::new("t").unwrap()).unwrap());
    bytes.push(0);
    bytes.extend(postcard::to_stdvec(&u32::try_from(MAX_VERTICES + 1).unwrap()).unwrap());
    assert!(Action::from_payload(&Payload::new(bytes).unwrap()).is_err());

    // One edge past the bulldoze limit, every edge well-formed.
    let edges = vec![ends(pos(0, 0, 0), pos(1, 1, 1)); MAX_EDGES + 1];
    let mut bytes = vec![ACTION_SCHEMA_VERSION as u8, 2, 0, 1];
    bytes.extend(postcard::to_stdvec(&edges).unwrap());
    assert!(Action::from_payload(&Payload::new(bytes).unwrap()).is_err());
    assert!(BoundedVec::<EdgeEnds, MAX_EDGES>::new(edges).is_err());
}

#[test]
fn oversized_and_hostile_text_is_refused() {
    let mut bytes = vec![ACTION_SCHEMA_VERSION as u8, 11, 0];
    bytes.extend(postcard::to_stdvec(&"x".repeat(65)).unwrap());
    assert!(Action::from_payload(&Payload::new(bytes).unwrap()).is_err());

    let mut bytes = vec![ACTION_SCHEMA_VERSION as u8, 11, 0];
    bytes.extend(postcard::to_stdvec(&"a\u{1b}[2J").unwrap());
    assert!(Action::from_payload(&Payload::new(bytes).unwrap()).is_err());
}

#[test]
fn a_link_to_a_missing_vertex_is_refused() {
    // A track build laid out field by field as the schema encodes it, but
    // without the polyline's checks: its second link points past the last
    // vertex.
    #[derive(serde::Serialize)]
    struct UncheckedTrack {
        track: String,
        catenary: bool,
        vertices: Vec<Vertex>,
        links: Vec<Link>,
        removals: Vec<EdgeEnds>,
    }
    let good = polyline();
    let mut vertices = good.vertices.to_vec();
    vertices.pop();
    assert!(
        Polyline::new(
            list(vertices.clone()),
            good.links.clone(),
            BoundedVec::empty()
        )
        .is_err()
    );
    let track = UncheckedTrack {
        track: "high_speed.lua".into(),
        catenary: false,
        vertices,
        links: good.links.to_vec(),
        removals: Vec::new(),
    };
    // The schema version, then Action::BuildTrack.
    let bytes = postcard::to_stdvec(&(ACTION_SCHEMA_VERSION, 1u32, &track)).unwrap();
    assert!(Action::from_payload(&Payload::new(bytes).unwrap()).is_err());

    // The same bytes with every vertex present decode.
    let mut track = track;
    track.vertices = good.vertices.to_vec();
    let bytes = postcard::to_stdvec(&(ACTION_SCHEMA_VERSION, 1u32, &track)).unwrap();
    assert!(Action::from_payload(&Payload::new(bytes).unwrap()).is_ok());
}

#[test]
fn the_payload_limit_holds() {
    let bytes = vec![ACTION_SCHEMA_VERSION as u8; MAX_PAYLOAD + 1];
    assert!(Payload::new(bytes).is_err());
    // The largest terraform the schema allows is refused by the payload, not
    // truncated.
    let cells = vec![
        TerrainCell {
            target: i32::MAX,
            before: i32::MIN,
        };
        8192
    ];
    let action =
        Action::Terraform(Terraform::new(Pos2 { x: 0, y: 0 }, 4_000, 64, list(cells)).unwrap());
    assert!(action.to_payload().is_err());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn corrupted_actions_decode_or_fail_cleanly(
        pick in any::<Index>(),
        edits in vec((any::<Index>(), any::<u8>(), 0u8..4), 1..8),
    ) {
        let samples = samples();
        let sample = &samples[pick.index(samples.len())];
        let mut bytes = sample.to_payload().unwrap().as_bytes().to_vec();
        for (at, value, kind) in edits {
            match kind {
                0 | 1 if !bytes.is_empty() => {
                    let index = at.index(bytes.len());
                    bytes[index] = value;
                }
                2 => bytes.insert(at.index(bytes.len() + 1), value),
                3 if !bytes.is_empty() => bytes.truncate(at.index(bytes.len())),
                _ => {}
            }
        }
        check(&bytes);
    }

    #[test]
    fn arbitrary_bytes_decode_or_fail_cleanly(bytes in vec(any::<u8>(), 0..300)) {
        check(&bytes);
        let mut versioned = vec![ACTION_SCHEMA_VERSION as u8];
        versioned.extend(bytes);
        check(&versioned);
    }
}
