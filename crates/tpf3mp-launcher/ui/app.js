// The launcher's page: draws what view.js works out, and sends the player's
// actions to the launcher. In the launcher's window it talks to the Rust
// side through Tauri (main.rs's commands); opened as a file or from a web
// server, with no Tauri, it shows sample states instead (preview.js), for
// looking at the design without a launcher.
//
// The layout and its behaviour follow tearded's TPF2 Multiplayer Launcher
// (MIT; THIRD_PARTY.md): one big button that says what comes next, quieter
// buttons under it, a status line, and the game's folder along the bottom.

import { present, notesBlocks, latestRelease, dateText, trackName } from "./view.js";

const $ = (id) => document.getElementById(id);
const POLL = 400;

let backend = null;
let current = null;
let busy = false;
// A form the player opened with a secondary button: "join".
let openForm = null;
// Fields are filled from the launcher once, then left to the player.
let offered = false;
let lastError = null;
let toastTimer;
let pendingConfirm = null;
// The release history, as loaded so far, and the next page to load.
let releases = [];
let releasePage = 1;
let releasesMore = true;
let releasesFailed = false;

function toast(text) {
  clearTimeout(toastTimer);
  $("toast").textContent = String(text);
  $("toast").hidden = false;
  toastTimer = setTimeout(() => ($("toast").hidden = true), 6500);
}

function icon(name) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#${name}`);
  svg.append(use);
  return svg;
}

function element(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value == null) continue;
    if (key === "dataset") Object.assign(node.dataset, value);
    else if (key in node) node[key] = value;
    else node.setAttribute(key, value);
  }
  node.append(...children.filter((child) => child != null));
  return node;
}

async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Older web views: the selection route.
    const area = element("textarea", { value: text });
    document.body.append(area);
    area.select();
    document.execCommand("copy");
    area.remove();
  }
  toast("Copied.");
}

// ---------- drawing ----------

function drawRows(rows) {
  $("rows").replaceChildren(
    ...rows.map((row) =>
      element(
        "div",
        { dataset: { large: String(Boolean(row.large)) } },
        element("dt", {}, row.label),
        element("dd", {}, row.value),
      ),
    ),
  );
}

function drawForms(p) {
  const form = p.main.form === "connect" ? "connect" : openForm || p.main.form || null;
  $("connect-form").hidden = form !== "connect";
  $("create-form").hidden = form !== "create";
  $("join-form").hidden = form !== "join";
  $("server-field").hidden = p.connect.serverFixed;
  if (!offered && current) {
    $("server").value = p.connect.server;
    $("name").value = p.connect.name;
    offered = true;
  }
  if (!$("max-players").options.length) {
    for (let n = 2; n <= 16; n++) $("max-players").append(element("option", { value: String(n) }, `${n} players`));
    $("max-players").value = "4";
  }
  const names = p.rules.map((rule) => rule.name).join("|");
  if ($("rules").dataset.names !== names) {
    $("rules").replaceChildren(
      ...p.rules.map((rule, i) =>
        element("option", { value: rule.name, title: rule.description }, i === 0 ? `${rule.name} (default)` : rule.name),
      ),
    );
    $("rules").dataset.names = names;
  }
  return form;
}

function drawMain(p, form) {
  const main = $("main-action");
  // The join form turns the big button into "Join room".
  const joining = form === "join";
  const label = joining ? "Join room" : p.main.label;
  main.querySelector("span").textContent = busy && !p.main.progress ? "Working…" : label;
  main.querySelector("use").setAttribute("href", `#${joining ? "users" : p.main.icon}`);
  main.disabled = busy || Boolean(p.main.disabled);
  main.setAttribute("aria-busy", String(busy));
  if (p.main.progress != null) {
    main.dataset.progress = p.main.progress > 0 ? "known" : "unknown";
    main.style.setProperty("--progress", `${p.main.progress}%`);
  } else delete main.dataset.progress;
}

function drawSecondary(p, form) {
  const buttons = p.secondary.map((b) => {
    const button = element(
      "button",
      { type: "button", className: `quiet-button secondary-play${b.danger ? " danger" : ""}`, id: `secondary-${b.id}` },
      icon(b.icon),
      form === "join" && b.form === "join" ? "Back to creating a room" : b.label,
    );
    button.disabled = busy || Boolean(b.disabled);
    button.addEventListener("click", () => secondaryClicked(b));
    return button;
  });
  $("secondary-actions").replaceChildren(...buttons);
}

function drawStatus(p) {
  const line = document.querySelector(".operation-status");
  line.hidden = !p.status.text;
  line.dataset.tone = p.status.tone;
  document.querySelector(".connection-status").textContent = p.status.text;
  $("notes").hidden = !p.notes.length;
  $("notes").textContent = p.notes.join(" ");
}

function drawRoom(p) {
  const room = p.room;
  $("room-title").textContent = room ? "Your room" : "How to play";
  $("room-panel").dataset.inRoom = String(Boolean(room));
  $("room-heading").hidden = !room;
  $("steps").hidden = Boolean(room);
  $("members").hidden = !room;
  $("chat").hidden = !room;
  $("steps").replaceChildren(
    ...p.steps.map((step) => element("li", { dataset: { done: String(step.done) } }, step.label)),
  );
  if (room) {
    $("room-rules").textContent = room.rules;
    $("room-name").textContent = room.name;
    $("members").replaceChildren(
      ...room.members.map((m) => {
        const remove = m.removable
          ? element("button", { type: "button", className: "quiet-button" }, "Remove")
          : null;
        remove?.addEventListener("click", () =>
          confirmThen(`Remove ${m.name} from the room?`, "They cannot come back to this room.", "Remove them", {
            action: "kick",
            player: m.id,
          }),
        );
        return element(
          "li",
          {},
          element(
            "div",
            { className: "member-name" },
            element("strong", {}, m.you ? `${m.name} (you)` : m.name),
            element("span", {}, m.platform),
          ),
          element(
            "div",
            { className: "member-badges" },
            ...m.badges.map((b) => element("span", { className: "status-pill", dataset: { state: b.state } }, b.text)),
          ),
          remove,
        );
      }),
    );
  }
  $("differences").hidden = !p.differences.length;
  $("differences").replaceChildren(...p.differences.map((line) => element("p", {}, line)));
  const lines = $("chat-lines");
  const atEnd = lines.scrollTop + lines.clientHeight >= lines.scrollHeight - 4;
  if (lines.childElementCount !== p.chat.length) {
    lines.replaceChildren(
      ...p.chat.map((line) =>
        element("li", { dataset: { you: String(line.you) } }, element("strong", {}, line.from), line.text),
      ),
    );
    if (atEnd) lines.scrollTop = lines.scrollHeight;
  }
  $("session-log").hidden = !p.notices.length;
  $("notices").replaceChildren(...p.notices.map((text) => element("li", {}, text)));
}

// ---------- releases ----------

function drawNotes(container, markdown) {
  const blocks = notesBlocks(markdown);
  const nodes = [];
  let list = null;
  for (const block of blocks) {
    if (block.kind === "li") {
      if (!list) nodes.push((list = element("ul")));
      list.append(element("li", {}, block.text));
      continue;
    }
    list = null;
    nodes.push(element(block.kind === "h" ? "h4" : "p", {}, block.text));
  }
  container.replaceChildren(...(nodes.length ? nodes : [element("p", {}, "No notes for this release.")]));
}

function installVersion(release, p) {
  const older =
    release.version !== p.version &&
    release.version.localeCompare(p.version, undefined, { numeric: true }) < 0;
  confirmThen(
    `Install version ${release.version}?`,
    `TPF3-MP ${release.version} replaces ${p.version} and the launcher restarts into it.` +
      (older ? " An older TPF3-MP cannot join a server that runs a newer one." : "") +
      " Updates then wait until you resume them in Settings.",
    `Install ${release.version}`,
    null,
    (yes) => {
      if (yes) task(() => backend.installVersion(release.version));
    },
  );
}

function drawReleases(p) {
  $("release-panel").hidden = Boolean(p.room);
  $("release-track-label").textContent = trackName(p.track);
  const latest = latestRelease(releases, p.track);
  if (releasesFailed && !releases.length) {
    $("news-headline").textContent = "Release notes unavailable";
    $("release-notes").textContent = "Could not load the releases from GitHub. You can still play.";
  } else if (latest && $("release-notes").dataset.version !== latest.version) {
    $("news-headline").textContent = `Version ${latest.version}`;
    drawNotes($("release-notes"), latest.notes);
    $("release-notes").dataset.version = latest.version;
  }
  const shown = new Set([...$("history-list").children].map((item) => item.dataset.version));
  for (const release of releases) {
    if (shown.has(release.version)) continue;
    const summary = element(
      "summary",
      {},
      element(
        "span",
        {},
        `Version ${release.version}${release.dev ? " · Dev build" : release.experimental ? " · Experimental" : ""}`,
      ),
      element("time", { dateTime: release.date || "" }, dateText(release.date)),
    );
    const notes = element("div", { className: "historical-notes" });
    drawNotes(notes, release.notes);
    const install = element("button", { type: "button", className: "quiet-button history-install" });
    install.dataset.version = release.version;
    install.addEventListener("click", () => installVersion(release, present(current)));
    $("history-list").append(element("details", { dataset: { version: release.version } }, summary, notes, install));
  }
  // Dev builds come with every change to dev: only their track lists them.
  for (const item of $("history-list").children) {
    const release = releases.find((r) => r.version === item.dataset.version);
    item.hidden = Boolean(release?.dev) && p.track !== "dev";
  }
  for (const button of document.querySelectorAll(".history-install")) {
    const release = releases.find((r) => r.version === button.dataset.version);
    const installed = release.version === p.version;
    let label = `Install ${release.version}`;
    if (!release.installable) label = "Not signed for installing";
    else if (installed) label = p.held === release.version ? "Installed and held" : "Installed";
    button.textContent = label;
    button.disabled = busy || !release.installable || installed || !p.canInstall;
    button.title = p.room ? "Leave the room first." : "";
  }
  $("load-history").hidden = !releasesMore;
  $("load-history").disabled = busy;
  $("load-history").querySelector("span").textContent =
    releasePage > 2 ? "Load more releases" : "Browse previous releases";
  $("track-dev").hidden = !p.devTrack && p.track !== "dev";
  $("release-track").value = p.track;
  $("release-track").disabled = busy || !p.canInstall;
  $("dev-warning").hidden = p.track !== "dev";
  $("held-note").hidden = !p.held;
  $("held-copy").textContent = p.held
    ? `You chose version ${p.held}. Updates wait until you resume them.`
    : "";
}

async function loadReleases() {
  try {
    const page = await backend.releases(releasePage);
    releasesFailed = false;
    for (const release of page.releases) {
      if (!releases.some((r) => r.version === release.version)) releases.push(release);
    }
    releasePage += 1;
    releasesMore = page.more;
    $("history-status").textContent = releasesMore ? "" : "All releases loaded.";
  } catch (error) {
    releasesFailed = true;
    $("history-status").textContent = "Could not load the release history. Try again.";
  }
  if (current) render(current);
}

function drawGame(p) {
  $("game-build").textContent = p.game.build ? `Game folder · Steam build ${p.game.build}` : "Game folder";
  $("game-path").textContent = p.game.folder || "Transport Fever 3 was not found in Steam.";
  $("game-path").title = p.game.folder || "";
  $("mod-pill").textContent = p.game.mod ? `TPF3-MP mod ${p.game.mod}` : "TPF3-MP mod not installed";
  $("mod-pill").dataset.state = p.game.mod ? "ready" : "update";
  $("mod-pill").title = p.game.mod
    ? ""
    : "Run INSTALL_TPF3MP.cmd (Windows) or ./install.sh from the TPF3-MP folder once, then activate TPF3-MP in the game's Mod Hub.";
  $("open-folder").disabled = !p.game.folder;
}

function drawHeader(p, v) {
  const chip = $("server-chip");
  const state = v.launcher;
  const server = state.server_name || state.server;
  chip.hidden = !server;
  chip.textContent = server || "";
  chip.dataset.state = state.connection === "connected" ? "connected" : v.reach;
  $("launcher-update-badge").hidden = !p.update.badge;
  $("launcher-update-badge").textContent = p.update.badge || "";
  $("launcher-update-copy").textContent = p.update.copy;
  $("launcher-install").disabled = !p.update.installable || p.inGame;
  $("launcher-install").title = p.inGame ? "Leave the room first." : "";
  document.querySelector(".footer-version").textContent = `v${p.version}`;
  $("support").hidden = !p.supportId;
  $("support-id").textContent = p.supportId || "";
  $("diagnostics-group").hidden = p.diagnostics == null;
  $("diagnostics").value = p.diagnostics ? "on" : "off";
  $("about-server").textContent = server
    ? `${server}${state.server_version ? ` · ${state.server_version}` : ""}`
    : "—";
  $("about-game").textContent = state.installed ? `Steam build ${state.installed.build}` : "Not found";
  $("platform-name").textContent = { windows: "Windows", linux: "Linux", macos: "macOS" }[v.platform] || v.platform;
}

function render(v) {
  current = v;
  const p = present(v);
  $("status-pill").textContent = p.pill.text;
  $("status-pill").dataset.state = p.pill.state;
  drawRows(p.rows);
  const form = drawForms(p);
  drawMain(p, form);
  drawSecondary(p, form);
  drawStatus(p);
  drawRoom(p);
  drawGame(p);
  drawHeader(p, v);
  drawReleases(p);
  if (v.launcher.error && v.launcher.error !== lastError) toast(v.launcher.error);
  lastError = v.launcher.error;
  if (p.askQuit && !$("confirm-dialog").open) {
    confirmThen(
      "Quit TPF3-MP?",
      "You are in a room. Quitting takes you out of the game the others are playing.",
      "Quit",
      null,
      (yes) => backend.answerQuit(yes),
    );
  }
}

// ---------- acting ----------

async function task(work) {
  if (busy) return;
  busy = true;
  if (current) render(current);
  try {
    await work();
  } catch (error) {
    toast(error);
  } finally {
    busy = false;
    await refresh();
  }
}

function act(action) {
  return task(() => backend.act(action));
}

function text(id) {
  return $(id).value.trim();
}

function formAction(form) {
  if (form === "connect") {
    return {
      action: "connect",
      server: current.launcher.server_fixed ? current.launcher.server : text("server"),
      name: text("name"),
    };
  }
  if (form === "create") {
    return {
      action: "create",
      room: text("room-name-input"),
      max_players: Number($("max-players").value),
      password: text("create-password") || null,
      rules: $("rules").value || null,
    };
  }
  if (form === "join") {
    return { action: "join", invite: text("join-invite").toUpperCase(), password: text("join-password") || null };
  }
  return null;
}

async function mainClicked() {
  if (!current || busy) return;
  const p = present(current);
  const form = p.main.form === "connect" ? "connect" : openForm || p.main.form;
  if (p.main.command === "install_update") return task(() => backend.installUpdate());
  if (form) {
    const action = formAction(form);
    await task(async () => {
      await backend.act(action);
      // An invite given with the name joins right after connecting.
      const invite = form === "connect" ? text("connect-invite").toUpperCase() : "";
      if (invite) {
        for (let i = 0; i < 50 && (await backend.view()).launcher.connection === "connecting"; i++) {
          await new Promise((resolve) => setTimeout(resolve, 100));
        }
        if ((await backend.view()).launcher.connection === "connected") {
          await backend.act({ action: "join", invite, password: null });
        }
      }
      if (form === "join") openForm = null;
    });
    return;
  }
  if (p.main.action) await act(p.main.action);
}

function secondaryClicked(b) {
  if (b.form) {
    openForm = openForm === b.form ? null : b.form;
    render(current);
    return;
  }
  if (b.copy) return copy(b.copy);
  if (b.confirm === "leave") {
    return confirmThen("Leave the room?", "You can come back with its invite while it is open.", "Leave", {
      action: "leave",
    });
  }
  if (b.action) act(b.action);
}

function confirmThen(title, message, yes, action, answered) {
  $("confirm-title").textContent = title;
  $("confirm-message").textContent = message;
  $("confirm-yes").textContent = yes;
  pendingConfirm = { action, answered };
  $("confirm-dialog").showModal();
}

// ---------- the launcher, or the preview ----------

function tauriBackend() {
  const invoke = window.__TAURI__.core.invoke;
  return {
    view: () => invoke("view"),
    act: (action) => invoke("act", { action }),
    ready: () => invoke("window_ready"),
    checkUpdate: () => invoke("check_update"),
    installUpdate: () => invoke("install_update"),
    openGameFolder: () => invoke("open_game_folder"),
    answerQuit: (quit) => invoke("answer_quit", { quit }),
    releases: (page) => invoke("releases", { page }),
    installVersion: (version) => invoke("install_version", { version }),
    setTrack: (track) => invoke("set_track", { track }),
    resumeUpdates: () => invoke("resume_updates"),
  };
}

async function refresh() {
  try {
    render(await backend.view());
  } catch (error) {
    document.querySelector(".connection-status").textContent = `The launcher does not answer: ${error}`;
    document.querySelector(".operation-status").hidden = false;
  }
}

async function start() {
  if (window.__TAURI__?.core) {
    backend = tauriBackend();
  } else {
    const { previewBackend } = await import("./preview.js");
    backend = previewBackend();
  }
  $("main-action").addEventListener("click", mainClicked);
  for (const id of ["connect-form", "create-form", "join-form"]) {
    $(id).addEventListener("submit", (event) => {
      event.preventDefault();
      mainClicked();
    });
  }
  $("chat-form").addEventListener("submit", (event) => {
    event.preventDefault();
    const said = text("chat-text");
    if (!said) return;
    $("chat-text").value = "";
    act({ action: "chat", text: said });
  });
  ["open-settings", "launcher-update-badge"].forEach((id) =>
    $(id).addEventListener("click", () => $("settings-dialog").showModal()),
  );
  document.querySelectorAll(".close-dialog").forEach((button) =>
    button.addEventListener("click", () => button.closest("dialog").close()),
  );
  $("confirm-dialog").addEventListener("close", () => {
    const pending = pendingConfirm;
    pendingConfirm = null;
    const yes = $("confirm-dialog").returnValue === "yes";
    $("confirm-dialog").returnValue = "";
    pending?.answered?.(yes);
    if (yes && pending?.action) act(pending.action);
  });
  $("confirm-yes").addEventListener("click", () => $("confirm-dialog").close("yes"));
  $("launcher-check").addEventListener("click", () => task(() => backend.checkUpdate()));
  $("launcher-install").addEventListener("click", () => task(() => backend.installUpdate()));
  $("open-folder").addEventListener("click", () => task(() => backend.openGameFolder()));
  $("copy-support").addEventListener("click", () => copy($("support-id").textContent));
  $("diagnostics").addEventListener("change", (event) =>
    act({ action: "diagnostics", on: event.target.value === "on" }),
  );
  $("load-history").addEventListener("click", () => task(loadReleases));
  $("release-track").addEventListener("change", (event) => {
    const track = event.target.value;
    task(async () => {
      await backend.setTrack(track);
      $("track-feedback").textContent = "Saved. Nothing was installed.";
      delete $("release-notes").dataset.version;
    });
  });
  $("resume-updates").addEventListener("click", () =>
    task(async () => {
      await backend.resumeUpdates();
      toast("Updates resumed.");
    }),
  );
  await refresh();
  loadReleases();
  document.documentElement.dataset.ready = "true";
  backend.ready?.();
  setInterval(() => {
    if (!busy && !document.hidden) refresh();
  }, POLL);
}

start();
