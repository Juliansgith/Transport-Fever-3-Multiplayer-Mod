//! The multiplayer rig end to end: several fake games on this PC, each with
//! its own agent, data folder and link, in one room on a throwaway server,
//! ending in the same world.

#![allow(clippy::unwrap_used)]

use std::{
    io::Read,
    path::Path,
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

const LIMIT: Duration = Duration::from_secs(180);

/// Runs the rig with `args` and returns its output, failing if it fails.
fn rig(data_root: &Path, args: &[&str]) -> String {
    let (status, output) = run_rig(data_root, args);
    assert!(status.success(), "the rig failed ({status}):\n{output}");
    output
}

/// Runs the rig with `args`; returns how it ended and its output, what it
/// printed and then its errors.
fn run_rig(data_root: &Path, args: &[&str]) -> (ExitStatus, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tpf3mp-rig"))
        .args(["--server", "local", "--step-rate", "50", "--data-root"])
        .arg(data_root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let read_all = |mut stream: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            stream.read_to_string(&mut text).unwrap();
            text
        })
    };
    let stdout = read_all(Box::new(child.stdout.take().unwrap()));
    let stderr = read_all(Box::new(child.stderr.take().unwrap()));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the rig ran past {LIMIT:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let output = stdout.join().unwrap() + &stderr.join().unwrap();
    (status, output)
}

/// The lane digest lines `player`'s game printed.
fn lanes(output: &str, player: &str) -> Vec<String> {
    let prefix = format!("[{player}]   lane ");
    output
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .map(str::to_owned)
        .collect()
}

#[test]
fn three_fake_games_play_one_room_and_agree() {
    let root = tempfile::tempdir().unwrap();
    let output = rig(root.path(), &["--players", "3", "--steps", "100"]);

    // The rig's launchers have a server of their own: the invite is the
    // room's code alone.
    let invite = output
        .lines()
        .find_map(|line| line.strip_prefix("rig: invite: "))
        .unwrap_or_else(|| panic!("no invite: {output}"));
    assert!(invite.parse::<tpf3mp_proto::Invite>().is_ok(), "{invite}");
    assert!(
        output.contains("rig: all 3 games ended on the same lane digests"),
        "{output}"
    );
    let host = lanes(&output, "p1");
    assert!(!host.is_empty(), "{output}");
    for guest in ["p2", "p3"] {
        assert_eq!(lanes(&output, guest), host, "{output}");
        // Guests ran every step, in the host's world.
        let report = output
            .lines()
            .find(|line| line.starts_with(&format!("[{guest}] ran ")))
            .unwrap_or_else(|| panic!("no report from {guest}:\n{output}"));
        assert!(
            report.starts_with(&format!("[{guest}] ran 100 steps")),
            "{report}"
        );
        assert!(report.contains("loaded 1 worlds from the room"), "{report}");
    }
    // Each player has a folder of its own, with its own identity.
    let keys: Vec<Vec<u8>> = ["p1", "p2", "p3"]
        .iter()
        .map(|player| std::fs::read(root.path().join(player).join("identity.key")).unwrap())
        .collect();
    assert_ne!(keys[0], keys[1]);
    assert_ne!(keys[1], keys[2]);
}

#[test]
fn staggered_games_start_one_after_another_and_still_agree() {
    let root = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let output = rig(
        root.path(),
        &["--players", "2", "--steps", "20", "--stagger", "2"],
    );
    assert!(
        output.contains("rig: starting p2's game in 2 s"),
        "{output}"
    );
    assert!(started.elapsed() >= Duration::from_secs(2), "{output}");
    assert!(
        output.contains("rig: all 2 games ended on the same lane digests"),
        "{output}"
    );
}

#[test]
fn without_snapshots_every_game_loads_its_own_world_once_all_attached() {
    let root = tempfile::tempdir().unwrap();
    let output = rig(
        root.path(),
        &[
            "--players",
            "2",
            "--steps",
            "60",
            "--stagger",
            "2",
            "--wait-for-games",
            "--no-snapshots",
        ],
    );
    let waited = output
        .find("rig: waiting for every game to attach")
        .unwrap_or_else(|| panic!("{output}"));
    let started = output
        .find("rig: game started with 2 players")
        .unwrap_or_else(|| panic!("{output}"));
    let second = output
        .find("rig: p2 plays on link")
        .unwrap_or_else(|| panic!("{output}"));
    assert!(waited < second && second < started, "{output}");
    assert!(
        output.contains("rig: all 2 games ended on the same lane digests"),
        "{output}"
    );
    for player in ["p1", "p2"] {
        let report = output
            .lines()
            .find(|line| line.starts_with(&format!("[{player}] ran ")))
            .unwrap_or_else(|| panic!("no report from {player}:\n{output}"));
        assert!(report.contains("loaded 0 worlds from the room"), "{report}");
    }
}

/// A library every system has, to load in the hook's place: `cargo test`
/// does not build the hook as a library of its own. The hook's rules have
/// their own tests; this one is about how the rig starts a game.
fn stand_in_hook() -> Option<String> {
    let candidates: Vec<std::path::PathBuf> = if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        vec![Path::new(&root).join("System32").join("version.dll")]
    } else {
        [
            "/lib/x86_64-linux-gnu/libc.so.6",
            "/usr/lib/x86_64-linux-gnu/libc.so.6",
            "/lib64/libc.so.6",
            "/usr/lib64/libc.so.6",
            "/usr/lib/libc.so.6",
        ]
        .map(std::path::PathBuf::from)
        .to_vec()
    };
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .map(|path| path.display().to_string())
}

/// A game given by path, as the real one will be, is started as the
/// launcher starts it: with a library loaded into it, and told its link in
/// the environment. The fake game stands in for the game; it finds its
/// link where the hook does, and each one joins its player's room.
#[test]
fn a_game_by_path_starts_with_the_hook_and_finds_its_link() {
    let root = tempfile::tempdir().unwrap();
    let fakegame = env!("CARGO_BIN_EXE_tpf3mp-fakegame");
    let hook = stand_in_hook();
    let mut args = vec![
        "--players",
        "2",
        "--game",
        fakegame,
        "--game-arg",
        "--steps",
        "--game-arg",
        "60",
    ];
    if let Some(hook) = &hook {
        args.extend(["--hook", hook]);
    }
    if cfg!(target_os = "macos") {
        // No game gets the hook on macOS yet; the rig says so and stops.
        let (status, output) = run_rig(root.path(), &args);
        assert!(!status.success(), "{output}");
        assert!(
            output.contains("not possible on this system yet"),
            "{output}"
        );
        return;
    }
    let output = rig(root.path(), &args);
    for player in ["p1", "p2"] {
        assert!(
            output
                .lines()
                .any(|line| line.starts_with(&format!("[{player}] game "))
                    && line.contains("attached")),
            "{player}'s game never reached its agent:\n{output}"
        );
    }
    assert!(
        output.contains("rig: game started with 2 players"),
        "{output}"
    );
    // Both games ran to their last step and exited cleanly: a game that
    // fails fails the rig.
    assert!(!output.contains("game failed"), "{output}");
}
