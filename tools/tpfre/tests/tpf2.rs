//! tpfre against the real Transport Fever 2 executable (build 35924).
//!
//! Ignored by default: the game is not in the repository. Run it with the
//! path to a local copy (read only; it is never modified or launched):
//!
//! ```text
//! TPFRE_TPF2_EXE="E:\SteamLibrary\steamapps\common\Transport Fever 2\TransportFever2.exe" \
//!     cargo test --release --test tpf2 -- --ignored --nocapture
//! ```
//!
//! It checks the numbers `tools/re/name_functions.py` recorded in
//! `investigation/tpf2-baseline/` and the eight known RVAs.

use std::path::PathBuf;
use std::time::Instant;

const SHA256: &str = "782b904a8f7bbdac1f7a18528f1a5c778691e5aa3087c37c351bf6912585175c";

fn run(args: &[&str]) -> (i32, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut argv = vec!["tpfre"];
    argv.extend_from_slice(args);
    let code = tpfre::cli::run(argv, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).expect("utf8") + &String::from_utf8(err).expect("utf8"),
    )
}

#[test]
#[ignore = "needs TPFRE_TPF2_EXE, a local TransportFever2.exe build 35924"]
fn tpf2_build_35924_matches_the_python_pipeline() {
    let exe = std::env::var("TPFRE_TPF2_EXE").expect("set TPFRE_TPF2_EXE to TransportFever2.exe");
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("tpf2.tpfdb");
    let db_s = db.to_str().expect("utf8");
    let t = Instant::now();
    let (code, out) = run(&["index", &exe, "-o", db_s]);
    println!("index: {:.1} s", t.elapsed().as_secs_f64());
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(&format!("sha256 {SHA256}")),
        "not build 35924:\n{out}"
    );
    // name_functions_TransportFever2.txt
    for line in [
        "naming.assert_funcsig_strings 18897",
        "naming.assert_file_strings 729",
        "naming.references_resolved 88198",
        "naming.functions_named_funcsig 20746",
        "naming.functions_named_ambiguous 3454",
        "naming.functions_file_direct 20062",
        "naming.distinct_source_files 725",
        "functions_pdata 115484",
    ] {
        assert!(out.contains(line), "expected {line:?} in:\n{out}");
    }
    let spec = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../investigation/tpf2-baseline/tpf2_known_rvas.txt");
    let (code, out) = run(&["q", db_s, "validate", spec.to_str().expect("utf8")]);
    println!("{out}");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("validate 8 checked: ALL OK"), "{out}");
}
