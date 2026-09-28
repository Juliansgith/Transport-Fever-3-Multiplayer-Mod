// What the launcher's page shows, worked out from the launcher's state alone.
//
// `present(view)` takes what the launcher reports (shell.rs `View`: the
// launcher's `State`, the updater, the server's reach) and returns every
// label, button and panel the page draws. It touches no page and no
// launcher, so tests/ui/view.test.mjs runs it under plain Node; app.js
// draws its result.

/** The room's speed, as players read it: "2×", "paused". */
export function speedText(percent) {
  if (!percent) return "paused";
  const times = percent / 100;
  return `${Number.isInteger(times) ? times : times.toFixed(2).replace(/0+$/, "")}×`;
}

/** Bytes as "12.3 MB". */
export function sizeText(bytes) {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(1)} MB`;
  if (bytes >= 1e3) return `${Math.round(bytes / 1e3)} kB`;
  return `${bytes} B`;
}

function serverLabel(state) {
  return state.server_name || state.server || "";
}

function reachText(reach) {
  return { online: "online", offline: "offline", unknown: "checking" }[reach] || "";
}

/** The five steps of a game together, and which are done. */
export function steps(state) {
  const room = state.room;
  const running = room?.phase === "running";
  const connected = state.connection === "connected";
  const everyone =
    running || (room && room.members.length > 0 && room.members.every((m) => m.ready));
  return [
    { label: "Connect to a server", done: connected },
    { label: "Create or join a room", done: Boolean(room) },
    { label: "Start the game from here", done: Boolean(state.game?.attached) },
    { label: "Everyone ready", done: Boolean(everyone) },
    { label: "Play together", done: running },
  ];
}

/** The pill beside the panel's title. */
function statusPill(state) {
  const game = state.game || {};
  if (state.outdated) return { text: "Update needed", state: "update" };
  if (state.connection === "connecting") return { text: "Connecting", state: "unknown" };
  if (state.connection !== "connected") return { text: "Not connected", state: "unknown" };
  if (!state.room) return { text: "Connected", state: "ready" };
  if (game.world === "playing") return { text: "Playing", state: "ready" };
  if (state.room.phase === "running") return { text: "Game running", state: "update" };
  return { text: "In the lobby", state: "update" };
}

/** The big button: what it says, what it does, and its progress. */
function mainAction(v, state) {
  const room = state.room;
  const game = state.game || {};
  const me = room?.members.find((m) => m.you);
  const launch = {
    label: "Start Transport Fever 3",
    icon: "play",
    action: { action: "launch_game" },
    disabled: !state.installed,
  };
  if (state.outdated) {
    return v.update?.state === "ready"
      ? { label: "Restart and update", icon: "download", command: "install_update" }
      : { label: "Update TPF3-MP to play here", icon: "download", disabled: true };
  }
  if (state.connection === "connecting") {
    return { label: "Connecting…", icon: "link", disabled: true, progress: 0 };
  }
  if (state.connection !== "connected") {
    return { label: "Connect", icon: "link", form: "connect" };
  }
  if (!room) return { label: "Create room", icon: "users", form: "create" };
  if (!game.attached) return launch;
  if (game.world === "fetching") {
    const percent = game.total > 0 ? Math.floor((game.bytes / game.total) * 100) : 0;
    return {
      label: `Receiving the world${percent ? ` ${percent}%` : "…"}`,
      icon: "download",
      disabled: true,
      progress: percent,
    };
  }
  if (game.world === "loading") {
    return { label: "Loading the world…", icon: "download", disabled: true, progress: 0 };
  }
  if (game.world === "playing") {
    return { label: `Playing · ${speedText(game.speed)}`, icon: "play", disabled: true };
  }
  if (room.phase === "running") {
    return { label: "Joining the game…", icon: "play", disabled: true, progress: 0 };
  }
  const everyone = room.members.length > 0 && room.members.every((m) => m.ready);
  if (room.you_own && everyone) {
    return { label: "Start the game", icon: "play", action: { action: "start" } };
  }
  if (me && !me.ready) {
    return { label: "Ready", icon: "check", action: { action: "ready", ready: true } };
  }
  return {
    label: room.you_own ? "Waiting for everyone to be ready" : "Waiting for the owner to start",
    icon: "play",
    disabled: true,
  };
}

/** The quieter buttons under the big one. */
function secondary(state) {
  const room = state.room;
  const game = state.game || {};
  if (state.connection !== "connected") return [];
  if (!room) {
    return [
      { id: "join", label: "Join with an invite", icon: "users", form: "join" },
      { id: "disconnect", label: "Disconnect", icon: "close", action: { action: "disconnect" } },
    ];
  }
  const buttons = [];
  const me = room.members.find((m) => m.you);
  if (room.phase === "lobby" && me?.ready) {
    buttons.push({
      id: "not-ready",
      label: "Not ready",
      icon: "close",
      action: { action: "ready", ready: false },
    });
  } else if (room.phase === "lobby" && me && !game.attached) {
    buttons.push({ id: "ready", label: "Ready", icon: "check", action: { action: "ready", ready: true } });
  }
  if (room.invite) buttons.push({ id: "copy-invite", label: "Copy invite", icon: "archive", copy: room.invite });
  buttons.push({ id: "leave", label: "Leave room", icon: "close", confirm: "leave", danger: true });
  return buttons;
}

/** The line under the buttons: what just happened, or what to do next. */
function statusLine(v, state) {
  if (state.error) return { text: state.error, tone: "error" };
  const room = state.room;
  const game = state.game || {};
  if (state.outdated) {
    return { text: "This TPF3-MP is older than the server's. Update to play there.", tone: "update" };
  }
  if (state.connection !== "connected") {
    if (state.server_fixed && v.reach === "offline") {
      return { text: `The server ${serverLabel(state)} does not answer right now.`, tone: "error" };
    }
    return { text: "", tone: "" };
  }
  if (!room) return { text: "Create a room, or join one with the invite a friend sent you.", tone: "" };
  if (!state.installed) {
    return { text: "Transport Fever 3 was not found in Steam. Install it, then come back here.", tone: "error" };
  }
  if (!game.attached) {
    return {
      text: "Start Transport Fever 3 from here: only a game TPF3-MP starts joins the room. Started from Steam, it is the plain game.",
      tone: "",
    };
  }
  if (game.world === "fetching" && game.total > 0) {
    return { text: `${sizeText(game.bytes)} of ${sizeText(game.total)} received.`, tone: "" };
  }
  if (game.world === "playing") {
    const step = game.step != null ? `Step ${game.step}, ` : "";
    return { text: `${step}${game.speed ? `at ${speedText(game.speed)}` : "paused"}.`, tone: "ready" };
  }
  if (game.world === "none") {
    return { text: `The game is connected (${game.attached}). The world loads when the room starts.`, tone: "" };
  }
  return { text: "", tone: "" };
}

/** The rows of the panel's list: what the player is connected to. */
function rows(v, state) {
  const out = [];
  const server = serverLabel(state);
  if (server) {
    const reach = state.connection === "connected" ? "connected" : reachText(v.reach);
    out.push({ label: "Server", value: reach ? `${server} · ${reach}` : server });
  }
  if (state.room) {
    out.push({ label: "Room", value: state.room.name, large: true });
    out.push({
      label: "Players",
      value: `${state.room.members.length} of ${state.room.max_players}`,
    });
  } else if (state.connection === "connected" && state.server_version) {
    out.push({ label: "Server version", value: state.server_version });
  }
  return out;
}

/** How this player's game differs from the room's, as lines to show. */
export function differences(diff) {
  if (!diff) return [];
  const lines = [];
  if (diff.game) lines.push(`Game build: the room runs ${diff.game[0]}, you run ${diff.game[1]}.`);
  const list = (title, items, more) => {
    if (!items.length) return;
    lines.push(`${title} ${items.join(", ")}${more ? ` and ${more} more` : ""}.`);
  };
  list("Mods you lack:", diff.missing, diff.missing_more);
  list("Mods the room lacks (turn them off):", diff.extra, diff.extra_more);
  list(
    "Other versions:",
    diff.changed.map(([name, room, yours]) => `${name} (room ${room}, you ${yours})`),
    diff.changed_more,
  );
  if (diff.reordered) lines.push("The same mods load in another order.");
  if (diff.unlisted) lines.push("The mods beyond the listed ones differ.");
  return lines;
}

/** The room's members, as the list draws them. */
function members(state) {
  const room = state.room;
  if (!room) return [];
  return room.members.map((m) => ({
    id: m.id,
    name: m.name,
    platform: m.platform,
    you: m.you,
    owner: m.owner,
    badges: [
      m.owner ? { text: "Owner", state: "ready" } : null,
      !m.connected ? { text: "Away", state: "unknown" } : null,
      room.phase === "lobby" ? (m.ready ? { text: "Ready", state: "ready" } : { text: "Not ready", state: "unknown" }) : null,
      m.owner
        ? null
        : { same: { text: "Same mods", state: "ready" }, differs: { text: "Other mods", state: "update" } }[m.content] || null,
    ].filter(Boolean),
    removable: room.you_own && !m.you,
  }));
}

/** What the launcher-update badge, and Settings, say about updates. */
function update(v) {
  const u = v.update || { state: "none" };
  switch (u.state) {
    case "ready":
      return { badge: `Launcher update · v${u.version}`, copy: `Version ${u.version} is ready. It installs when you restart the launcher.`, installable: true };
    case "downloading": {
      const percent = u.total ? ` ${Math.floor((u.bytes / u.total) * 100)}%` : "";
      return { badge: null, copy: `Downloading version ${u.version}…${percent}`, installable: false };
    }
    case "installing":
      return { badge: null, copy: `Installing version ${u.version}…`, installable: false };
    case "checking":
      return { badge: null, copy: "Checking for launcher updates…", installable: false };
    case "up_to_date":
      return { badge: null, copy: `Launcher ${v.version} is up to date.`, installable: false };
    case "failed":
      return { badge: null, copy: `Could not update: ${u.reason}`, installable: false };
    case "held":
      return {
        badge: null,
        copy: `You chose version ${u.version}: updates wait until you resume them.`,
        installable: false,
      };
    case "off":
      return { badge: null, copy: `Updates are off: ${u.reason}.`, installable: false };
    default:
      return { badge: null, copy: "This launcher does not update itself.", installable: false };
  }
}

/** A release's notes (Markdown) as blocks of plain text: headings, list
 * items and paragraphs. Nothing becomes HTML, so notes cannot put anything
 * into the page; links keep their text. */
export function notesBlocks(markdown) {
  const inline = (text) =>
    text
      .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/(\*\*|__)(.+?)\1/g, "$2")
      .replace(/`([^`]*)`/g, "$1")
      .replace(/(^|\s)[*_]([^*_\s][^*_]*)[*_](?=\s|$|[.,;:!?])/g, "$1$2")
      .trim();
  const blocks = [];
  let paragraph = [];
  const flush = () => {
    if (paragraph.length) blocks.push({ kind: "p", text: inline(paragraph.join(" ")) });
    paragraph = [];
  };
  for (const raw of String(markdown || "").split(/\r?\n/)) {
    const line = raw.trim();
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    const item = line.match(/^(?:[-*+]|\d+[.)])\s+(.*)$/);
    if (!line || /^(-{3,}|\*{3,})$/.test(line)) flush();
    else if (heading) {
      flush();
      blocks.push({ kind: "h", text: inline(heading[1]) });
    } else if (item) {
      flush();
      blocks.push({ kind: "li", text: inline(item[1]) });
    } else paragraph.push(line);
  }
  flush();
  return blocks.filter((block) => block.text);
}

/** The latest release on the player's track, from a page of releases. */
export function latestRelease(releases, track) {
  return (
    (releases || [])
      .filter((r) => r.installable && (track === "experimental" || !r.experimental))
      .sort((a, b) => String(b.date).localeCompare(String(a.date)))[0] || null
  );
}

/** A release's date as "1 Oct 2026". */
export function dateText(iso) {
  const date = new Date(iso);
  if (!iso || Number.isNaN(date.getTime())) return "";
  return date.toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" });
}

/** Everything the page draws. */
export function present(v) {
  const state = v.launcher;
  const game = state.game || {};
  const inGame = Boolean(state.room);
  return {
    pill: statusPill(state),
    rows: rows(v, state),
    main: mainAction(v, state),
    secondary: secondary(state),
    status: statusLine(v, state),
    steps: steps(state),
    notes: [
      state.announcement ? `From the server: ${state.announcement}` : null,
      state.content_diff ? state.content_diff.summary : null,
      state.tunneled ? "Connected through the WebSocket fallback: your network blocks UDP." : null,
    ].filter(Boolean),
    differences: differences(state.content_diff),
    room: state.room
      ? {
          name: state.room.name,
          rules: state.room.rules,
          invite: state.room.invite,
          members: members(state),
          locked: state.room.has_password,
        }
      : null,
    chat: state.chat || [],
    notices: (state.notices || []).slice(-12),
    connect: {
      server: state.server || "",
      serverFixed: state.server_fixed,
      name: state.name || "",
    },
    rules: state.rules || [],
    game: {
      folder: state.installed?.dir || null,
      build: state.installed?.build || null,
      mod: v.installedMod,
      running: Boolean(game.attached),
    },
    update: update(v),
    diagnostics: state.diagnostics,
    supportId: state.support_id || null,
    version: v.version,
    askQuit: Boolean(v.quitAsked),
    track: v.track || "stable",
    held: v.held || null,
    // Versions are chosen outside a room: installing restarts TPF3-MP.
    canInstall: !state.room && (v.update?.state ?? "none") !== "none" && v.update?.state !== "off",
    inGame,
  };
}
