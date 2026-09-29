#!/usr/bin/env python3
"""Tests for dayone.py, on made-up Steam folders, executables and logs: no
game, no network, standard library only.

    python tools/dayone/test_dayone.py
"""

from __future__ import annotations

import io
import json
import os
import random
import struct
import sys
import tempfile
import unittest
import zipfile
from contextlib import redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import dayone  # noqa: E402

MANIFEST = """"AppState"
{
	"appid"		"3493540"
	"name"		"Transport Fever 3"
	"buildid"		"20364158"
	"installdir"		"Transport Fever 3"
	"InstalledDepots"
	{
		"3493541"
		{
			"manifest"		"1234567890123456789"
			"size"		"4200000000"
		}
	}
}
"""


def build_pe(sections: list[tuple[str, bytes, bool]], tls_callbacks: int = 0) -> bytes:
    """A PE32+ with these sections (name, contents, executable), and a TLS
    directory with this many callbacks in the first section after it."""
    align = 0x200
    head = bytearray(0x400)
    head[0:2] = b"MZ"
    struct.pack_into("<I", head, 0x3C, 0x80)
    pe = 0x80
    head[pe:pe + 4] = b"PE\0\0"
    opt_size = 240
    struct.pack_into("<HHIIIHH", head, pe + 4, 0x8664, len(sections), 0x6000_0000, 0, 0, opt_size, 0x22)
    opt = pe + 24
    image_base = 0x1_4000_0000
    struct.pack_into("<H", head, opt, 0x20B)
    struct.pack_into("<Q", head, opt + 24, image_base)
    struct.pack_into("<I", head, opt + 108, 16)  # NumberOfRvaAndSizes
    raw = bytearray()
    table = opt + opt_size
    rva = 0x1000
    placed = []
    for i, (name, data, code) in enumerate(sections):
        size = (len(data) + align - 1) // align * align
        ptr = 0x400 + len(raw)
        raw += data + b"\0" * (size - len(data))
        chars = 0x60000020 if code else 0x40000040
        struct.pack_into("<8sIIIIIIHHI", head, table + i * 40, name.encode(), len(data), rva, size, ptr, 0, 0, 0, 0, chars)
        placed.append((rva, ptr))
        rva += (size + 0xFFF) // 0x1000 * 0x1000
    out = bytearray(head) + raw
    if tls_callbacks:
        # The TLS directory at the start of the last section, its callback
        # array right after it.
        last_rva, last_ptr = placed[-1]
        callbacks_rva = last_rva + 0x40
        struct.pack_into("<QQQQ", out, last_ptr, 0, 0, 0, image_base + callbacks_rva)
        for n in range(tls_callbacks):
            struct.pack_into("<Q", out, last_ptr + 0x40 + n * 8, image_base + 0x1000 + n * 16)
        struct.pack_into("<Q", out, last_ptr + 0x40 + tls_callbacks * 8, 0)
        struct.pack_into("<II", out, opt + 112 + 9 * 8, last_rva, 40)
    return bytes(out)


def code(size: int = 8192) -> bytes:
    """Low-entropy bytes, as compiled code is."""
    return bytes((i * 7) % 64 for i in range(size))


def noise(size: int = 8192, seed: int = 1) -> bytes:
    rng = random.Random(seed)
    return bytes(rng.getrandbits(8) for _ in range(size))


def quiet(fn, *args):
    buffer = io.StringIO()
    with redirect_stdout(buffer):
        result = fn(*args)
    return result, buffer.getvalue()


class SteamFixture:
    """A Steam folder with a second library holding the game."""

    def __init__(self, root: Path, exe_name: str | None = "TransportFever3.exe"):
        self.steam = root / "Steam"
        self.library = root / "Library"
        (self.steam / "steamapps").mkdir(parents=True)
        (self.library / "steamapps").mkdir(parents=True)
        (self.steam / "steamapps" / "libraryfolders.vdf").write_text(
            '"libraryfolders"\n{\n\t"0"\n\t{\n\t\t"path"\t\t"%s"\n\t}\n\t"1"\n\t{\n\t\t"path"\t\t"%s"\n\t}\n}\n'
            % (str(self.steam).replace("\\", "\\\\"), str(self.library).replace("\\", "\\\\")))
        (self.library / "steamapps" / "appmanifest_3493540.acf").write_text(MANIFEST)
        self.game = self.library / "steamapps" / "common" / "Transport Fever 3"
        self.game.mkdir(parents=True)
        if exe_name:
            (self.game / exe_name).write_bytes(build_pe([(".text", code(), True)]))
        (self.game / "steam_api64.dll").write_bytes(b"MZ" + b"\0" * 100)
        local = self.steam / "userdata" / "123456" / "3493540" / "local"
        (local / "staging_area").mkdir(parents=True)
        (local / "stdout.txt").write_text("game log\n")


class FindTest(unittest.TestCase):
    def test_steam_manifests_are_read(self):
        state = dayone.parse_acf(MANIFEST)["AppState"]
        self.assertEqual(state["buildid"], "20364158")
        self.assertEqual(state["InstalledDepots"]["3493541"]["manifest"], "1234567890123456789")

    def test_the_game_is_found_in_a_second_library(self):
        with tempfile.TemporaryDirectory() as tmp:
            fx = SteamFixture(Path(tmp))
            game = dayone.find_game(None, fx.steam)
            self.assertEqual(game.folder, fx.game)
            self.assertEqual(game.build, "20364158")
            self.assertEqual([e.name for e in dayone.executables(fx.game)], ["TransportFever3.exe"])
            self.assertEqual(dayone.mods_folders(game)[0].name, "staging_area")

    def test_the_launchers_guess_is_checked(self):
        with tempfile.TemporaryDirectory() as tmp:
            for exe, expected in [("TransportFever3.exe", 0), ("TF3.exe", 0), (None, 1)]:
                root = Path(tmp) / (exe or "none")
                fx = SteamFixture(root, exe)
                out = root / "out"
                code_, text = quiet(dayone.main, ["--steam", str(fx.steam), "--out", str(out), "find", "--windows"])
                self.assertEqual(code_, expected, text)
                report = (out / "1-find.md").read_text(encoding="utf-8")
                word = {"TransportFever3.exe": "Verdict: GO", "TF3.exe": "Verdict: CHECK", None: "Verdict: STOP"}[exe]
                self.assertIn(word, report)
                self.assertIn("stdout.txt", report, "the TPF2 log place is looked in")


class ExecutableTest(unittest.TestCase):
    def judge(self, sections, tls=0):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "game.exe"
            path.write_bytes(build_pe(sections, tls))
            info = dayone.read_pe(path)
            return info, dayone.judge_pe(info)

    def test_steamstub_alone_is_what_the_plan_expects(self):
        info, verdict = self.judge([(".text", code(), True), (".rdata", code(), False), (".bind", noise(), True)], tls=3)
        self.assertEqual([s.name for s in info.sections], [".text", ".rdata", ".bind"])
        self.assertEqual(info.tls_callbacks, 3)
        self.assertGreater(info.sections[2].entropy, 7.5)
        self.assertEqual(verdict.word, "GO", verdict.reasons)

    def test_a_protector_stops_the_plan(self):
        _, verdict = self.judge([(".text", code(), True), (".vmp0", noise(), True)])
        self.assertEqual(verdict.word, "STOP")
        self.assertTrue(any("VMProtect" in r for r in verdict.reasons))

    def test_packed_code_under_another_name_is_looked_at(self):
        _, verdict = self.judge([(".text", noise(), True)])
        self.assertEqual(verdict.word, "CHECK", verdict.reasons)
        _, verdict = self.judge([(".text", code(), True), (".xyz", noise(1 << 21), False)])
        self.assertEqual(verdict.word, "CHECK", verdict.reasons)

    def test_the_build_is_archived_once(self):
        with tempfile.TemporaryDirectory() as tmp:
            fx = SteamFixture(Path(tmp))
            to = Path(tmp) / "builds"
            args = ["--steam", str(fx.steam), "--out", str(Path(tmp) / "out"), "archive", "--to", str(to)]
            code_, text = quiet(dayone.main, args)
            self.assertEqual(code_, 0, text)
            record = json.loads((to / "build.json").read_text(encoding="utf-8"))
            self.assertEqual(record["build"], "20364158")
            names = {f["name"]: f for f in record["files"]}
            self.assertEqual(set(names), {"TransportFever3.exe", "steam_api64.dll"})
            self.assertEqual(len(names["TransportFever3.exe"]["sha256"]), 64)
            self.assertTrue((to / "TransportFever3.exe").is_file())
            self.assertFalse((to / "steam_api64.dll").exists(), "only programs are copied")
            # Again: nothing changes. A copy with other contents: refused.
            self.assertEqual(quiet(dayone.main, args)[0], 0)
            (to / "TransportFever3.exe").write_bytes(b"MZ other")
            code_, text = quiet(dayone.main, args)
            self.assertEqual(code_, 1)
            self.assertIn("STOP", text)


class LaunchTest(unittest.TestCase):
    PROCS = [
        {"pid": 10, "ppid": 1, "name": "steam.exe", "cmd": ""},
        {"pid": 20, "ppid": 1, "name": "tpf3mp-launcher.exe", "cmd": ""},
    ]

    def test_a_game_the_launcher_started_is_good(self):
        procs = self.PROCS + [{"pid": 30, "ppid": 20, "name": "TransportFever3.exe", "cmd": ""}]
        word, lines = dayone.judge_launch(procs, ["TransportFever3.exe"])
        self.assertEqual(word, "GO", lines)

    def test_a_game_steam_started_lost_the_hook(self):
        procs = self.PROCS + [{"pid": 30, "ppid": 10, "name": "TransportFever3.exe", "cmd": ""}]
        word, lines = dayone.judge_launch(procs, ["TransportFever3.exe"])
        self.assertEqual(word, "STOP")
        self.assertTrue(any("restarted itself through Steam" in line for line in lines))

    def test_other_parents_and_no_game_are_looked_at(self):
        procs = self.PROCS + [{"pid": 30, "ppid": 99, "name": "TransportFever3", "cmd": ""}]
        self.assertEqual(dayone.judge_launch(procs, ["TransportFever3"])[0], "CHECK")
        self.assertEqual(dayone.judge_launch(self.PROCS, ["TransportFever3.exe"])[0], "CHECK")


class ScriptsTest(unittest.TestCase):
    def test_command_factories_and_senders_are_listed(self):
        with tempfile.TemporaryDirectory() as tmp:
            game = Path(tmp)
            (game / "res/scripts").mkdir(parents=True)
            (game / "res/scripts/system.d.tl").write_text(
                "local record cmd\n  makeLineCreateCmd: function(string): Cmd\n  makeTownCreateCmd: function(): Cmd\nend\n")
            (game / "res/scripts/line_tool.tl").write_text(
                "local c = api.cmd.makeLineCreateCmd(name)\napi.cmd.sendCommand(c, function() end)\n")
            (game / "res/scripts/old.lua").write_text("local p = api.cmd.make.buildProposal(x)\n")
            (game / "res/gui").mkdir(parents=True)
            (game / "res/gui/game_bar.tl").write_text("GameSpeedControl = function() setSpeed(2) end\n")
            (game / "res/config/game_script").mkdir(parents=True)
            (game / "res/config/game_script/base.lua").write_text("-- nothing\n")
            s = dayone.scan_scripts(game)
            self.assertEqual((s["tl"], s["dtl"], s["lua"]), (2, 1, 2))
            self.assertEqual(sorted(s["factories"]), ["make.buildProposal", "makeLineCreateCmd"])
            self.assertEqual(sorted(s["declared"]), ["makeLineCreateCmd", "makeTownCreateCmd"])
            self.assertEqual(s["senders"], ["res/scripts/line_tool.tl:2"])
            self.assertEqual(len(s["speed"]), 1)
            self.assertEqual(s["game_script_dirs"], ["res/config/game_script/base.lua"])

    def test_scripts_inside_tf3_content_archives_are_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            game = Path(tmp)
            (game / "base/content").mkdir(parents=True)
            packed = game / "base/content/gui.zip"
            with zipfile.ZipFile(packed, "w") as z:
                z.writestr("gui/line_tool.tl", "api.cmd.sendCommand(api.cmd.makeLineCreateCmd(n))\n")
                z.writestr("gui/model.msh", b"\0" * 64, compress_type=zipfile.ZIP_DEFLATED)
                z.writestr("gui/speed.tl", "GameSpeedControl()\n", compress_type=zipfile.ZIP_DEFLATED)
            # TF3 archives: every local header's magic is UG, not PK.
            packed.write_bytes(packed.read_bytes().replace(b"PK\x03\x04", b"UG\x03\x04"))
            with zipfile.ZipFile(game / "base/content/broken.zip", "w") as z:
                z.writestr("x.tl", "api.cmd.makeTownCreateCmd()\n")
            broken = game / "base/content/broken.zip"
            broken.write_bytes(broken.read_bytes().replace(b"PK\x03\x04", b"XX\x03\x04"))
            s = dayone.scan_scripts(game)
            self.assertEqual((s["tl"], s["packed"]), (2, 2))
            self.assertEqual(sorted(s["factories"]), ["makeLineCreateCmd"])
            self.assertEqual(s["senders"], ["base/content/gui.zip!gui/line_tool.tl:1"])
            self.assertEqual(len(s["speed"]), 1)
            # An archive it cannot read is named, never skipped quietly.
            self.assertEqual(len(s["archives"]), 1)
            self.assertIn("broken.zip", s["archives"][0])


class CarryNamesTest(unittest.TestCase):
    def test_an_executable_is_indexed_then_matched_onto_the_new_index(self):
        with tempfile.TemporaryDirectory() as tmp:
            calls = []

            def run(argv):
                calls.append(argv)
                return "matched 12 functions, 5 of them named in the old build (string 5)" if argv[0] == "match" else "seconds 1"
            report = dayone.Report("t")
            with redirect_stdout(io.StringIO()):
                dayone.carry_names(Path("TransportFever2.exe"), Path("tf3.tpfdb"), Path(tmp), run, report)
            self.assertEqual(calls[0][0], "index")
            self.assertEqual(calls[1][:1] + calls[1][2:], ["match", "tf3.tpfdb", "--limit", "0"])
            self.assertIn("5 of them named", "\n".join(report.lines))
            # A .tpfdb is used as it is, not indexed again.
            calls.clear()
            with redirect_stdout(io.StringIO()):
                dayone.carry_names(Path("tpf2.tpfdb"), Path("tf3.tpfdb"), Path(tmp), run, report)
            self.assertEqual([c[0] for c in calls], ["match"])


class ProbesTest(unittest.TestCase):
    def test_probes_go_into_the_accounts_staging_area_and_out_again(self):
        with tempfile.TemporaryDirectory() as tmp:
            fx = SteamFixture(Path(tmp))
            staging = fx.steam / "userdata/123456/3493540/local/staging_area"
            base = ["--steam", str(fx.steam), "--out", str(Path(tmp) / "out")]
            code_, text = quiet(dayone.main, base + ["probes", "install"])
            self.assertEqual(code_, 0, text)
            for mod_id in dayone.PROBES.values():
                self.assertEqual(json.loads((staging / mod_id / "mod.json").read_text())["modId"], mod_id)
            # Again replaces our own; another mod under the name is refused.
            self.assertEqual(quiet(dayone.main, base + ["probes", "install"])[0], 0)
            other = staging / "tpf3mp_detprobe_1"
            (other / "mod.json").write_text('{"modId": "someone_else_1"}')
            self.assertEqual(quiet(dayone.main, base + ["probes", "install", "--which", "det"])[0], 1)
            self.assertTrue((other / "mod.json").read_text().count("someone_else_1"))
            quiet(dayone.main, base + ["probes", "remove"])
            self.assertFalse((staging / "tpf3mp_apidump_1").exists())
            self.assertTrue(other.exists(), "not ours: left alone")


class CollectTest(unittest.TestCase):
    LOG = "\n".join([
        "12:00:01 engine started",
        "12:00:02 [tpf3mp-probe api-gui] BEGIN script_api_dump_gui.txt",
        "12:00:02 [tpf3mp-probe api-gui] # TPF3-MP script_api_dump (TF3) -- state: gui",
        "12:00:02 something else interleaved",
        "12:00:02 [tpf3mp-probe api-gui] api.cmd.makeLineCreateCmd : fn",
        "12:00:02 [tpf3mp-probe api-gui] END script_api_dump_gui.txt lines=2",
        "12:00:09 [tpf3mp-probe det] # determinism_probe (tf3) stride=100 stepTime=200.000000 lanes=v,p,e,c,t,m,n",
        "12:00:10 [tpf3mp-probe det] step=1300 time=260000.000000 v=2 p=1-2 e=3 c=4 t=5 m=6 n=7",
    ])

    def test_blocks_and_samples_come_out_of_the_game_log(self):
        blocks, det = dayone.collect_lines(self.LOG)
        self.assertEqual(blocks, {"script_api_dump_gui.txt": [
            "# TPF3-MP script_api_dump (TF3) -- state: gui", "api.cmd.makeLineCreateCmd : fn"]})
        self.assertEqual(len(det), 2)
        self.assertTrue(det[1].startswith("step=1300 "))

    def test_logs_of_other_step_times_are_not_compared(self):
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a.log", Path(tmp) / "b.log"
            a.write_text("# determinism_probe (tf3) stride=100 stepTime=200.000000\nstep=100 v=1 p=1 e=1 c=1 t=1 m=1 n=1\n")
            b.write_text("# determinism_probe (tf3) stride=100 stepTime=600.000000\nstep=100 v=1 p=1 e=1 c=1 t=1 m=1 n=1\n")
            code_, text = quiet(dayone.main, ["--out", tmp, "compare", str(a), str(b)])
            self.assertEqual(code_, 1)
            self.assertIn("STOP", text)
            b.write_text(a.read_text())
            code_, _ = quiet(dayone.main, ["--out", tmp, "compare", str(a), str(b)])
            self.assertEqual(code_, 0)
            self.assertTrue((Path(tmp) / "6-determinism.md").is_file())


class DecodeTest(unittest.TestCase):
    def test_every_target_is_looked_up(self):
        asked = []

        def run(argv):
            asked.append(argv)
            if argv[2] == "names" and "GameSim" in argv[3]:
                return "0x15aa00 GameSim::Step funcsig exact\n"
            return ""
        report = dayone.Report("test")
        with redirect_stdout(io.StringIO()):
            word = dayone.decode_report(Path("x.exe"), Path("x.tpfdb"), run, report)
        text = "\n".join(report.lines)
        self.assertIn("0x15aa00 GameSim::Step", text)
        self.assertEqual(word, "CHECK", "most targets were not found")
        looked = {argv[3] for argv in asked if argv[2] == "names"}
        self.assertEqual(looked, {pattern for _, pattern in dayone.TARGETS})


if __name__ == "__main__":
    unittest.main()
