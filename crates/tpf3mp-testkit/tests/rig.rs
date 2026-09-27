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

    assert!(output.contains("rig: invite: 127.0.0.1:"), "{output}");
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

/// A game given by path, as the real one will be, is started as the
/// launcher starts it: with the hook loaded into it (the one built next to
/// the rig), and told its link in the environment. The fake game stands in
/// for the game; it finds its link where the hook does, and each one joins
/// its player's room.
#[test]
fn a_game_by_path_starts_with_the_hook_and_finds_its_link() {
    let root = tempfile::tempdir().unwrap();
    let fakegame = env!("CARGO_BIN_EXE_tpf3mp-fakegame");
    let args = [
        "--players",
        "2",
        "--game",
        fakegame,
        "--game-arg",
        "--steps",
        "--game-arg",
        "60",
    ];
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
