// What the launcher's page shows for each state, checked without a page:
// ui/view.js's present(), over the preview's sample states (ui/preview.js).
// Run with `node --test crates/tpf3mp-launcher/tests/ui/` (CI's launcher page
// job); plain Node, no packages.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  present,
  speedText,
  sizeText,
  differences,
  steps,
  notesBlocks,
  latestRelease,
  onTrack,
  trackName,
} from "../../ui/view.js";
import { SCENES } from "../../ui/preview.js";

const base = {
  name: "Ann",
  server: "play.example:29470",
  server_fixed: true,
  server_name: "EU",
  rules: [{ name: "native", description: "" }],
  connection: "disconnected",
  error: null,
  outdated: false,
  room: null,
  content_diff: null,
  game: { attached: null, world: "none", bytes: 0, total: 0, step: null, speed: 100 },
  installed: { dir: "C:\Games\Transport Fever 3", build: "20364158" },
  chat: [],
  notices: [],
  diagnostics: true,
};

function view(scene, extra = {}) {
  return {
    launcher: { ...structuredClone(base), ...structuredClone(SCENES[scene]), ...extra },
    update: { state: "up_to_date" },
    reach: "online",
    version: "0.1.0",
    installedMod: "0.1.0",
    quitAsked: false,
    platform: "windows",
  };
}

test("every sample state can be drawn", () => {
  for (const scene of Object.keys(SCENES)) {
    const p = present(view(scene));
    assert.ok(p.main.label, `${scene}: the big button says something`);
    assert.ok(p.pill.text, `${scene}: the pill says something`);
  }
});

test("the big button says what comes next", () => {
  assert.equal(present(view("Not connected")).main.label, "Connect");
  assert.equal(present(view("Not connected")).main.form, "connect");
  assert.equal(present(view("Connecting")).main.disabled, true);
  assert.equal(present(view("Connected")).main.label, "Create room");
  const lobby = present(view("In the lobby"));
  assert.equal(lobby.main.label, "Start Transport Fever 3");
  assert.deepEqual(lobby.main.action, { action: "launch_game" });
  const started = present(view("Game started"));
  assert.equal(started.main.label, "Start the game", "the owner starts once everyone is ready");
  assert.deepEqual(started.main.action, { action: "start" });
  const fetching = present(view("Receiving the world"));
  assert.equal(fetching.main.label, "Receiving the world 42%");
  assert.equal(fetching.main.progress, 42);
  assert.equal(present(view("Playing")).main.label, "Playing · 2×");
});

test("only the owner starts, and only once everyone is ready", () => {
  const notReady = view("Game started");
  notReady.launcher.room.members[2].ready = false;
  assert.equal(present(notReady).main.label, "Waiting for everyone to be ready");
  assert.equal(present(notReady).main.disabled, true);
  const guest = view("Game started");
  guest.launcher.room.you_own = false;
  assert.equal(present(guest).main.label, "Waiting for the owner to start");
});

test("the game cannot be started when Steam has none", () => {
  const p = present(view("In the lobby", { installed: null }));
  assert.equal(p.main.disabled, true);
  assert.match(p.status.text, /not found in Steam/);
});

test("an outdated TPF3-MP updates before it plays", () => {
  const waiting = present(view("Update needed"));
  assert.equal(waiting.main.disabled, true);
  const ready = view("Update needed");
  ready.update = { state: "ready", version: "0.2.0" };
  assert.equal(present(ready).main.command, "install_update");
});

test("a refusal shows as an error", () => {
  const p = present(view("Connected", { error: "No room has that invite." }));
  assert.deepEqual(p.status, { text: "No room has that invite.", tone: "error" });
});

test("members show who owns, who is ready, whose mods differ", () => {
  const p = present(view("In the lobby"));
  const [ann, bob, cat] = p.room.members;
  assert.deepEqual(ann.badges.map((b) => b.text), ["Owner", "Ready"]);
  assert.equal(ann.removable, false, "not yourself");
  assert.deepEqual(bob.badges.map((b) => b.text), ["Ready", "Same mods"]);
  assert.deepEqual(cat.badges.map((b) => b.text), ["Not ready", "Other mods"]);
  assert.equal(cat.removable, true);
});

test("the room's secondary buttons", () => {
  const ids = present(view("In the lobby")).secondary.map((b) => b.id);
  assert.deepEqual(ids, ["not-ready", "copy-invite", "leave"]);
  assert.equal(present(view("In the lobby")).secondary[1].copy, "K7QM2X");
  assert.deepEqual(
    present(view("Connected")).secondary.map((b) => b.id),
    ["join", "disconnect"],
  );
});

test("the steps tick off as the game comes together", () => {
  const done = (scene) => steps(view(scene).launcher).filter((s) => s.done).length;
  assert.equal(done("Not connected"), 0);
  assert.equal(done("Connected"), 1);
  assert.equal(done("In the lobby"), 2);
  assert.equal(done("Game started"), 4);
  assert.equal(done("Playing"), 5);
});

test("a server that does not answer is said to be down", () => {
  const down = view("Not connected");
  down.reach = "offline";
  assert.match(present(down).status.text, /does not answer/);
  assert.equal(present(down).status.tone, "error");
});

test("launcher updates show on the badge when ready", () => {
  const v = view("Connected");
  v.update = { state: "ready", version: "0.2.0" };
  assert.equal(present(v).update.badge, "Launcher update · v0.2.0");
  assert.equal(present(v).update.installable, true);
  v.update = { state: "downloading", version: "0.2.0", bytes: 5, total: 10 };
  assert.equal(present(v).update.badge, null);
  assert.equal(present(v).update.copy, "Downloading version 0.2.0… 50%");
});

test("differences read as sentences", () => {
  assert.deepEqual(
    differences({
      game: ["40391", "40400"],
      missing: ["a 1"],
      missing_more: 2,
      extra: [],
      extra_more: 0,
      changed: [["b", "1", "2"]],
      changed_more: 0,
      reordered: true,
      unlisted: false,
    }),
    [
      "Game build: the room runs 40391, you run 40400.",
      "Mods you lack: a 1 and 2 more.",
      "Other versions: b (room 1, you 2).",
      "The same mods load in another order.",
    ],
  );
});

test("speeds and sizes read as players say them", () => {
  assert.equal(speedText(0), "paused");
  assert.equal(speedText(100), "1×");
  assert.equal(speedText(250), "2.5×");
  assert.equal(sizeText(48_000_000), "48.0 MB");
  assert.equal(sizeText(1_500), "2 kB");
});

test("release notes become plain blocks, never markup", () => {
  assert.deepEqual(
    notesBlocks(
      [
        "## Fixes",
        "- **Bold** [a link](https://example.org)",
        "1. `code`",
        "",
        "One",
        "paragraph",
        "---",
        "<img src=x onerror=alert(1)>",
      ].join("\n"),
    ),
    [
      { kind: "h", text: "Fixes" },
      { kind: "li", text: "Bold a link" },
      { kind: "li", text: "code" },
      { kind: "p", text: "One paragraph" },
      { kind: "p", text: "<img src=x onerror=alert(1)>" },
    ],
  );
  assert.deepEqual(notesBlocks(null), []);
});

test("the latest release depends on the track", () => {
  const releases = [
    { version: "0.3.0-beta.1", date: "2026-10-05", experimental: true, installable: true },
    { version: "0.2.1", date: "2026-10-03", experimental: false, installable: false },
    { version: "0.2.0", date: "2026-10-01", experimental: false, installable: true },
  ];
  assert.equal(latestRelease(releases, "stable").version, "0.2.0", "the newest signed stable one");
  assert.equal(latestRelease(releases, "experimental").version, "0.3.0-beta.1");
  assert.equal(latestRelease([], "stable"), null);
});

test("only the Dev track offers dev builds", () => {
  const dev = { version: "0.3.1-dev.12", date: "2026-10-06", experimental: true, dev: true, installable: true };
  const beta = { version: "0.3.0-beta.1", date: "2026-10-05", experimental: true, installable: true };
  const stable = { version: "0.2.0", date: "2026-10-01", experimental: false, installable: true };
  const releases = [dev, beta, stable];
  assert.equal(latestRelease(releases, "stable").version, "0.2.0");
  assert.equal(latestRelease(releases, "experimental").version, "0.3.0-beta.1");
  assert.equal(latestRelease(releases, "dev").version, "0.3.1-dev.12");
  assert.deepEqual(
    ["stable", "experimental", "dev"].map((track) => onTrack(dev, track)),
    [false, false, true],
  );
  assert.equal(trackName("dev"), "Dev builds");
  assert.equal(trackName("anything else"), "Stable");
});

test("the Dev track is offered only by a build that trusts dev builds", () => {
  assert.equal(present({ ...view("Not connected"), devTrack: true }).devTrack, true);
  assert.equal(present({ ...view("Not connected") }).devTrack, false);
});

test("a held version says so, and versions are chosen outside rooms", () => {
  const v = view("Connected");
  v.update = { state: "held", version: "0.1.0" };
  v.held = "0.1.0";
  const p = present(v);
  assert.equal(p.held, "0.1.0");
  assert.match(p.update.copy, /You chose version 0\.1\.0/);
  assert.equal(p.canInstall, true);
  assert.equal(present(view("In the lobby")).canInstall, false, "installing restarts TPF3-MP");
  const none = view("Connected");
  none.update = { state: "none" };
  assert.equal(present(none).canInstall, false, "a copy that does not update itself");
});
