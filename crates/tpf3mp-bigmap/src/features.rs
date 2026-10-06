//! Which big-map features a build can run.
//!
//! Each feature names the hook-profile targets it patches (docs/HOOKS.md:
//! found per build, byte-verified, resolved in the running game's memory).
//! The names are roles, not TF3 symbols, which nobody has read yet; each
//! notes the TPF2 function Big Maps patched for it. A build's profile maps
//! a role to TF3's function once it is found.
//!
//! As in Big Maps, a feature whose sites are missing on a build does not
//! stop the rest. But a room cannot degrade the way a single player can: a
//! feature that changes the simulation must run on every game in the room
//! or none, so if the room's settings want one and this build cannot run it,
//! the game refuses the room instead of joining without it. A feature that
//! only changes the menu or the screen is left off with a reason.

use crate::config::{Config, STOCK_PLACEMENT_ATTEMPTS};

/// What a feature changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Only what one player sees: may be missing on some games.
    Ui,
    /// How the world runs: every game in the room, or none.
    Simulation,
}

/// A big-map feature and the sites it patches.
#[derive(Debug, Clone, Copy)]
pub struct Feature {
    pub name: &'static str,
    pub effect: Effect,
    /// The profile targets it needs, all of them.
    pub targets: &'static [&'static str],
    /// Whether the settings ask for it.
    pub wanted: fn(&Config) -> bool,
}

/// Big Maps' features, as TF3 would carry them.
pub const FEATURES: [Feature; 8] = [
    Feature {
        // TPF2: UI::GetNumTilesNew (MenuUI.cpp:264) and the size dropdown's
        // fill. TF3's New Game page is a script: the mod's copy of it
        // (mod/tpf3mp_bigmap_1, crate::page), served by the hook's loader
        // redirect, adds the rows, so no native target.
        name: "size_rows",
        effect: Effect::Ui,
        targets: &[],
        wanted: |config| config.sizes.add_rows,
    },
    Feature {
        // TPF2: the street raster's size, RVA 0x90d410. TF3: the Obstacle
        // constructor, detoured by the hook (bigmap/raster.rs).
        name: "street_raster",
        effect: Effect::Simulation,
        targets: &["bigmap::Obstacle::Obstacle"],
        wanted: |config| config.limits.street_raster,
    },
    Feature {
        // TPF2: the spacing score, RVA 0x910ce0. TF3: 0x8d31f0, replaced by
        // the hook (bigmap/placement.rs).
        name: "placement_distance",
        effect: Effect::Simulation,
        targets: &["bigmap::placement spacing"],
        wanted: |config| config.limits.placement_distance,
    },
    Feature {
        // TPF2: ecs::OctreeSystem's root box, a two-tier constant. TF3:
        // OctreeSystem::Resize and its two sites, spliced by the hook
        // (crates/tpf3mp-hook/src/bigmap/octree.rs).
        name: "octree_depth",
        effect: Effect::Simulation,
        targets: &[
            "bigmap::OctreeSystem::Resize",
            "bigmap::octree_root_init",
            "bigmap::octree_root_load",
        ],
        wanted: |config| config.limits.octree_depth != 0,
    },
    Feature {
        // TPF2: RandomLocationFactory's inner search, 200 attempts. Runtime
        // industry founding takes the same path.
        name: "placement_attempts",
        effect: Effect::Simulation,
        targets: &["bigmap::placement_attempts"],
        wanted: |config| config.generation.placement_attempts != STOCK_PLACEMENT_ATTEMPTS,
    },
    Feature {
        // TPF2: the New Game parameters' refresh and the industry
        // multipliers. A save made with an added level needs it to load.
        name: "density_levels",
        effect: Effect::Simulation,
        targets: &["bigmap::density_levels"],
        wanted: |config| config.generation.density_levels,
    },
    Feature {
        // A script mod on TF3: raw pixels into an ImageView, no native
        // part (docs/MINIMAP.md).
        name: "minimap",
        effect: Effect::Ui,
        targets: &[],
        wanted: |config| config.ui.minimap,
    },
    Feature {
        // A proposal with no TPF2 counterpart: the emission grid's update
        // and the emitters' splat on every Nth update, by the saved update
        // counter. TF3: two vtable slots (crates/tpf3mp-hook/src/bigmap/
        // emission.rs).
        name: "emission_every",
        effect: Effect::Simulation,
        targets: &[
            "bigmap::EmissionGridSystem::Update",
            "bigmap::EmissionGridSystem vtable load",
            "bigmap::EmissionEmitterSystem::Update2",
            "bigmap::EmissionEmitterSystem vtable load",
        ],
        wanted: |config| config.simulation.emission_every != 1,
    },
];

/// What becomes of a feature on this build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// The settings do not ask for it.
    Off,
    On,
    /// Wanted, only what a player sees, and its sites are missing: off,
    /// with the reason.
    Degraded {
        missing: Vec<&'static str>,
    },
    /// Wanted, it changes the simulation, and its sites are missing: the
    /// game must not play these settings.
    Refused {
        missing: Vec<&'static str>,
    },
}

/// Every feature's state on a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub features: Vec<(&'static str, State)>,
}

impl Plan {
    /// Why this build cannot play these settings, if it cannot.
    pub fn refusal(&self) -> Option<String> {
        let refused: Vec<String> = self
            .features
            .iter()
            .filter_map(|(name, state)| match state {
                State::Refused { missing } => {
                    Some(format!("{name} (missing {})", missing.join(", ")))
                }
                _ => None,
            })
            .collect();
        (!refused.is_empty()).then(|| {
            format!(
                "this game build cannot run the big-map settings the world needs: {}",
                refused.join("; ")
            )
        })
    }

    pub fn state(&self, name: &str) -> Option<&State> {
        self.features
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, state)| state)
    }
}

/// Plans `config`'s features on a build whose profile resolved the targets
/// `resolved` answers yes for: with the hook, `|name|
/// profile.get(name).is_some()` on its `ResolvedProfile`.
pub fn plan(config: &Config, resolved: impl Fn(&str) -> bool) -> Plan {
    plan_features(&FEATURES, config, resolved)
}

/// [`plan`] over `features` instead of [`FEATURES`].
pub fn plan_features(
    features: &[Feature],
    config: &Config,
    resolved: impl Fn(&str) -> bool,
) -> Plan {
    let features = features
        .iter()
        .map(|feature| {
            let state = if !(feature.wanted)(config) {
                State::Off
            } else {
                let missing: Vec<&'static str> = feature
                    .targets
                    .iter()
                    .copied()
                    .filter(|target| !resolved(target))
                    .collect();
                match (missing.is_empty(), feature.effect) {
                    (true, _) => State::On,
                    (false, Effect::Ui) => State::Degraded { missing },
                    (false, Effect::Simulation) => State::Refused { missing },
                }
            };
            (feature.name, state)
        })
        .collect();
    Plan { features }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Config {
        Config::from_toml(text).unwrap().0
    }

    #[test]
    fn nothing_wanted_is_nothing_needed() {
        let plan = plan(&Config::default(), |_| false);
        assert!(plan.features.iter().all(|(_, state)| *state == State::Off));
        assert_eq!(plan.refusal(), None);
    }

    #[test]
    fn a_missing_screen_feature_degrades() {
        // A screen feature with native sites, as size_rows was on TPF2.
        const NATIVE_ROWS: Feature = Feature {
            name: "native_rows",
            effect: Effect::Ui,
            targets: &["bigmap::tile_count", "bigmap::size_list"],
            wanted: |config| config.sizes.add_rows,
        };
        let settings = config("[sizes]\nadd_rows = true\n[ui]\nminimap = true");
        let native = plan_features(&[NATIVE_ROWS], &settings, |target| {
            target == "bigmap::tile_count"
        });
        assert_eq!(
            native.state("native_rows"),
            Some(&State::Degraded {
                missing: vec!["bigmap::size_list"]
            })
        );
        assert_eq!(native.refusal(), None);
        let plan = plan(&settings, |_| false);
        assert_eq!(
            plan.state("size_rows"),
            Some(&State::On),
            "TF3's rows are the mod's page: no native site"
        );
        assert_eq!(
            plan.state("minimap"),
            Some(&State::On),
            "the minimap needs no target"
        );
        assert_eq!(
            plan.refusal(),
            None,
            "a player may play without the added rows"
        );
    }

    #[test]
    fn a_missing_simulation_feature_refuses_the_room() {
        let settings = config("[limits]\nstreet_raster = true\noctree_depth = 11");
        let plan = plan(&settings, |target| target == "bigmap::Obstacle::Obstacle");
        assert_eq!(plan.state("street_raster"), Some(&State::On));
        assert_eq!(
            plan.state("octree_depth"),
            Some(&State::Refused {
                missing: vec![
                    "bigmap::OctreeSystem::Resize",
                    "bigmap::octree_root_init",
                    "bigmap::octree_root_load"
                ]
            })
        );
        let refusal = plan.refusal().unwrap();
        assert!(
            refusal.contains("octree_depth (missing bigmap::OctreeSystem::Resize, "),
            "{refusal}"
        );
        assert_eq!(plan_all(&settings).refusal(), None);
        let throttled = config("[simulation]\nemission_every = 4");
        let throttled = super::plan(&throttled, |target| !target.contains("Emitter"));
        assert!(matches!(
            throttled.state("emission_every"),
            Some(State::Refused { missing }) if missing.len() == 2
        ));
    }

    fn plan_all(config: &Config) -> Plan {
        plan(config, |_| true)
    }

    #[test]
    fn the_native_targets_are_the_tf3_profiles() {
        let profile = include_str!("../../../profiles/tf3_build40408_steam_windows/hooks.toml");
        for target in FEATURES
            .iter()
            .filter(|f| {
                matches!(
                    f.name,
                    "octree_depth" | "street_raster" | "placement_distance" | "emission_every"
                )
            })
            .flat_map(|f| f.targets)
        {
            assert!(
                profile.contains(&format!("name = \"{target}\"")),
                "{target} is not in the TF3 profile"
            );
        }
    }

    #[test]
    fn every_simulation_feature_names_its_sites() {
        for feature in FEATURES {
            if feature.effect == Effect::Simulation {
                assert!(!feature.targets.is_empty(), "{}", feature.name);
            }
        }
    }
}
