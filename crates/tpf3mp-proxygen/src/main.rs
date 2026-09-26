//! Command line front end for the proxy generator.
//!
//! ```text
//! tpf3mp-proxygen <source.dll|source.def> <out_dir> [proxy_stem] [real_stem] [--load-hook <hook.dll>]
//! ```
//!
//! Reads `source.dll`'s exports, or those of a `.def` an earlier run wrote,
//! and writes a proxy crate into `out_dir` that forwards each one to
//! `real_stem.dll` (default: `<proxy_stem>_real`). With `--load-hook`, the
//! proxy also loads that library from its own folder. Build the crate with
//! cargo to produce `proxy_stem.dll`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use tpf3mp_proxygen::{parse_def, parse_exports, write_proxy_with};

/// Generates a Windows proxy DLL crate from a DLL's exports.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    /// The DLL to stand in for, or a `.def` this program wrote for it.
    source: PathBuf,
    /// Where the proxy crate is written. Keep it outside any cargo
    /// workspace.
    out_dir: PathBuf,
    /// The proxy's file stem; the source's by default.
    proxy_stem: Option<String>,
    /// The stem of the renamed original; `<proxy_stem>_real` by default.
    real_stem: Option<String>,
    /// Also load this library, from the proxy's own folder, when the game
    /// loads the proxy: the hook, such as `tpf3mp_hook.dll`.
    #[arg(long)]
    load_hook: Option<String>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let proxy_stem = args.proxy_stem.clone().unwrap_or_else(|| {
        args.source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("proxy")
            .to_owned()
    });
    let real_stem = args
        .real_stem
        .clone()
        .unwrap_or_else(|| format!("{proxy_stem}_real"));

    let bytes = match std::fs::read(&args.source) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("cannot read {}: {error}", args.source.display());
            return ExitCode::FAILURE;
        }
    };
    let is_def = args
        .source
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("def"));
    let exports = if is_def {
        String::from_utf8(bytes)
            .map_err(|_| "the .def is not UTF-8".to_owned())
            .and_then(|text| parse_def(&text).map_err(|error| error.to_string()))
    } else {
        parse_exports(&bytes).map_err(|error| error.to_string())
    };
    let exports = match exports {
        Ok(exports) => exports,
        Err(error) => {
            eprintln!(
                "cannot read the exports of {}: {error}",
                args.source.display()
            );
            return ExitCode::FAILURE;
        }
    };
    match write_proxy_with(
        &args.out_dir,
        &exports,
        &proxy_stem,
        &real_stem,
        args.load_hook.as_deref(),
    ) {
        Ok(dir) => {
            println!(
                "wrote proxy crate to {} ({} exports forwarded to {real_stem}.dll{})",
                dir.display(),
                exports.entries.len(),
                match &args.load_hook {
                    Some(hook) => format!(", loading {hook}"),
                    None => String::new(),
                }
            );
            println!(
                "build it with: cargo build --release --manifest-path {}",
                dir.join("Cargo.toml").display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("cannot write proxy crate: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_line_is_consistent() {
        Args::command().debug_assert();
    }
}
