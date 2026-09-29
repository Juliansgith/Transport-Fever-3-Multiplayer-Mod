//! `tpf3mp-bigmap`: what a big map would cost and need, before generating it.
//!
//! ```text
//! tpf3mp-bigmap ladder [--config tpf3mp_bigmap.toml]   the added sizes and their shapes
//! tpf3mp-bigmap check 320x320 [--config ...]           one size: cost, ceilings, terms
//! tpf3mp-bigmap lua [--config ...]                     the ladder as the mod's data file
//! ```
//!
//! Numbers are TPF2 build 35924's (`WorldModel::TPF2_BUILD_35924`) until
//! TF3's are measured.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tpf3mp_bigmap::ceilings::{SizeReport, check};
use tpf3mp_bigmap::config::Config;
use tpf3mp_bigmap::ladder::{RATIOS, shapes};
use tpf3mp_bigmap::terms::Terms;
use tpf3mp_bigmap::world::WorldModel;

#[derive(Parser)]
#[command(about = "What a big map would cost and need, before generating it")]
struct Args {
    /// The settings file; every feature off without one.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// The added sizes, each shape, and whether the settings can build it.
    Ladder,
    /// One size, as <tiles x>x<tiles y>: its cost, the ceilings it hits and
    /// the room terms it makes.
    Check { size: String },
    /// The ladder as a Lua data file for mod/tpf3mp_bigmap_1.
    Lua,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("tpf3mp-bigmap: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<ExitCode, String> {
    let world = WorldModel::TPF2_BUILD_35924;
    let (config, notes) = match &args.config {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            Config::from_toml(&text).map_err(|error| error.to_string())?
        }
        None => (Config::default(), Vec::new()),
    };
    for note in notes {
        eprintln!("note: {note}");
    }
    match args.command {
        Command::Ladder => {
            println!("numbers: {} (TF3's are not measured yet)", world.name);
            for row in config.rows(&world) {
                println!("\n{:<14} {} tiles", row.label, row.tiles);
                for (k, (x, y)) in RATIOS.iter().zip(shapes(&row, config.sizes.max_tiles)) {
                    println!("  1:{k}  {}", line(&check(&world, &config, x, y)));
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Check { size } => {
            let (x, y) = parse_size(&size)?;
            let report = check(&world, &config, x, y);
            println!("numbers: {} (TF3's are not measured yet)", world.name);
            println!("{}", line(&report));
            for ceiling in &report.handled {
                println!("  handled: {ceiling:?}");
            }
            for ceiling in &report.blocked {
                println!("  BLOCKED: {ceiling:?}: {}", ceiling.remedy());
            }
            let terms = Terms::of(&report, &config);
            let print: String = terms.fingerprint()[..8]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            println!("  room terms {print}: {terms:?}");
            Ok(if report.buildable() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
        Command::Lua => {
            print!("{}", tpf3mp_bigmap::mod_data::lua(&world, &config));
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let (x, y) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("{text:?} is not <tiles>x<tiles>"))?;
    let parse = |part: &str| {
        part.trim()
            .parse::<u32>()
            .map_err(|_| format!("{text:?} is not <tiles>x<tiles>"))
    };
    Ok((parse(x)?, parse(y)?))
}

fn line(report: &SizeReport) -> String {
    let (x, y) = report.tiles;
    let verdict = if report.buildable() {
        "ok".to_owned()
    } else {
        let needs: Vec<&str> = report.blocked.iter().map(|c| c.remedy()).collect();
        format!("BLOCKED: {}", needs.join(", "))
    };
    format!(
        "{x:>4} x {y:<4} {:>8.0} km²  peak {:>5.1} GB  octree {}  street cell {} m  {verdict}",
        report.area_km2,
        report.peak_mb / 1024.0,
        report.octree_depth,
        report.street_cell_m,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse() {
        assert_eq!(parse_size("320x160"), Ok((320, 160)));
        assert_eq!(parse_size("96 X 96"), Ok((96, 96)));
        assert!(parse_size("320").is_err());
        assert!(parse_size("ax2").is_err());
    }
}
