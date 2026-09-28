//! The TPF3-MP mod the install scripts put in the game's mods folder.

use std::path::Path;

/// The TPF3-MP version whose mod the install scripts put in place, from
/// their record in TPF3-MP's data folder `dir` (`installed.json` from
/// Windows's, `installed.txt` from Linux's and macOS's), while the mod is
/// still where the record says: a folder with Transport Fever 3's
/// `mod.json`, or Transport Fever 2's `mod.lua` that the mod still carries.
pub fn installed_mod(dir: &Path) -> Option<String> {
    let (version, folder) = if let Ok(text) = std::fs::read_to_string(dir.join("installed.json")) {
        let record: serde_json::Value =
            serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
        (
            record.get("version")?.as_str()?.to_owned(),
            record.get("mod")?.as_str()?.to_owned(),
        )
    } else {
        let text = std::fs::read_to_string(dir.join("installed.txt")).ok()?;
        let field = |name: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(name))
                .map(str::to_owned)
        };
        (field("version=")?, field("mod=")?)
    };
    let folder = Path::new(&folder);
    ["mod.json", "mod.lua"]
        .iter()
        .any(|file| folder.join(file).is_file())
        .then_some(version)
}

#[cfg(test)]
mod tests {
    use super::installed_mod;

    #[test]
    fn the_version_the_install_scripts_recorded_is_read() {
        let dir = std::env::temp_dir().join(format!("tpf3mp-installed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mods = dir.join("mods").join("tpf3mp_1");
        std::fs::create_dir_all(&mods).unwrap();
        std::fs::write(mods.join("mod.lua"), "-- mod").unwrap();
        assert_eq!(installed_mod(&dir), None, "nothing installed");

        // install.sh's record.
        std::fs::write(
            dir.join("installed.txt"),
            format!("version=0.1.0\nmod={}\n", mods.display()),
        )
        .unwrap();
        assert_eq!(installed_mod(&dir).as_deref(), Some("0.1.0"));

        // install.ps1's, which Windows tools may start with a byte order mark.
        let json = serde_json::json!({ "version": "0.2.0", "mod": mods });
        std::fs::write(dir.join("installed.json"), format!("\u{feff}{json}")).unwrap();
        assert_eq!(installed_mod(&dir).as_deref(), Some("0.2.0"));

        // A mod in Transport Fever 3's layout, with mod.json.
        std::fs::remove_file(mods.join("mod.lua")).unwrap();
        std::fs::write(mods.join("mod.json"), "{}").unwrap();
        assert_eq!(installed_mod(&dir).as_deref(), Some("0.2.0"));

        // The mod taken away since.
        std::fs::remove_file(mods.join("mod.json")).unwrap();
        assert_eq!(installed_mod(&dir), None, "the mod is gone");

        std::fs::write(dir.join("installed.json"), "damaged").unwrap();
        assert_eq!(installed_mod(&dir), None, "a damaged record");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
