//! What TPF3-MP's build scripts put into its programs, so a program can say
//! exactly which build it is (docs/DEVELOPMENT.md, "Which build is this"):
//!
//! - `TPF3MP_COMMIT`: the git commit built, short, or `unknown` outside a
//!   git checkout;
//! - `TPF3MP_BUILT`: when the build script last ran, in UTC
//!   (`SOURCE_DATE_EPOCH` when set, for reproducible builds);
//! - `TPF3MP_BUILD_NUMBER`: the number of commits up to the one built,
//!   which grows with every commit on `dev`.
//!
//! Each is read with `env!` in the crate whose build script calls [`emit`].
//! The build script itself runs again when the checkout's `HEAD` moves.
//!
//! On Windows, [`emit`] also links a VERSIONINFO resource into the program
//! or library: Explorer's Properties show its version and commit, and
//! Windows Installer replaces a versioned file only with a newer version.
//! Its file version is the crate's `major.minor.patch` and the build number
//! as the fourth part, so a later commit's build always counts as newer.
//! Override them with `TPF3MP_COMMIT` and `TPF3MP_BUILD_NUMBER` in the
//! build's environment.

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

/// What the build script builds, for the version resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Artifact {
    /// The package's program (`[[bin]]`, `src/main.rs`).
    Program,
    /// The package's `cdylib`, as the game's hook.
    Library,
}

/// Emits the build's commit, time and number for the crate whose build
/// script calls it, and on Windows links its version resource.
/// `description` is the resource's file description, such as "TPF3-MP
/// launcher"; `file` the name the file is built as, such as
/// `tpf3mp-launcher.exe`.
pub fn emit(artifact: Artifact, description: &str, file: &str) {
    emit_with_icon(artifact, description, file, None);
}

/// Links version information and an optional icon in one resource, so neither
/// resource compiler overwrites the other's output.
pub fn emit_with_icon(artifact: Artifact, description: &str, file: &str, icon: Option<&Path>) {
    let dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    for name in ["TPF3MP_COMMIT", "TPF3MP_BUILD_NUMBER", "SOURCE_DATE_EPOCH"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    // A commit, a checkout or a reset moves HEAD's reflog; HEAD itself
    // where there is none.
    for watched in ["logs/HEAD", "HEAD"] {
        if let Some(path) = git(&dir, &["rev-parse", "--git-path", watched])
            .map(|path| dir.join(path))
            .filter(|path| path.is_file())
        {
            println!("cargo:rerun-if-changed={}", path.display());
            break;
        }
    }
    let commit = given("TPF3MP_COMMIT")
        .or_else(|| git(&dir, &["rev-parse", "--short=10", "HEAD"]))
        .filter(|commit| is_commit(commit))
        .unwrap_or_else(|| "unknown".to_owned());
    let number = given("TPF3MP_BUILD_NUMBER")
        .and_then(|number| number.parse::<u64>().ok())
        .or_else(|| build_number(&dir))
        .unwrap_or(0);
    let seconds = given("SOURCE_DATE_EPOCH")
        .and_then(|seconds| seconds.parse::<u64>().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs())
        });
    println!("cargo:rustc-env=TPF3MP_COMMIT={commit}");
    println!("cargo:rustc-env=TPF3MP_BUILT={}", utc(seconds));
    println!("cargo:rustc-env=TPF3MP_BUILD_NUMBER={number}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = file_version(
            &std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap_or_default(),
            &std::env::var("CARGO_PKG_VERSION_MINOR").unwrap_or_default(),
            &std::env::var("CARGO_PKG_VERSION_PATCH").unwrap_or_default(),
            number,
        );
        let mut rc = version_rc(&Resource {
            artifact,
            version,
            product: &std::env::var("CARGO_PKG_VERSION").unwrap_or_default(),
            commit: &commit,
            description,
            file,
        });
        if let Some(icon) = icon {
            let icon = icon.to_str().expect("icon path is UTF-8").replace('\\', "/").replace('"', "\"\"");
            rc.push_str(&format!("\n1 ICON \"{icon}\"\n"));
        }
        let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default());
        let path = out.join("tpf3mp-version.rc");
        if let Err(error) = std::fs::write(&path, rc) {
            println!("cargo:warning=cannot write the version resource: {error}");
            return;
        }
        link(artifact, &path);
    }
}

#[cfg(windows)]
fn link(artifact: Artifact, rc: &Path) {
    let result = match artifact {
        Artifact::Program => embed_resource::compile(rc, embed_resource::NONE),
        Artifact::Library => embed_resource::compile_for_cdylib(rc, embed_resource::NONE),
    };
    // No resource compiler at all (a machine without the Windows SDK) builds
    // without the resource, and says so; one that fails stops the build.
    match result {
        embed_resource::CompilationResult::NotAttempted(why) => {
            println!("cargo:warning=built without a version resource: {why}");
        }
        embed_resource::CompilationResult::Failed(why) => {
            panic!("compiling the version resource failed: {why}");
        }
        embed_resource::CompilationResult::Ok | embed_resource::CompilationResult::NotWindows => {}
    }
}

/// Cross-compiled for Windows from another system: no resource compiler is
/// looked for.
#[cfg(not(windows))]
fn link(_artifact: Artifact, _rc: &Path) {
    println!("cargo:warning=built for Windows without a version resource");
}

fn given(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(text.trim().to_owned()).filter(|text| !text.is_empty())
}

/// The commits up to `HEAD`; none in a shallow clone, which knows only a
/// few and would count too low.
fn build_number(dir: &Path) -> Option<u64> {
    if git(dir, &["rev-parse", "--is-shallow-repository"]).as_deref() == Some("true") {
        println!(
            "cargo:warning=a shallow clone has no build number; fetch the whole history or set TPF3MP_BUILD_NUMBER"
        );
        return None;
    }
    git(dir, &["rev-list", "--count", "HEAD"])?.parse().ok()
}

/// Whether `text` can be a commit as the program shows it: hex digits only,
/// so nothing odd reaches the resource or a log line.
pub fn is_commit(text: &str) -> bool {
    (4..=40).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The version resource's four-part file version: `major.minor.patch` of
/// the crate and the build number, each as Windows keeps it (16 bits).
pub fn file_version(major: &str, minor: &str, patch: &str, number: u64) -> [u16; 4] {
    let part = |text: &str| text.parse::<u64>().map_or(0, clamp);
    [part(major), part(minor), part(patch), clamp(number)]
}

fn clamp(value: u64) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// `seconds` since 1970 as `2026-09-30 21:04 UTC`.
pub fn utc(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let of_day = seconds % 86_400;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        of_day / 3600,
        of_day % 3600 / 60
    )
}

/// What the version resource says.
#[derive(Debug, Clone)]
pub struct Resource<'a> {
    pub artifact: Artifact,
    pub version: [u16; 4],
    /// The crate's version as written, such as `0.1.0`.
    pub product: &'a str,
    pub commit: &'a str,
    pub description: &'a str,
    pub file: &'a str,
}

/// The resource script of a VERSIONINFO for `resource`, in numbers alone so
/// that it needs no header from the Windows SDK.
pub fn version_rc(resource: &Resource<'_>) -> String {
    let [a, b, c, d] = resource.version;
    // VFT_APP or VFT_DLL; VOS_NT_WINDOWS32.
    let file_type = match resource.artifact {
        Artifact::Program => 1,
        Artifact::Library => 2,
    };
    let text = |value: &str| value.replace('"', "\"\"");
    format!(
        r#"1 VERSIONINFO
FILEVERSION {a},{b},{c},{d}
PRODUCTVERSION {a},{b},{c},{d}
FILEFLAGSMASK 0x3F
FILEFLAGS 0x0
FILEOS 0x40004
FILETYPE {file_type}
FILESUBTYPE 0x0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "TPF3-MP"
      VALUE "FileDescription", "{description}"
      VALUE "FileVersion", "{a}.{b}.{c}.{d}"
      VALUE "InternalName", "{file}"
      VALUE "OriginalFilename", "{file}"
      VALUE "ProductName", "TPF3-MP"
      VALUE "ProductVersion", "{product}"
      VALUE "Commit", "{commit}"
      VALUE "Comments", "TPF3-MP {product}, commit {commit}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        description = text(resource.description),
        file = text(resource.file),
        product = text(resource.product),
        commit = text(resource.commit),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_version_is_the_crates_and_the_build_number() {
        assert_eq!(file_version("0", "1", "0", 442), [0, 1, 0, 442]);
        assert_eq!(
            file_version("1", "x", "70000", 1 << 40),
            [1, 0, u16::MAX, u16::MAX],
            "Windows keeps 16 bits a part"
        );
    }

    #[test]
    fn times_are_utc_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(utc(1_790_802_000), "2026-09-30 21:00 UTC");
    }

    #[test]
    fn only_hex_passes_as_a_commit() {
        assert!(is_commit("1316710abc"));
        assert!(!is_commit("unknown"));
        assert!(!is_commit("abc\"def"));
        assert!(!is_commit(""));
    }

    #[test]
    fn the_resource_names_version_and_commit() {
        let rc = version_rc(&Resource {
            artifact: Artifact::Library,
            version: [0, 1, 0, 442],
            product: "0.1.0",
            commit: "1316710abc",
            description: "TPF3-MP \"hook\"",
            file: "tpf3mp_hook.dll",
        });
        assert!(rc.contains("FILEVERSION 0,1,0,442"));
        assert!(rc.contains("FILETYPE 2"));
        assert!(rc.contains(r#"VALUE "FileVersion", "0.1.0.442""#));
        assert!(rc.contains(r#"VALUE "ProductVersion", "0.1.0""#));
        assert!(rc.contains(r#"VALUE "Commit", "1316710abc""#));
        assert!(rc.contains(r#""TPF3-MP ""hook""""#), "quotes doubled");
    }
}
