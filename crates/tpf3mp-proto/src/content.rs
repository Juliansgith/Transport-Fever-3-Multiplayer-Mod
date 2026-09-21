//! What a player's game runs, and how two games differ.
//!
//! Every player declares a [`ContentManifest`]: the game's build and its
//! active mods in load order. Its [`ContentFingerprint`] is what rooms
//! compare, and when two differ, [`ContentManifest::compare`] says how, so
//! players learn which mods to add, remove or update instead of only that
//! something differs.

use std::{collections::HashMap, fmt};

use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};

use crate::{ContentFingerprint, FixedBytes, Text};

/// The name a mod goes by, as the game lists it.
pub type ModId = Text<96>;
/// A mod's version, as the game or its author states it.
pub type ModVersion = Text<32>;

/// Most mods a manifest lists by name; [`Unlisted`] summarises the rest.
pub const MAX_LISTED_MODS: usize = 2048;
/// Largest encoded manifest, so that declaring one fits a control frame.
pub const MAX_MANIFEST_BYTES: usize = 48 * 1024;
/// Most mods a [`ContentDiff`] names of each kind; its totals count all.
pub const MAX_DIFF_LISTED: usize = 32;

/// Keeps content fingerprints apart from every other SHA-256 in the
/// project, and names their format.
const FINGERPRINT_DOMAIN: &[u8] = b"tpf3mp content 1\0";

/// One mod a game runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModRef {
    pub id: ModId,
    pub version: ModVersion,
}

/// The mods after the listed ones, when a game runs more than a manifest
/// lists: how many, and a digest of their list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unlisted {
    pub count: u32,
    pub digest: FixedBytes<32>,
}

/// What a player's game runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentManifest {
    /// The game's build, as the game names it.
    pub game: Text<64>,
    /// The active mods in load order, as many as fit.
    pub mods: Vec<ModRef>,
    /// The mods after `mods`, if there are more than fit.
    pub unlisted: Option<Unlisted>,
}

impl ContentManifest {
    /// `game` running `mods` in load order. Lists as many mods as fit in a
    /// manifest and summarises the rest, so the same game and mods always
    /// give the same manifest.
    pub fn new(game: Text<64>, mods: Vec<ModRef>) -> Self {
        let mut manifest = Self {
            game,
            mods: Vec::new(),
            unlisted: None,
        };
        let mut size = encoded_len(&manifest);
        let mut mods = mods.into_iter();
        for listed in mods.by_ref() {
            // Room for the summary of what does not fit, and for the list's
            // length growing by a byte or two.
            let grown = size + encoded_len(&listed) + 48;
            if manifest.mods.len() == MAX_LISTED_MODS || grown > MAX_MANIFEST_BYTES {
                let rest: Vec<ModRef> = std::iter::once(listed).chain(mods).collect();
                let mut digest = Context::new(&SHA256);
                digest.update(&postcard::to_allocvec(&rest).unwrap_or_default());
                manifest.unlisted = Some(Unlisted {
                    count: u32::try_from(rest.len()).unwrap_or(u32::MAX),
                    digest: FixedBytes(digest.finish().as_ref().try_into().unwrap_or([0; 32])),
                });
                break;
            }
            size += encoded_len(&listed);
            manifest.mods.push(listed);
        }
        manifest
    }

    /// Whether the manifest is within the limits one declaration may take.
    pub fn is_valid(&self) -> bool {
        self.mods.len() <= MAX_LISTED_MODS && encoded_len(self) <= MAX_MANIFEST_BYTES
    }

    /// What rooms compare: equal only for the same build and the same mods
    /// in the same order.
    pub fn fingerprint(&self) -> ContentFingerprint {
        let mut digest = Context::new(&SHA256);
        digest.update(FINGERPRINT_DOMAIN);
        digest.update(&postcard::to_allocvec(self).unwrap_or_default());
        ContentFingerprint(FixedBytes(
            digest.finish().as_ref().try_into().unwrap_or([0; 32]),
        ))
    }

    /// How `yours` differs from this manifest, the room's, or `None` if
    /// they are the same.
    pub fn compare(&self, yours: &Self) -> Option<ContentDiff> {
        if self == yours {
            return None;
        }
        let room: HashMap<&str, &ModRef> = first_by_id(&self.mods);
        let mine: HashMap<&str, &ModRef> = first_by_id(&yours.mods);
        let mut diff = ContentDiff {
            game: (self.game != yours.game).then(|| GameBuilds {
                room: self.game.clone(),
                yours: yours.game.clone(),
            }),
            ..ContentDiff::default()
        };
        for listed in &self.mods {
            match mine.get(listed.id.as_str()) {
                None => push(&mut diff.missing, &mut diff.missing_total, listed.clone()),
                Some(own) if own.version != listed.version => push(
                    &mut diff.changed,
                    &mut diff.changed_total,
                    ModChange {
                        id: listed.id.clone(),
                        room: listed.version.clone(),
                        yours: own.version.clone(),
                    },
                ),
                Some(_) => {}
            }
        }
        for own in &yours.mods {
            if !room.contains_key(own.id.as_str()) {
                push(&mut diff.extra, &mut diff.extra_total, own.clone());
            }
        }
        diff.unlisted = self.unlisted != yours.unlisted;
        let same_mods = diff.missing_total == 0 && diff.extra_total == 0 && diff.changed_total == 0;
        diff.reordered = same_mods && self.mods != yours.mods;
        Some(diff)
    }
}

fn encoded_len<T: Serialize>(value: &T) -> usize {
    postcard::to_allocvec(value).map_or(usize::MAX, |bytes| bytes.len())
}

/// The first mod of each id: a list that names one twice still compares.
fn first_by_id(mods: &[ModRef]) -> HashMap<&str, &ModRef> {
    let mut by_id = HashMap::with_capacity(mods.len());
    for listed in mods {
        by_id.entry(listed.id.as_str()).or_insert(listed);
    }
    by_id
}

fn push<T>(list: &mut Vec<T>, total: &mut u32, item: T) {
    if list.len() < MAX_DIFF_LISTED {
        list.push(item);
    }
    *total = total.saturating_add(1);
}

/// Two builds of the game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameBuilds {
    pub room: Text<64>,
    pub yours: Text<64>,
}

/// A mod both games run, in different versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModChange {
    pub id: ModId,
    pub room: ModVersion,
    pub yours: ModVersion,
}

/// How a player's game differs from the room's: the owner's in the lobby,
/// the game's once it runs. Lists name the first [`MAX_DIFF_LISTED`] of
/// each kind, in load order; the totals count them all.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDiff {
    /// The builds, when they differ.
    pub game: Option<GameBuilds>,
    /// Mods the room runs and this player does not.
    pub missing: Vec<ModRef>,
    pub missing_total: u32,
    /// Mods this player runs and the room does not.
    pub extra: Vec<ModRef>,
    pub extra_total: u32,
    /// Mods both run in different versions.
    pub changed: Vec<ModChange>,
    pub changed_total: u32,
    /// The same mods, loaded in another order.
    pub reordered: bool,
    /// The mods after the listed ones differ: one of the games runs more
    /// mods than a manifest names.
    pub unlisted: bool,
}

impl fmt::Display for ContentDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(builds) = &self.game {
            parts.push(format!(
                "the room runs game build {}, you run {}",
                builds.room, builds.yours
            ));
        }
        let named = |mods: &[ModRef], total: u32| {
            let mut text = mods
                .iter()
                .map(|listed| format!("{} {}", listed.id, listed.version))
                .collect::<Vec<_>>()
                .join(", ");
            let more = total.saturating_sub(u32::try_from(mods.len()).unwrap_or(u32::MAX));
            if more > 0 {
                text.push_str(&format!(" and {more} more"));
            }
            text
        };
        if self.missing_total > 0 {
            parts.push(format!(
                "you lack {}",
                named(&self.missing, self.missing_total)
            ));
        }
        if self.extra_total > 0 {
            parts.push(format!(
                "the room lacks {}",
                named(&self.extra, self.extra_total)
            ));
        }
        if self.changed_total > 0 {
            let mut text = self
                .changed
                .iter()
                .map(|change| {
                    format!(
                        "{} (the room has {}, you have {})",
                        change.id, change.room, change.yours
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let more = self
                .changed_total
                .saturating_sub(u32::try_from(self.changed.len()).unwrap_or(u32::MAX));
            if more > 0 {
                text.push_str(&format!(" and {more} more"));
            }
            parts.push(format!("other versions: {text}"));
        }
        if self.reordered {
            parts.push("the same mods load in another order".to_owned());
        }
        if self.unlisted {
            parts.push("the mods beyond the listed ones differ".to_owned());
        }
        if parts.is_empty() {
            f.write_str("your game differs from the room's")
        } else {
            f.write_str(&parts.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(id: &str, version: &str) -> ModRef {
        ModRef {
            id: Text::new(id).unwrap(),
            version: Text::new(version).unwrap(),
        }
    }

    fn game(build: &str, mods: &[(&str, &str)]) -> ContentManifest {
        ContentManifest::new(
            Text::new(build).unwrap(),
            mods.iter()
                .map(|(id, version)| listed(id, version))
                .collect(),
        )
    }

    #[test]
    fn the_same_game_matches() {
        let room = game("35924", &[("trains", "1.2"), ("stations", "3")]);
        let yours = game("35924", &[("trains", "1.2"), ("stations", "3")]);
        assert_eq!(room.fingerprint(), yours.fingerprint());
        assert_eq!(room.compare(&yours), None);
    }

    #[test]
    fn differences_name_the_mods_to_add_remove_or_update() {
        let room = game(
            "35924",
            &[("trains", "1.2"), ("stations", "3"), ("maps", "1")],
        );
        let yours = game("35925", &[("trains", "1.1"), ("maps", "1"), ("trees", "2")]);
        assert_ne!(room.fingerprint(), yours.fingerprint());
        let diff = room.compare(&yours).unwrap();
        assert_eq!(
            diff.game,
            Some(GameBuilds {
                room: Text::new("35924").unwrap(),
                yours: Text::new("35925").unwrap(),
            })
        );
        assert_eq!(diff.missing, [listed("stations", "3")]);
        assert_eq!(diff.extra, [listed("trees", "2")]);
        assert_eq!(
            diff.changed,
            [ModChange {
                id: Text::new("trains").unwrap(),
                room: Text::new("1.2").unwrap(),
                yours: Text::new("1.1").unwrap(),
            }]
        );
        assert!(!diff.reordered);
        assert_eq!(
            diff.to_string(),
            "the room runs game build 35924, you run 35925; you lack stations 3; \
             the room lacks trees 2; other versions: trains (the room has 1.2, you have 1.1)"
        );
    }

    #[test]
    fn load_order_counts() {
        let room = game("35924", &[("trains", "1"), ("stations", "1")]);
        let yours = game("35924", &[("stations", "1"), ("trains", "1")]);
        assert_ne!(room.fingerprint(), yours.fingerprint());
        let diff = room.compare(&yours).unwrap();
        assert!(diff.reordered);
        assert_eq!(diff.to_string(), "the same mods load in another order");
    }

    #[test]
    fn long_lists_name_the_first_and_count_the_rest() {
        let many: Vec<(String, String)> = (0..100)
            .map(|n| (format!("mod{n}"), "1".to_owned()))
            .collect();
        let many: Vec<(&str, &str)> = many.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let room = game("35924", &many);
        let yours = game("35924", &[]);
        let diff = room.compare(&yours).unwrap();
        assert_eq!(diff.missing.len(), MAX_DIFF_LISTED);
        assert_eq!(diff.missing_total, 100);
        assert!(diff.to_string().ends_with("mod31 1 and 68 more"));
    }

    #[test]
    fn a_huge_mod_list_is_summarised_and_still_told_apart() {
        let many: Vec<ModRef> = (0..3000)
            .map(|n| {
                listed(
                    &format!("workshop_{n:010}_a_rather_long_mod_folder_name"),
                    "1.0",
                )
            })
            .collect();
        let room = ContentManifest::new(Text::new("35924").unwrap(), many.clone());
        assert!(room.is_valid());
        assert!(room.mods.len() < many.len());
        let unlisted = room.unlisted.unwrap();
        assert_eq!(room.mods.len() + unlisted.count as usize, 3000);
        // The same list gives the same manifest.
        assert_eq!(
            ContentManifest::new(Text::new("35924").unwrap(), many.clone()),
            room
        );
        // A difference past the listed mods still changes the fingerprint.
        let mut other = many;
        other[2999] = listed("something_else", "1.0");
        let yours = ContentManifest::new(Text::new("35924").unwrap(), other);
        assert_ne!(room.fingerprint(), yours.fingerprint());
        let diff = room.compare(&yours).unwrap();
        assert!(diff.unlisted);
        assert_eq!(
            diff.missing_total + diff.extra_total + diff.changed_total,
            0
        );
    }

    #[test]
    fn an_oversized_manifest_is_invalid() {
        let mods: Vec<ModRef> = (0..MAX_LISTED_MODS + 1)
            .map(|n| listed(&format!("m{n}"), "1"))
            .collect();
        let manifest = ContentManifest {
            game: Text::new("35924").unwrap(),
            mods,
            unlisted: None,
        };
        assert!(!manifest.is_valid());
    }
}
