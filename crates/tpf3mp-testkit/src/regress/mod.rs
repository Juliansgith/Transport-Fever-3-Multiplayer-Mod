//! The regression harness: scripted play-throughs of the portable action
//! schema, in rooms of two or more games, judged by checks every replica
//! makes at the same step and by comparing the replicas' worlds. It plays
//! the model game ([`model`]) today; the real game's hook plugs into the
//! same script once it applies actions (`docs/REGRESSION.md`).

pub mod library;
pub mod model;
pub mod offline;
pub mod replica;
pub mod run;
pub mod script;
