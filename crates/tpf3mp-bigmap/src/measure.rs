//! Stage 0: what a game log says about generating, entering and saving a
//! map (investigation/TF3_BIGMAPS_PORT_2026-10-01.md, section 4).
//!
//! The game prints its terrain toolkit's use at the end of generation,
//!
//! ```text
//! [... - Main ]+ Terrain toolkit used 49 maps and 10074 MB...
//!     +   Pipeline took: 19.44983s
//! ```
//!
//! and silver2127's tpf2-bigmap found the law behind the number on TPF2:
//! `MB = (64 x + 1)(64 y + 1) * maps * 4 / 10^6`, truncated, one
//! full-resolution float map per name, all alive at once. This reads those
//! lines, the stage timings and the saves from a log, and checks the law
//! against the map's size: given, or found among the game's own sizes. It
//! reads only what the person running the measurement hands it; nothing
//! here launches or touches the game.

use crate::stock::{StockSize, TF3_BUILD_40408};
use crate::world::WorldModel;

/// The stage timings the game prints in milliseconds, by their log names.
pub const STAGES_MS: [&str; 5] = [
    "Place assets",
    "Create Industries",
    "InitGame",
    "Enter Game Asynchronously",
    "Initial material index generation",
];

/// One "Terrain toolkit used" line, and the pipeline time after it.
#[derive(Debug, Clone, PartialEq)]
pub struct Toolkit {
    pub maps: u32,
    pub mb: u64,
    pub pipeline_s: Option<f64>,
}

/// One save: the "Savegame info" size and the time it took.
#[derive(Debug, Clone, PartialEq)]
pub struct Save {
    pub bytes: u64,
    pub took_ms: Option<u64>,
}

/// What one log holds, in the order the game printed it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Measurements {
    pub toolkits: Vec<Toolkit>,
    /// (stage, milliseconds); "Init game took" is given in seconds and
    /// kept here in milliseconds too.
    pub stages: Vec<(String, f64)>,
    pub saves: Vec<Save>,
}

/// The toolkit's memory by the law, in decimal MB, truncated.
pub fn toolkit_mb(tiles_x: u32, tiles_y: u32, maps: u32) -> u64 {
    let samples = (64 * u64::from(tiles_x) + 1) * (64 * u64::from(tiles_y) + 1);
    samples * u64::from(maps) * 4 / 1_000_000
}

/// The game's own sizes whose toolkit line would read `maps` and `mb`: the
/// size's name, the ratio's k (1:k) and the shape.
pub fn stock_matches(maps: u32, mb: u64) -> Vec<(&'static str, u32, (u32, u32))> {
    let mut found = Vec::new();
    for StockSize { name, shapes } in TF3_BUILD_40408 {
        for (k, (x, y)) in (1u32..).zip(shapes) {
            if toolkit_mb(x, y, maps) == mb && !found.iter().any(|(_, _, s)| *s == (x, y)) {
                found.push((name, k, (x, y)));
            }
        }
    }
    found
}

/// The first number in `text`, as written (digits, one dot).
fn number(text: &str) -> Option<&str> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

fn after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    line.find(marker).map(|at| &line[at + marker.len()..])
}

/// Reads `log`, a game log (`stdout.txt` or a `crash_dump/*.txt`).
pub fn parse(log: &str) -> Measurements {
    let mut out = Measurements::default();
    for line in log.lines() {
        if let Some(rest) = after(line, "Terrain toolkit used ") {
            let maps = number(rest).and_then(|n| n.parse().ok());
            let mb = after(rest, " maps and ")
                .and_then(number)
                .and_then(|n| n.parse().ok());
            if let (Some(maps), Some(mb)) = (maps, mb) {
                out.toolkits.push(Toolkit {
                    maps,
                    mb,
                    pipeline_s: None,
                });
            }
        } else if let Some(rest) = after(line, "Pipeline took: ") {
            let seconds = number(rest).and_then(|n| n.parse().ok());
            if let Some(last) = out.toolkits.last_mut().filter(|t| t.pipeline_s.is_none()) {
                last.pipeline_s = seconds;
            }
        } else if let Some(rest) = after(line, "Init game took: ") {
            if let Some(seconds) = number(rest).and_then(|n| n.parse::<f64>().ok()) {
                out.stages.push(("Init game".to_owned(), seconds * 1000.0));
            }
        } else if let Some(rest) = after(line, "Savegame info: ") {
            if let Some(bytes) = after(rest, "size = ")
                .and_then(number)
                .and_then(|n| n.parse().ok())
            {
                out.saves.push(Save {
                    bytes,
                    took_ms: None,
                });
            }
        } else if let Some(rest) = after(line, "Saving game complete, took ") {
            let ms = number(rest).and_then(|n| n.parse().ok());
            if let Some(last) = out.saves.last_mut().filter(|s| s.took_ms.is_none()) {
                last.took_ms = ms;
            }
        } else {
            for stage in STAGES_MS {
                let marker = format!("]  {stage}: ");
                if let Some(ms) = after(line, &marker)
                    .filter(|rest| rest.trim_end().ends_with(" ms"))
                    .and_then(number)
                    .and_then(|n| n.parse::<f64>().ok())
                {
                    out.stages.push((stage.to_owned(), ms));
                }
            }
        }
    }
    out
}

/// A report of `measured`, for docs/BIGMAPS.md's stage 0 table. `tiles`
/// is the map's size when the person measuring knows it.
pub fn report(world: &WorldModel, measured: &Measurements, tiles: Option<(u32, u32)>) -> String {
    let mut out = String::new();
    if measured.toolkits.is_empty() {
        out.push_str("no \"Terrain toolkit used\" line: this log generated no map\n");
    }
    for toolkit in &measured.toolkits {
        let pipeline = toolkit
            .pipeline_s
            .map_or_else(|| "?".to_owned(), |s| format!("{s:.1} s"));
        out.push_str(&format!(
            "terrain toolkit: {} maps, {} MB, pipeline {pipeline}\n",
            toolkit.maps, toolkit.mb
        ));
        let sizes: Vec<(String, (u32, u32))> = match tiles {
            Some(size) => vec![("given".to_owned(), size)],
            None => stock_matches(toolkit.maps, toolkit.mb)
                .into_iter()
                .map(|(name, k, size)| (format!("{name} 1:{k}"), size))
                .collect(),
        };
        if sizes.is_empty() {
            out.push_str("  no stock size matches the law; give the size with --tiles\n");
        }
        for (name, (x, y)) in sizes {
            let law = toolkit_mb(x, y, toolkit.maps);
            let km2 = world.area_km2(x, y);
            let verdict = if law == toolkit.mb {
                "the law holds".to_owned()
            } else {
                format!("the law says {law} MB")
            };
            out.push_str(&format!(
                "  {name} {x} x {y}: {km2:.0} km², {:.2} MB per km², {verdict}\n",
                toolkit.mb as f64 / km2
            ));
        }
    }
    for (stage, ms) in &measured.stages {
        out.push_str(&format!("{stage}: {:.1} s\n", ms / 1000.0));
    }
    for save in &measured.saves {
        let took = save.took_ms.map_or_else(
            || "?".to_owned(),
            |ms| format!("{:.1} s", ms as f64 / 1000.0),
        );
        out.push_str(&format!(
            "save: {:.0} MB in {took}\n",
            save.bytes as f64 / 1e6
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines shaped as the game prints them; the values are made up.
    const LOG: &str = "\
[2026-10-02 10:00:00Z - MESSAGE  -                  - Main           ]  Place assets: 412.5 ms
[2026-10-02 10:00:00Z - MESSAGE  -                  - Main           ]+ Terrain toolkit used 49 maps and 10074 MB...
    +   Pipeline took: 21.5s
[2026-10-02 10:00:01Z - MESSAGE  -                  - Main           ]  Create Industries: 1200.25 ms
[2026-10-02 10:01:00Z - MESSAGE  - Load Game Pool 2 - Main           ]  InitGame: 90000.5 ms
[2026-10-02 10:02:00Z - MESSAGE  - Main Thread      - Main           ]  Init game took: 200.5 s
[2026-10-02 10:03:00Z - MESSAGE  - Simulation Threa - Saving         ]  Savegame info: version = 604, size = 250000000, time = x
[2026-10-02 10:03:00Z - MESSAGE  - Simulation Threa - Saving         ]  Saving game complete, took 5000 ms
";

    #[test]
    fn the_log_lines_are_read() {
        let measured = parse(LOG);
        assert_eq!(
            measured.toolkits,
            [Toolkit {
                maps: 49,
                mb: 10_074,
                pipeline_s: Some(21.5)
            }]
        );
        let stages: Vec<&str> = measured.stages.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(
            stages,
            ["Place assets", "Create Industries", "InitGame", "Init game"]
        );
        assert_eq!(measured.stages[3].1, 200_500.0);
        assert_eq!(
            measured.saves,
            [Save {
                bytes: 250_000_000,
                took_ms: Some(5000)
            }]
        );
    }

    #[test]
    fn the_law_finds_the_size() {
        // The investigation's one log: 49 maps and 10,074 MB is exactly
        // Gigantomaniac 1:4, 56 x 224; 112 x 112 would read 10,073.
        assert_eq!(toolkit_mb(56, 224, 49), 10_074);
        assert_eq!(toolkit_mb(112, 112, 49), 10_073);
        assert_eq!(stock_matches(49, 10_074), [("colossal", 4, (56, 224))]);
        // TPF2's desert from tpf2-bigmap's docs: 18 maps at 256 tiles,
        // "19.3 GB".
        assert_eq!(toolkit_mb(256, 256, 18), 19_329);
    }

    #[test]
    fn the_report_names_the_size_and_its_cost() {
        let world = WorldModel::TF3_BUILD_40408;
        let text = report(&world, &parse(LOG), None);
        assert!(
            text.contains("49 maps, 10074 MB, pipeline 21.5 s"),
            "{text}"
        );
        assert!(text.contains("colossal 1:4 56 x 224: 822 km²"), "{text}");
        assert!(text.contains("the law holds"), "{text}");
        assert!(text.contains("save: 250 MB in 5.0 s"), "{text}");
        let given = report(&world, &parse(LOG), Some((112, 112)));
        assert!(given.contains("the law says 10073 MB"), "{given}");
        assert!(report(&world, &Measurements::default(), None).contains("generated no map"));
    }
}
