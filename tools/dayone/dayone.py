#!/usr/bin/env python3
"""dayone.py -- the release-day checks for Transport Fever 3 (docs/PLAN.md,
Part 1; docs/DAY_ONE.md). Standard library only; nothing here launches the
game or changes its folder (AGENTS.md): it reads, hashes, copies out, and
installs the probe mods where Steam keeps a player's own mods.

    python tools/dayone/dayone.py find              where the game is; our guesses checked
    python tools/dayone/dayone.py archive           hashes and a private copy of the build
    python tools/dayone/dayone.py gonogo            anti-tamper in the executable
    python tools/dayone/dayone.py check-launch      how the running game was started
    python tools/dayone/dayone.py decode            tpfre over the executable: hook targets
    python tools/dayone/dayone.py scripts           the game's .tl sources: command factories
    python tools/dayone/dayone.py probes install    the probe mods, into the mods folder
    python tools/dayone/dayone.py collect           the probes' output, out of the game log
    python tools/dayone/dayone.py compare A B       two determinism logs, first divergence

Every command takes --game DIR (the game's folder) and --steam DIR (Steam's)
to skip the search, and writes a Markdown report under --out (default
investigation/dayone-<date>/) to paste into the recon report
(investigation/TPF3_RECON_TEMPLATE.md). Each prints a verdict line:
"GO", "CHECK" (a person must look) or "STOP" (the plan does not hold).
"""

from __future__ import annotations

import argparse
import datetime as _dt
import hashlib
import json
import math
import os
import re
import shutil
import struct
import subprocess
import sys
import zlib
from dataclasses import dataclass, field
from pathlib import Path

APP_ID = "3493540"
REPO = Path(__file__).resolve().parents[2]
# What crates/tpf3mp-launch's find_executable looks for, and
# crates/tpf3mp-agent/src/logs.rs's game_candidates_in: the two guesses
# release day confirms or corrects (PLAN.md Part 1).
LAUNCH_NAMES_WINDOWS = ["TransportFever3.exe", "Transport Fever 3.exe"]
LAUNCH_NAMES_UNIX = ["TransportFever3", "Transport Fever 3"]
HELPER_WORDS = ["crash", "redist", "setup", "unins", "launcher", "helper"]
# Protectors known by their section names. SteamStub alone is what TPF2 had
# and what the plan assumes.
PROTECTORS = {
    ".bind": "SteamStub (Steam DRM)",
    ".vmp0": "VMProtect", ".vmp1": "VMProtect", ".vmp2": "VMProtect",
    ".themida": "Themida", ".winlice": "WinLicense",
    ".enigma1": "Enigma Protector", ".enigma2": "Enigma Protector",
    ".arxan": "Arxan", "UPX0": "UPX", "UPX1": "UPX", ".aspack": "ASPack",
}
PROBES = {"api": "tpf3mp_apidump_1", "run": "tpf3mp_rundump_1", "det": "tpf3mp_detprobe_1"}
# The targets the hook profile needs, by their TPF2 names
# (investigation/tpf2-baseline/tpf2_known_rvas.txt, HOOKS.md).
TARGETS = [
    ("the simulation step", r"GameSim::Step\b"),
    ("the game's step", r"CGame::Step\b"),
    ("the simulation loop", r"CGame::RunGameSimLoop"),
    ("the command queue's add", r"CommandList::Add"),
    ("the command factories", r"make_cmd::"),
    ("game speed", r"CGameTime::"),
    ("starting a savegame", r"CMenuUI::StartSavegame"),
    ("menu pages", r"CMenuUI::CreatePage"),
    ("saving", r"(?i)save(game)?::|::Save\b|SaveGame"),
    ("loading", r"(?i)::Load(Game)?\b|LoadGame"),
]
TARGET_FILES = ["gamesim.cpp", "gametime.cpp", "commandlist.cpp", "make_command.cpp", "savegame"]


def say(text: str = "") -> None:
    print(text, flush=True)


# ---------- finding Steam and the game ----------

def steam_roots(explicit: Path | None) -> list[Path]:
    """Steam's folders on this computer, the registry's first on Windows."""
    if explicit:
        return [explicit]
    roots: list[Path] = []
    if sys.platform == "win32":
        try:
            import winreg  # noqa: PLC0415 (Windows only)
            for hive, key in [(winreg.HKEY_CURRENT_USER, r"Software\Valve\Steam"),
                              (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam")]:
                try:
                    with winreg.OpenKey(hive, key) as k:
                        for name in ("SteamPath", "InstallPath"):
                            try:
                                roots.append(Path(winreg.QueryValueEx(k, name)[0]))
                            except OSError:
                                pass
                except OSError:
                    pass
        except ImportError:
            pass
        roots.append(Path(r"C:\Program Files (x86)\Steam"))
    elif sys.platform == "darwin":
        roots.append(Path.home() / "Library/Application Support/Steam")
    else:
        roots += [Path.home() / ".steam/steam", Path.home() / ".local/share/Steam",
                  Path.home() / ".var/app/com.valvesoftware.Steam/.local/share/Steam"]
    seen, out = set(), []
    for root in roots:
        key = str(root).lower()
        if key not in seen and root.is_dir():
            seen.add(key)
            out.append(root)
    return out


def libraries(root: Path) -> list[Path]:
    """Steam's libraries: its own folder and those libraryfolders.vdf names."""
    libs = [root]
    vdf = root / "steamapps" / "libraryfolders.vdf"
    if vdf.is_file():
        for match in re.finditer(r'"path"\s+"([^"]+)"', vdf.read_text(encoding="utf-8", errors="replace")):
            libs.append(Path(match.group(1).replace("\\\\", "\\")))
    return libs


def parse_acf(text: str) -> dict:
    """Valve's KeyValues text, as appmanifest files use it, into dicts."""
    tokens = re.findall(r'"((?:[^"\\]|\\.)*)"|([{}])', text)
    stack: list[dict] = [{}]
    key = None
    for quoted, brace in tokens:
        if brace == "{":
            new: dict = {}
            stack[-1][key] = new
            stack.append(new)
            key = None
        elif brace == "}":
            if len(stack) > 1:
                stack.pop()
        elif key is None:
            key = quoted
        else:
            stack[-1][key] = quoted
            key = None
    return stack[0]


@dataclass
class Game:
    steam: Path | None
    library: Path | None
    folder: Path
    manifest: dict = field(default_factory=dict)

    @property
    def build(self) -> str:
        return str(self.manifest.get("buildid", "unknown"))


def find_game(game: Path | None, steam: Path | None) -> Game | None:
    if game:
        manifest = {}
        roots = steam_roots(steam)
        steam = steam or (roots[0] if roots else None)
        for root in roots:
            for lib in libraries(root):
                acf = lib / "steamapps" / f"appmanifest_{APP_ID}.acf"
                if acf.is_file():
                    manifest = parse_acf(acf.read_text(encoding="utf-8", errors="replace")).get("AppState", {})
        return Game(steam, None, game, manifest)
    for root in steam_roots(steam):
        for lib in libraries(root):
            acf = lib / "steamapps" / f"appmanifest_{APP_ID}.acf"
            if not acf.is_file():
                continue
            manifest = parse_acf(acf.read_text(encoding="utf-8", errors="replace")).get("AppState", {})
            folder = lib / "steamapps" / "common" / manifest.get("installdir", "Transport Fever 3")
            if folder.is_dir():
                return Game(root, lib, folder, manifest)
    return None


def kind_of(path: Path) -> str | None:
    """'pe', 'elf' or 'macho' by the file's first bytes, else None."""
    try:
        with open(path, "rb") as f:
            head = f.read(4)
    except OSError:
        return None
    if head[:2] == b"MZ":
        return "pe"
    if head == b"\x7fELF":
        return "elf"
    if head in (b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xfe\xed\xfa\xcf"):
        return "macho"
    return None


def executables(folder: Path) -> list[Path]:
    """The programs at the top of the game's folder (and in a macOS .app)."""
    found = []
    for path in sorted(folder.iterdir()):
        if path.is_file() and path.suffix.lower() not in (".dll", ".so", ".dylib", ".pdb"):
            kind = kind_of(path)
            if kind and not (kind == "pe" and path.suffix.lower() != ".exe"):
                found.append(path)
        elif path.suffix == ".app" and (path / "Contents/MacOS").is_dir():
            found += [p for p in sorted((path / "Contents/MacOS").iterdir()) if kind_of(p) == "macho"]
    return found


def what_launch_would_pick(folder: Path, windows: bool) -> Path | None:
    """crates/tpf3mp-launch's find_executable, the same rule."""
    names = LAUNCH_NAMES_WINDOWS if windows else LAUNCH_NAMES_UNIX
    for name in names:
        if (folder / name).is_file():
            return folder / name
    if not windows:
        return None
    exes = [p for p in folder.iterdir() if p.is_file() and p.suffix.lower() == ".exe"
            and not any(w in p.name.lower() for w in HELPER_WORDS)]
    return exes[0] if len(exes) == 1 else None


def log_places(game: Game) -> list[tuple[str, Path]]:
    """Every place a log or crash dump might be, the TPF2 guess first."""
    places = []
    roots = [game.steam] if game.steam else []
    for root in roots:
        userdata = root / "userdata"
        if userdata.is_dir():
            for account in sorted(userdata.iterdir()):
                local = account / APP_ID / "local"
                places.append(("TPF2's place: userdata/<account>/%s/local" % APP_ID, local))
    places.append(("the game's folder", game.folder))
    home = Path.home()
    for label, path in [
        ("%LOCALAPPDATA%", Path(os.environ.get("LOCALAPPDATA", home / "AppData/Local"))),
        ("%APPDATA%", Path(os.environ.get("APPDATA", home / "AppData/Roaming"))),
        ("Documents", home / "Documents"),
        ("~/.local/share", home / ".local/share"),
        ("~/Library/Application Support", home / "Library/Application Support"),
        ("~/Library/Logs", home / "Library/Logs"),
    ]:
        for name in ("Transport Fever 3", "TransportFever3", "Urban Games"):
            if (path / name).exists():
                places.append((f"{label}/{name}", path / name))
    return places


LOG_PATTERN = re.compile(r"(?i)(stdout.*\.txt|.*\.log|.*\.dmp|.*crash.*)$")


def logs_in(place: Path, depth: int = 3) -> list[Path]:
    found: list[Path] = []
    if not place.is_dir():
        return found

    def walk(folder: Path, level: int) -> None:
        try:
            entries = sorted(folder.iterdir())
        except OSError:
            return
        for entry in entries:
            if entry.is_file() and LOG_PATTERN.match(entry.name):
                found.append(entry)
            elif entry.is_dir() and level < depth and len(found) < 200:
                walk(entry, level + 1)
    walk(place, 0)
    return found


def mods_folders(game: Game) -> list[Path]:
    """Where a player's own mods go: staging_area (TF3, reported) and
    mods (TPF2), per Steam account."""
    out = []
    if game.steam and (game.steam / "userdata").is_dir():
        for account in sorted((game.steam / "userdata").iterdir()):
            local = account / APP_ID / "local"
            if local.is_dir():
                out += [local / "staging_area", local / "mods"]
    return out


# ---------- executables: hashes and headers ----------

def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


@dataclass
class Section:
    name: str
    vaddr: int
    vsize: int
    raw_ptr: int
    raw_size: int
    characteristics: int
    entropy: float = 0.0

    @property
    def executable(self) -> bool:
        return bool(self.characteristics & 0x20000000)


@dataclass
class PeInfo:
    machine: int
    timestamp: int
    image_base: int
    sections: list[Section]
    imports: list[str]
    tls_callbacks: int


def entropy(data: bytes) -> float:
    if not data:
        return 0.0
    counts = [0] * 256
    for byte in data:
        counts[byte] += 1
    n = len(data)
    return -sum(c / n * math.log2(c / n) for c in counts if c)


def sample(data: bytes, limit: int = 1 << 20) -> bytes:
    """Up to `limit` bytes, in 16 pieces spread over `data`."""
    if len(data) <= limit:
        return data
    piece = limit // 16
    step = len(data) // 16
    return b"".join(data[i * step:i * step + piece] for i in range(16))


def read_pe(path: Path) -> PeInfo:
    data = path.read_bytes()
    if data[:2] != b"MZ":
        raise ValueError("not a PE file")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("no PE header")
    machine, nsections, timestamp = struct.unpack_from("<HHI", data, pe + 4)
    opt_size = struct.unpack_from("<H", data, pe + 20)[0]
    opt = pe + 24
    magic = struct.unpack_from("<H", data, opt)[0]
    if magic == 0x20B:
        image_base = struct.unpack_from("<Q", data, opt + 24)[0]
        dirs = opt + 112
        ptr = 8
    else:
        image_base = struct.unpack_from("<I", data, opt + 28)[0]
        dirs = opt + 96
        ptr = 4
    ndirs = struct.unpack_from("<I", data, dirs - 4)[0]

    def directory(index: int) -> tuple[int, int]:
        if index >= ndirs:
            return 0, 0
        return struct.unpack_from("<II", data, dirs + index * 8)

    sections = []
    table = opt + opt_size
    for i in range(nsections):
        at = table + i * 40
        name = data[at:at + 8].rstrip(b"\0").decode("latin-1")
        vsize, vaddr, raw_size, raw_ptr = struct.unpack_from("<IIII", data, at + 8)
        chars = struct.unpack_from("<I", data, at + 36)[0]
        s = Section(name, vaddr, vsize, raw_ptr, raw_size, chars)
        s.entropy = entropy(sample(data[raw_ptr:raw_ptr + raw_size]))
        sections.append(s)

    def offset(rva: int) -> int | None:
        for s in sections:
            if s.vaddr <= rva < s.vaddr + max(s.vsize, s.raw_size):
                return s.raw_ptr + (rva - s.vaddr)
        return None

    imports = []
    imp_rva, _ = directory(1)
    at = offset(imp_rva) if imp_rva else None
    while at is not None and at + 20 <= len(data):
        name_rva = struct.unpack_from("<I", data, at + 12)[0]
        if name_rva == 0:
            break
        name_at = offset(name_rva)
        if name_at is not None:
            end = data.find(b"\0", name_at)
            imports.append(data[name_at:end].decode("latin-1"))
        at += 20
        if len(imports) > 512:
            break

    callbacks = 0
    tls_rva, _ = directory(9)
    tls_at = offset(tls_rva) if tls_rva else None
    if tls_at is not None:
        cb_va = struct.unpack_from("<Q" if ptr == 8 else "<I", data, tls_at + 3 * ptr)[0]
        cb_at = offset(cb_va - image_base) if cb_va else None
        while cb_at is not None and callbacks < 64:
            value = struct.unpack_from("<Q" if ptr == 8 else "<I", data, cb_at)[0]
            if value == 0:
                break
            callbacks += 1
            cb_at += ptr
    return PeInfo(machine, timestamp, image_base, sections, imports, callbacks)


@dataclass
class Verdict:
    word: str  # GO, CHECK, STOP
    reasons: list[str]


def judge_pe(info: PeInfo) -> Verdict:
    """Anti-tamper, as the plan needs it known (PLAN.md Part 1)."""
    reasons, stop, check = [], False, False
    names = [s.name for s in info.sections]
    found = sorted({PROTECTORS[n] for n in names if n in PROTECTORS})
    for protector in found:
        if protector.startswith("SteamStub"):
            reasons.append("SteamStub (.bind): as TPF2 had; the hook loads into the unpacked process")
        else:
            stop = True
            reasons.append(f"{protector}: a protector the native plan does not expect")
    known = {".text", ".rdata", ".data", ".pdata", ".rsrc", ".reloc", ".tls", "_RDATA", ".00cfg", ".gfids", ".idata", ".didat", ".bind", ".retplne", ".voltbl", "_guard"}
    for s in info.sections:
        if s.executable and s.entropy > 7.2 and s.name != ".bind":
            check = True
            reasons.append(f"executable section {s.name!r} has entropy {s.entropy:.2f}: packed or encrypted code")
        if s.name not in known and s.name not in PROTECTORS and s.raw_size > (1 << 20):
            check = True
            reasons.append(f"unexplained section {s.name!r} of {s.raw_size >> 20} MiB (entropy {s.entropy:.2f})")
    if info.tls_callbacks:
        # TPF2's executable had 3 (SteamStub's and the C runtime's), and
        # the hook worked: a note, not a finding.
        reasons.append(f"{info.tls_callbacks} TLS callback(s) (TPF2 had 3): if the hook fails to load, try the loader-first fallback (DAY_ONE.md section 5)")
    if not reasons:
        reasons.append("no protector found: plain executable")
    return Verdict("STOP" if stop else "CHECK" if check else "GO", reasons)


# ---------- the report ----------

class Report:
    def __init__(self, title: str):
        self.lines = [f"# {title}", "", f"Made by tools/dayone/dayone.py on {_dt.datetime.now():%Y-%m-%d %H:%M}.", ""]

    def add(self, text: str = "") -> None:
        self.lines.append(text)
        say(text)

    def save(self, folder: Path, name: str) -> Path:
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / name
        path.write_text("\n".join(self.lines) + "\n", encoding="utf-8")
        say(f"\nwritten: {path}")
        return path


def out_dir(args) -> Path:
    if args.out:
        return Path(args.out)
    return REPO / "investigation" / f"dayone-{_dt.date.today():%Y-%m-%d}"


def need_game(args) -> Game:
    game = find_game(Path(args.game) if args.game else None, Path(args.steam) if args.steam else None)
    if game is None:
        say("STOP: Transport Fever 3 (Steam app %s) was not found; pass --game DIR" % APP_ID)
        sys.exit(2)
    return game


# ---------- commands ----------

def cmd_find(args) -> int:
    game = need_game(args)
    windows = args.windows if args.windows is not None else sys.platform == "win32"
    r = Report("Transport Fever 3 on this computer")
    r.add(f"- Game folder: `{game.folder}`")
    r.add(f"- Steam build: {game.build}")
    depots = game.manifest.get("InstalledDepots", {})
    for depot, info in sorted(depots.items()):
        r.add(f"- Depot {depot}: manifest {info.get('manifest', '?')}, size {info.get('size', '?')}")
    r.add("")
    r.add("## Executables")
    exes = executables(game.folder)
    for exe in exes:
        r.add(f"- `{exe.name}` ({kind_of(exe)}, {exe.stat().st_size:,} bytes)")
    picked = what_launch_would_pick(game.folder, windows)
    r.add("")
    r.add("## Guess 1: the executable's name (crates/tpf3mp-launch, find_executable)")
    names = LAUNCH_NAMES_WINDOWS if windows else LAUNCH_NAMES_UNIX
    verdict_exe = "GO"
    if picked and picked.name in names:
        r.add(f"GO: the launcher finds `{picked.name}` by its expected name. Drop the TODO(TF3 release).")
    elif picked:
        verdict_exe = "CHECK"
        r.add(f"CHECK: the launcher would take `{picked.name}` as the only program there, not by name. Add it to find_executable's names.")
    else:
        verdict_exe = "STOP"
        r.add(f"STOP: the launcher would find no game. Add the real name to find_executable ({', '.join(e.name for e in exes) or 'no executables found'}); until then, start with --game-exe.")
    scripts = sorted(p.name for p in game.folder.iterdir() if p.suffix in (".sh", ".bat", ".cmd"))
    if scripts:
        r.add(f"Start scripts next to it: {', '.join(scripts)}: see whether the game must be started through one (DAY_ONE.md section 5, Linux).")
    r.add("")
    r.add("## Guess 2: the game's log and crash dumps (crates/tpf3mp-agent/src/logs.rs)")
    any_found = False
    for label, place in log_places(game):
        files = logs_in(place)
        if files:
            any_found = True
            r.add(f"- {label}: `{place}`")
            for f in files[:12]:
                r.add(f"  - `{f.relative_to(place)}` ({f.stat().st_size:,} bytes, {_dt.datetime.fromtimestamp(f.stat().st_mtime):%Y-%m-%d %H:%M})")
    if not any_found:
        r.add("CHECK: no log found yet: start the game once, then run find again.")
    r.add("Correct game_candidates_in to the places above, and drop the TPF2 mark from those confirmed.")
    r.add("")
    r.add("## Mods folders (packaging/*/install)")
    for folder in mods_folders(game):
        r.add(f"- `{folder}`: {'there' if folder.is_dir() else 'not there'}")
    r.add("")
    r.add(f"Verdict: {verdict_exe}")
    r.save(out_dir(args), "1-find.md")
    if args.json:
        print(json.dumps({"folder": str(game.folder), "build": game.build,
                          "executables": [str(e) for e in exes],
                          "picked": str(picked) if picked else None}, indent=2))
    return 0 if verdict_exe != "STOP" else 1


def cmd_archive(args) -> int:
    game = need_game(args)
    to = Path(args.to) if args.to else Path.home() / "TPF3-MP-builds" / game.build
    r = Report(f"Build {game.build}, archived")
    record = {"build": game.build, "folder": str(game.folder), "depots": game.manifest.get("InstalledDepots", {}), "files": []}
    programs = executables(game.folder)
    targets = programs + sorted(p for p in game.folder.iterdir()
                                if p.is_file() and p.suffix.lower() in (".dll", ".so", ".dylib"))
    to.mkdir(parents=True, exist_ok=True)
    for path in targets:
        digest = sha256(path)
        entry = {"name": path.name, "size": path.stat().st_size, "sha256": digest}
        if kind_of(path) == "pe":
            try:
                info = read_pe(path)
                entry["pe_timestamp"] = info.timestamp
                entry["machine"] = hex(info.machine)
                entry["image_size_sections"] = sum(s.vsize for s in info.sections)
            except (ValueError, struct.error) as error:
                entry["pe_error"] = str(error)
        record["files"].append(entry)
        r.add(f"- `{path.name}` {entry['size']:,} bytes, sha256 `{digest}`")
        if path in programs:
            copy = to / path.name
            if copy.exists() and sha256(copy) != digest:
                r.add(f"STOP: `{copy}` exists with other contents: a patched build under the same build ID? Nothing overwritten.")
                return 1
            if not copy.exists():
                shutil.copy2(path, copy)
    (to / "build.json").write_text(json.dumps(record, indent=2), encoding="utf-8")
    r.add("")
    r.add(f"Executables copied to `{to}` with build.json. Keep them out of the repository (they are Urban Games').")
    r.add("Verdict: GO")
    r.save(out_dir(args), "2-archive.md")
    return 0


def cmd_gonogo(args) -> int:
    game = need_game(args) if not args.exe else None
    exes = [Path(args.exe)] if args.exe else executables(game.folder)
    r = Report("Go or no-go: anti-tamper")
    worst = "GO"
    for exe in exes:
        r.add(f"## `{exe.name}`")
        if kind_of(exe) != "pe":
            r.add(f"CHECK: a {kind_of(exe)} binary: run tools/re/binary_survey.py on it (packers, codesigning).")
            worst = "CHECK" if worst == "GO" else worst
            continue
        info = read_pe(exe)
        r.add(f"- machine {hex(info.machine)}, PE timestamp {info.timestamp}, image base {hex(info.image_base)}")
        for s in info.sections:
            r.add(f"  - `{s.name}` {s.raw_size:,} bytes, entropy {s.entropy:.2f}{' (code)' if s.executable else ''}")
        r.add(f"- imports: {', '.join(info.imports[:40])}{' ...' if len(info.imports) > 40 else ''}")
        verdict = judge_pe(info)
        for reason in verdict.reasons:
            r.add(f"- {reason}")
        r.add(f"Verdict for {exe.name}: {verdict.word}")
        order = ["GO", "CHECK", "STOP"]
        worst = max(worst, verdict.word, key=order.index)
        r.add("")
    r.add("## The launcher's start (by hand; nothing here starts the game)")
    r.add("1. Steam running, start the game from the TPF3-MP launcher (Start Transport Fever 3) in a room.")
    r.add("2. While it runs: `python tools/dayone/dayone.py check-launch`.")
    r.add("3. In the game: signed in (your Steam name shows), the Workshop / Mod Hub works, and it did not close and restart.")
    r.add("")
    r.add(f"Verdict: {worst}")
    r.save(out_dir(args), "1-gonogo.md")
    return 0 if worst != "STOP" else 1


def processes() -> list[dict]:
    """Every process: pid, parent, name, command line. Read only."""
    if sys.platform == "win32":
        ps = ("Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,CommandLine | "
              "ConvertTo-Json -Compress")
        out = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True, check=False).stdout
        rows = json.loads(out or "[]")
        return [{"pid": row["ProcessId"], "ppid": row["ParentProcessId"], "name": row["Name"] or "",
                 "cmd": row.get("CommandLine") or ""} for row in rows]
    rows = []
    out = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,comm=,args="], capture_output=True, text=True, check=False).stdout
    for line in out.splitlines():
        parts = line.split(None, 3)
        if len(parts) >= 3:
            rows.append({"pid": int(parts[0]), "ppid": int(parts[1]), "name": Path(parts[2]).name,
                         "cmd": parts[3] if len(parts) > 3 else ""})
    return rows


def judge_launch(procs: list[dict], game_names: list[str]) -> tuple[str, list[str]]:
    """How the running game was started, from the process table."""
    by_pid = {p["pid"]: p for p in procs}
    wanted = {n.lower() for n in game_names}
    games = [p for p in procs if p["name"].lower() in wanted]
    if not games:
        return "CHECK", ["no game process running: start it from the launcher first"]
    lines, word = [], "GO"
    for g in games:
        parent = by_pid.get(g["ppid"], {"name": "?(gone)"})
        pname = parent["name"].lower()
        lines.append(f"{g['name']} pid {g['pid']}, started by {parent['name']} (pid {g['ppid']})")
        if "tpf3mp" in pname:
            lines.append("the launcher started it: the hook can be in it (D11)")
        elif "steam" in pname:
            word = "STOP"
            lines.append("Steam started it: either it was started from Steam, or it restarted itself through Steam and lost the hook (DAY_ONE.md section 5)")
        else:
            word = "CHECK"
            lines.append("started by something else: a start script? The hook needs the launcher as its parent (TPF3MP_LAUNCHER_PID)")
    return word, lines


def data_dir() -> Path:
    if sys.platform == "win32":
        return Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData/Local")) / "TPF3-MP"
    if sys.platform == "darwin":
        return Path.home() / "Library/Application Support/TPF3-MP"
    return Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "TPF3-MP"


def cmd_check_launch(args) -> int:
    names = args.name or (LAUNCH_NAMES_WINDOWS if sys.platform == "win32" else LAUNCH_NAMES_UNIX)
    if not args.name and (game := find_game(Path(args.game) if args.game else None, Path(args.steam) if args.steam else None)):
        names = list(dict.fromkeys(names + [e.name for e in executables(game.folder)]))
    word, lines = judge_launch(processes(), names)
    r = Report("Go or no-go: how the running game was started")
    for line in lines:
        r.add(f"- {line}")
    hook = data_dir() / "hook.log"
    r.add("")
    if hook.is_file():
        tail = hook.read_text(encoding="utf-8", errors="replace").splitlines()[-15:]
        r.add(f"## The hook's log ({hook}), its end")
        r.add("```")
        for line in tail:
            r.add(line)
        r.add("```")
        r.add("It should name the build and say it is waiting for, or linked to, the launcher.")
    else:
        r.add(f"CHECK: no hook log at {hook}: the hook did not start in this game.")
        word = "CHECK" if word == "GO" else word
    r.add("")
    r.add(f"Verdict: {word}")
    r.save(out_dir(args), "1-check-launch.md")
    return 0 if word != "STOP" else 1


def tpfre_path(explicit: str | None) -> Path | None:
    if explicit:
        return Path(explicit)
    exe = "tpfre.exe" if sys.platform == "win32" else "tpfre"
    built = REPO / "tools" / "tpfre" / "target" / "release" / exe
    return built if built.is_file() else None


def run_tool(argv: list[str]) -> str:
    done = subprocess.run(argv, capture_output=True, text=True, check=False)
    return (done.stdout or "") + (("\n" + done.stderr) if done.returncode and done.stderr else "")


def carry_names(source: Path, db: Path, folder: Path, run, report: Report) -> None:
    """TF3 has no __FUNCSIG__ strings, which named TPF2's functions: carry
    an older build's names over with `tpfre match` (strings, RTTI slots,
    the call graph and source-file order). `source` is that build's
    executable or its .tpfdb."""
    old_db = source
    if source.suffix.lower() != ".tpfdb":
        old_db = folder / f"{source.stem}.names-from.tpfdb"
        say(f"indexing {source} for its names ...")
        say(run(["index", str(source), "-o", str(old_db)]).strip())
    say(f"matching {old_db.name} onto {db.name} ...")
    out = run(["match", str(old_db), str(db), "--limit", "0"]).strip()
    report.add(f"Names carried from `{source.name}` (tpfre match): {out.splitlines()[0] if out else 'nothing'}")
    report.add("A carried name is shown with source `matched`; check it against the function's source file.")
    report.add("")


def decode_report(exe: Path, db: Path, run, report: Report) -> str:
    """The hook's targets, looked up in tpfre's index of `exe`."""
    word = "GO"
    report.add(f"```\n{run(['q', str(db), 'info']).strip()}\n```")
    report.add("")
    for label, pattern in TARGETS:
        report.add(f"## {label} (`{pattern}`)")
        found = "\n".join(run(["q", str(db), "names", pattern]).strip().splitlines()[:25])
        if not found or "no match" in found.lower():
            report.add("CHECK: nothing by this name: look by source file and strings below.")
            word = "CHECK"
        else:
            report.add(f"```\n{found}\n```")
        report.add("")
    report.add("## By source file (`__FILE__`)")
    for name in TARGET_FILES:
        found = run(["q", str(db), "file", name]).strip()
        report.add(f"### {name}")
        report.add(f"```\n{found[:4000]}\n```" if found else "nothing")
    report.add("")
    report.add("Then, for each target found: `tpfre q DB sig <name> --toml` gives its [[target]] block; "
               "`tools/re/make_profile.py` writes the whole profile (DAY_ONE.md, release-day order 1).")
    return word


def cmd_decode(args) -> int:
    game = need_game(args) if not args.exe else None
    exe = Path(args.exe) if args.exe else next((e for e in executables(game.folder) if kind_of(e) == "pe"), None)
    if exe is None:
        say("CHECK: no Windows executable to index; for Linux or macOS use tools/re/name_functions.py")
        return 1
    tpfre = tpfre_path(args.tpfre)
    if tpfre is None:
        say("CHECK: tpfre is not built. Build it once:")
        say("  cargo build --release --manifest-path tools/tpfre/Cargo.toml")
        return 1
    folder = out_dir(args)
    folder.mkdir(parents=True, exist_ok=True)
    db = folder / f"{exe.stem}.tpfdb"
    say(f"indexing {exe} ...")
    say(run_tool([str(tpfre), "index", str(exe), "-o", str(db)]).strip())
    r = Report(f"Hook targets in {exe.name}")
    if args.names_from:
        carry_names(Path(args.names_from), db, folder, lambda argv: run_tool([str(tpfre)] + argv), r)
    word = decode_report(exe, db, lambda argv: run_tool([str(tpfre)] + argv), r)
    r.add(f"Verdict: {word}")
    r.save(folder, "3-decode.md")
    return 0


# TF3's api.cmd.makeLineCreateCmd, and TPF2's api.cmd.make.buildProposal.
CMD_PATTERN = re.compile(r"\bapi\.cmd\.(make\w*Cmd|make\.\w+)\b")
DECL_PATTERN = re.compile(r"\b(make\w*Cmd)\s*:")


# TF3's content archives (base/content/*.zip) are zips whose local file
# headers start "UG\x03\x04" instead of "PK\x03\x04"; the central directory
# is a plain zip's. Seen in build 40408's download, 2026-09-29.
ZIP_LOCAL_MAGICS = (b"PK\x03\x04", b"UG\x03\x04")


def archive_scripts(path: Path):
    """The .tl and .lua files inside a zip or TF3 content archive, as (name,
    text). Raises ValueError for an archive it cannot read whole."""
    data = path.read_bytes()
    end = data.rfind(b"PK\x05\x06")
    if end < 0:
        raise ValueError("no end of central directory")
    count, _size, at = struct.unpack_from("<HII", data, end + 10)
    for _ in range(count):
        if data[at:at + 4] != b"PK\x01\x02":
            raise ValueError(f"bad central directory entry at {at}")
        method = struct.unpack_from("<H", data, at + 10)[0]
        csize, usize = struct.unpack_from("<II", data, at + 20)
        nlen, xlen, clen = struct.unpack_from("<HHH", data, at + 28)
        local = struct.unpack_from("<I", data, at + 42)[0]
        name = data[at + 46:at + 46 + nlen].decode("utf-8", "replace")
        at += 46 + nlen + xlen + clen
        if not name.lower().endswith((".tl", ".lua")):
            continue
        if data[local:local + 4] not in ZIP_LOCAL_MAGICS:
            raise ValueError(f"{name}: unknown local header {data[local:local + 4]!r}")
        lnlen, lxlen = struct.unpack_from("<HH", data, local + 26)
        start = local + 30 + lnlen + lxlen
        raw = data[start:start + csize]
        if method == 0:
            body = raw
        elif method == 8:
            body = zlib.decompress(raw, -15)
        else:
            raise ValueError(f"{name}: compression method {method}")
        if len(body) != usize:
            raise ValueError(f"{name}: {len(body)} bytes, expected {usize}")
        yield name, body.decode("utf-8", errors="replace")


def scan_scripts(folder: Path) -> dict:
    """The game's own script sources, loose and inside its content archives:
    which command factories exist, and which files send commands."""
    result = {"files": 0, "tl": 0, "dtl": 0, "lua": 0, "archives": [], "packed": 0, "factories": {},
              "declared": {}, "senders": [], "game_script_dirs": [], "speed": []}
    for path in sorted(folder.rglob("*")):
        if not path.is_file():
            continue
        name = path.name.lower()
        rel = path.relative_to(folder).as_posix()
        if path.suffix.lower() == ".zip":
            try:
                for inner, text in archive_scripts(path):
                    result["packed"] += 1
                    scan_script_text(f"{rel}!{inner}", inner.lower(), text, result)
            except (OSError, ValueError, struct.error, zlib.error) as error:
                result["archives"].append(f"{rel} ({error})")
            continue
        if path.suffix.lower() in (".pak", ".arc", ".dat") and path.stat().st_size > (1 << 20) \
                and not re.search(r"(?i)texture|audio|sound|model|font", rel):
            result["archives"].append(rel)
        if not (name.endswith(".tl") or name.endswith(".lua")):
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        scan_script_text(rel, name, text, result)
    return result


def scan_script_text(rel: str, name: str, text: str, result: dict) -> None:
    result["files"] += 1
    if name.endswith(".d.tl"):
        result["dtl"] += 1
    elif name.endswith(".tl"):
        result["tl"] += 1
    else:
        result["lua"] += 1
    if "/game_script/" in "/" + rel:
        result["game_script_dirs"].append(rel)
    for number, line in enumerate(text.splitlines(), 1):
        for m in CMD_PATTERN.finditer(line):
            result["factories"].setdefault(m.group(1), []).append(f"{rel}:{number}")
        if name.endswith(".d.tl"):
            for m in DECL_PATTERN.finditer(line):
                result["declared"].setdefault(m.group(1), []).append(f"{rel}:{number}")
        if "sendCommand" in line:
            result["senders"].append(f"{rel}:{number}")
        if re.search(r"GameSpeedControl|setSpeed|IA_GAME_PAUSE", line):
            result["speed"].append(f"{rel}:{number}: {line.strip()[:120]}")


def cmd_scripts(args) -> int:
    game = need_game(args)
    s = scan_scripts(game.folder)
    r = Report("The game's script sources")
    r.add(f"- {s['files']} script files: {s['tl']} .tl, {s['dtl']} .d.tl declarations, {s['lua']} .lua;"
          f" {s['packed']} of them inside the content archives")
    if s["archives"]:
        r.add(f"- CHECK: archives not read, which may hold more scripts: {', '.join(s['archives'][:20])}")
    r.add(f"- game_script folders: {', '.join(s['game_script_dirs']) or 'none (engine-state scripts may be gone; the probes use the GUI state)'}")
    r.add("")
    names = sorted(set(s["factories"]) | set(s["declared"]))
    r.add(f"## Command factories ({len(names)})")
    for name in names:
        uses = s["factories"].get(name, [])
        decl = s["declared"].get(name, [])
        r.add(f"- `api.cmd.{name}`: declared {', '.join(decl[:2]) or 'nowhere seen'}; used {len(uses)}x"
              + (f" ({', '.join(uses[:4])})" if uses else ""))
    r.add("")
    r.add(f"## Files that send commands ({len(s['senders'])})")
    for where in s["senders"][:60]:
        r.add(f"- {where}")
    r.add("")
    r.add("## Speed and pause")
    for where in s["speed"][:30]:
        r.add(f"- {where}")
    r.add("")
    word = "GO" if names else "CHECK"
    if not names:
        r.add("CHECK: no command factory found in plain files: the scripts may be packed; look in the archives above.")
    r.add(f"Verdict: {word}")
    r.save(out_dir(args), "5-scripts.md")
    return 0


def cmd_probes(args) -> int:
    game = need_game(args)
    source_root = REPO / "tools" / "probe" / "tf3"
    which = args.which.split(",") if args.which else ["api", "run", "det"]
    if args.mods:
        targets = [Path(args.mods)]
    else:
        targets = [f for f in mods_folders(game) if f.name == "staging_area"]
        if len(targets) != 1:
            say(f"CHECK: {len(targets)} Steam accounts with Transport Fever 3; name the folder with --mods DIR:")
            for t in targets:
                say(f"  {t}")
            return 1
    target = targets[0]
    for key in which:
        mod_id = PROBES[key]
        src = source_root / mod_id
        dst = target / mod_id
        if args.action == "remove":
            if dst.is_dir() and json.loads((dst / "mod.json").read_text(encoding="utf-8")).get("modId") == mod_id:
                shutil.rmtree(dst)
                say(f"removed {dst}")
            continue
        if dst.exists():
            manifest = dst / "mod.json"
            if not manifest.is_file() or json.loads(manifest.read_text(encoding="utf-8")).get("modId") != mod_id:
                say(f"STOP: {dst} exists and is not our probe; nothing changed")
                return 1
            shutil.rmtree(dst)
        target.mkdir(parents=True, exist_ok=True)
        shutil.copytree(src, dst)
        say(f"installed {mod_id} -> {dst}")
    if args.action == "install":
        say("Now start the game once, open Mod Hub, and activate the probes (a mod not activated does nothing).")
        say("Output: %LOCALAPPDATA%/tpf3mp/probe where the game lets scripts write files, else the game's log: "
            "then run `dayone.py collect --log <the game's log>`.")
        say("Take them out again with `dayone.py probes remove` before playing a room.")
    return 0


BLOCK = re.compile(r"\[tpf3mp-probe ([\w-]+)\] (.*)$")


def collect_lines(text: str) -> tuple[dict[str, list[str]], list[str]]:
    """The probes' blocks (by file name) and determinism lines, out of a
    game log. The determinism lines are the last run's: each game loaded in
    one session starts the probe again with a header, and a run is compared
    from its own header on."""
    blocks: dict[str, list[str]] = {}
    det: list[str] = []
    open_blocks: dict[str, str] = {}
    for raw in text.splitlines():
        m = BLOCK.search(raw)
        if not m:
            continue
        tag, rest = m.groups()
        if tag == "det":
            if rest.startswith("# determinism_probe"):
                det = []
            det.append(rest)
            continue
        if rest.startswith("BEGIN "):
            name = rest[6:].strip()
            open_blocks[tag] = name
            blocks[name] = []
        elif rest.startswith("END "):
            open_blocks.pop(tag, None)
        elif tag in open_blocks:
            blocks[open_blocks[tag]].append(rest)
    return blocks, det


def cmd_collect(args) -> int:
    folder = out_dir(args) / "probe"
    folder.mkdir(parents=True, exist_ok=True)
    got = 0
    probe_dir = Path(os.environ.get("LOCALAPPDATA", Path.home() / ".local/share")) / "tpf3mp" / "probe"
    if probe_dir.is_dir():
        for f in probe_dir.iterdir():
            shutil.copy2(f, folder / f.name)
            say(f"copied {f.name} (written by the probe itself)")
            got += 1
    for log in args.log or []:
        blocks, det = collect_lines(Path(log).read_text(encoding="utf-8", errors="replace"))
        for name, lines in blocks.items():
            (folder / name).write_text("\n".join(lines) + "\n", encoding="utf-8")
            say(f"{name}: {len(lines)} lines, from {log}")
            got += 1
        if det:
            label = args.label or Path(log).stem
            (folder / f"determinism_probe_{label}.log").write_text("\n".join(det) + "\n", encoding="utf-8")
            say(f"determinism_probe_{label}.log: {len(det)} lines, from {log}")
            got += 1
    if not got:
        say("CHECK: nothing from the probes yet. Pass --log <the game's log> (dayone.py find lists candidates).")
        return 1
    say(f"in {folder}")
    return 0


def step_time(path: Path) -> str | None:
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        m = re.search(r"stepTime=(\S+)", line)
        if m:
            return m.group(1)
    return None


def sampled_steps(path: Path) -> set[int]:
    steps = set()
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        m = re.match(r"step=(\d+) ", line)
        if m:
            steps.add(int(m.group(1)))
    return steps


def cmd_compare(args) -> int:
    a, b = Path(args.a), Path(args.b)
    sa, sb = sampled_steps(a), sampled_steps(b)
    if not sa & sb:
        say(f"STOP: the two logs share no sampled step ({min(sa, default='-')}..{max(sa, default='-')} and "
            f"{min(sb, default='-')}..{max(sb, default='-')}): nothing to compare. Load the same save for both runs, "
            "unpaused, and run each past the same step.")
        return 1
    ta, tb = step_time(a), step_time(b)
    if ta != tb:
        say(f"STOP: the two logs label steps differently ({ta} and {tb}), so their steps do not match. "
            "Both must count the game's updateCount, or both run at 1x speed; sample again.")
        return 1
    return subprocess.run([sys.executable, str(REPO / "tools" / "probe" / "compare_runs.py"), str(a), str(b),
                           "-o", str(out_dir(args) / "6-determinism.md")], check=False).returncode


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--game", help="the game's folder (skips the search)")
    ap.add_argument("--steam", help="Steam's folder")
    ap.add_argument("--out", help="where reports go (default investigation/dayone-<date>)")
    sub = ap.add_subparsers(dest="command", required=True)
    p = sub.add_parser("find")
    p.add_argument("--json", action="store_true")
    p.add_argument("--windows", action=argparse.BooleanOptionalAction, default=None,
                   help="judge names as on Windows (default: this system)")
    p = sub.add_parser("archive")
    p.add_argument("--to", help="where the copies go (default ~/TPF3-MP-builds/<build>)")
    p = sub.add_parser("gonogo")
    p.add_argument("--exe", help="one executable to judge (default: the game's)")
    p = sub.add_parser("check-launch")
    p.add_argument("--name", action="append", help="the game's process name (repeatable)")
    p = sub.add_parser("decode")
    p.add_argument("--exe")
    p.add_argument("--tpfre", help="the tpfre binary (default tools/tpfre/target/release)")
    p.add_argument("--names-from", help="an older build's executable or .tpfdb (TPF2's) to carry function names from")
    sub.add_parser("scripts")
    p = sub.add_parser("probes")
    p.add_argument("action", choices=["install", "remove"])
    p.add_argument("--which", help="api,run,det (default all)")
    p.add_argument("--mods", help="the mods folder (default the account's staging_area)")
    p = sub.add_parser("collect")
    p.add_argument("--log", action="append", help="a game log to take probe lines from (repeatable)")
    p.add_argument("--label", help="the determinism log's name (default the log's)")
    p = sub.add_parser("compare")
    p.add_argument("a")
    p.add_argument("b")
    args = ap.parse_args(argv)
    handler = {"find": cmd_find, "archive": cmd_archive, "gonogo": cmd_gonogo, "check-launch": cmd_check_launch,
               "decode": cmd_decode, "scripts": cmd_scripts, "probes": cmd_probes, "collect": cmd_collect,
               "compare": cmd_compare}[args.command]
    return handler(args)


if __name__ == "__main__":
    sys.exit(main())
