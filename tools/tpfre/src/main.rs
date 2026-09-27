//! The `tpfre` binary: see `tpfre --help` and README.md.

use std::io::Write;

fn main() {
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let code = tpfre::cli::run(std::env::args_os(), &mut out, &mut stderr.lock());
    // A closed pipe (`| head`) is not an error worth reporting.
    let _ = out.flush();
    drop(out);
    std::process::exit(code);
}
