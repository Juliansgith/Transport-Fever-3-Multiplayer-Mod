// The page without a launcher: sample states to look at the design with, as
// tearded's preview mode did. Open ui/index.html from any web server
// (`python -m http.server` in ui/) and pick a state at the top; the buttons
// move the sample along. Nothing here reaches a server or a game.

const base = {
  name: "Ann",
  player: "7QM2",
  server: "play.tpf3mp.example:29470",
  server_fixed: true,
  server_name: "EU",
  server_version: null,
  support_id: null,
  rules: [
    { name: "native", description: "The game's own economy, as in single player." },
    { name: "canonical", description: "The server settles the economy." },
  ],
  connection: "disconnected",
  tunneled: false,
  error: null,
  outdated: false,
  room: null,
  content_diff: null,
  game: { attached: null, world: "none", bytes: 0, total: 0, step: null, speed: 100 },
  installed: { dir: "C:\\Program Files (x86)\\Steam\\steamapps\\common\\Transport Fever 3", build: "20364158" },
  chat: [],
  notices: [],
  announcement: null,
  diagnostics: true,
};

const members = [
  { id: "p1", name: "Ann", platform: "Windows x86-64", ready: true, connected: true, owner: true, you: true, content: "same" },
  { id: "p2", name: "Bob", platform: "Windows x86-64", ready: true, connected: true, owner: false, you: false, content: "same" },
  { id: "p3", name: "Cat", platform: "Linux x86-64", ready: false, connected: true, owner: false, you: false, content: "differs" },
];

const connected = { connection: "connected", server_version: "0.1.0", support_id: "S4TK9Q" };
const room = {
  name: "Friday trains",
  rules: "native",
  phase: "lobby",
  invite: "K7QM2X",
  you_own: true,
  max_players: 4,
  has_password: false,
  members,
};
const chat = [
  { from: "Bob", text: "I'll take the coal line up north.", you: false },
  { from: "Ann", text: "Fine by me, I'm on passengers.", you: true },
];

export const SCENES = {
  "Not connected": {},
  Connecting: { connection: "connecting" },
  Connected: { ...connected },
  "In the lobby": {
    ...connected,
    room,
    chat,
    content_diff: {
      summary: "Cat runs other mods than the room.",
      game: null,
      missing: [],
      missing_more: 0,
      extra: ["gw_cheats_1 1"],
      extra_more: 0,
      changed: [],
      changed_more: 0,
      reordered: false,
      unlisted: false,
    },
  },
  "Game started": {
    ...connected,
    room: { ...room, members: members.map((m) => ({ ...m, ready: true, content: "same" })) },
    chat,
    game: { attached: "40391", world: "none", bytes: 0, total: 0, step: null, speed: 100 },
  },
  "Receiving the world": {
    ...connected,
    room: { ...room, phase: "running" },
    chat,
    game: { attached: "40391", world: "fetching", bytes: 48_000_000, total: 112_000_000, step: null, speed: 100 },
  },
  Playing: {
    ...connected,
    room: { ...room, phase: "running" },
    chat,
    notices: ["Bob joined the room.", "The room started.", "Speed 2×."],
    game: { attached: "40391", world: "playing", bytes: 0, total: 0, step: 18432, speed: 200 },
  },
  "Update needed": { outdated: true, error: "The server speaks a newer protocol: update TPF3-MP to play there." },
};

function scene(name) {
  return { ...structuredClone(base), ...structuredClone(SCENES[name] || {}) };
}

const RELEASES = [
  {
    version: "0.2.1-dev.14",
    title: "Dev build 0.2.1-dev.14",
    notes: "Built from dev at 1fca952: Keep the paced-latency test from failing on slow CI machines.",
    date: "2026-10-07T10:00:00Z",
    experimental: true,
    dev: true,
    installable: true,
  },
  {
    version: "0.2.0",
    title: "TPF3-MP 0.2.0",
    notes:
      "## New\n- Rooms remember their rules across a server restart.\n- The launcher shows release notes.\n\n## Fixes\n- Joining by invite works in either case.",
    date: "2026-10-06T10:00:00Z",
    experimental: false,
    installable: true,
  },
  {
    version: "0.2.0-beta.2",
    title: "Beta",
    notes: "Testing the new world transfer. **Experimental**: play on the test server.",
    date: "2026-10-04T10:00:00Z",
    experimental: true,
    installable: true,
  },
  {
    version: "0.1.0",
    title: "TPF3-MP 0.1.0",
    notes: "The first release, for Transport Fever 3's launch day.",
    date: "2026-09-29T10:00:00Z",
    experimental: false,
    installable: true,
  },
];

export function previewBackend() {
  let name = decodeURIComponent(location.hash.slice(1)) || "Not connected";
  if (!(name in SCENES)) name = "Not connected";
  let state = scene(name);
  const update = { state: "ready", version: "0.2.0" };
  let track = "stable";
  let held = null;

  // A picker at the top of the page, in tearded's preview style; left out
  // with ?shot, for screenshots of a state.
  const picker = document.createElement("select");
  picker.className = "preview-label";
  picker.setAttribute("aria-label", "Preview state");
  for (const option of Object.keys(SCENES)) picker.append(new Option(`Preview: ${option}`, option));
  picker.value = name;
  picker.addEventListener("change", () => {
    name = picker.value;
    location.hash = encodeURIComponent(name);
    state = scene(name);
  });
  if (!new URLSearchParams(location.search).has("shot")) {
    document.querySelector(".header-actions").prepend(picker);
  }

  const go = (next) => {
    name = next;
    picker.value = next;
    state = scene(next);
  };
  return {
    async view() {
      return {
        launcher: state,
        update,
        reach: "online",
        version: "0.1.0",
        installedMod: "0.1.0",
        quitAsked: false,
        platform: "windows",
        track,
        devTrack: true,
        held,
      };
    },
    async act(action) {
      switch (action.action) {
        case "connect":
          return go("Connected");
        case "disconnect":
        case "leave":
          return go(action.action === "leave" ? "Connected" : "Not connected");
        case "create":
        case "join":
          return go("In the lobby");
        case "launch_game":
          return go("Game started");
        case "start":
          return go("Receiving the world");
        case "chat":
          state.chat.push({ from: state.name, text: action.text, you: true });
          return;
        case "ready":
          state.room.members.find((m) => m.you).ready = action.ready;
          return;
        default:
          return;
      }
    },
    async checkUpdate() {},
    async installUpdate() {},
    async openGameFolder() {},
    async answerQuit() {},
    async releases(page) {
      return page === 1
        ? { releases: RELEASES.slice(0, 3), more: true }
        : { releases: RELEASES.slice(3), more: false };
    },
    async installVersion(version) {
      held = version;
    },
    async setTrack(chosen) {
      track = chosen;
    },
    async resumeUpdates() {
      held = null;
    },
  };
}
