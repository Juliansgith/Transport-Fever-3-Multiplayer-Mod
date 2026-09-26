#!/usr/bin/env python3
"""test_make_profile.py -- make_profile.py against a synthetic PE (no game binary).

Builds a tiny PE32+ whose `.text` holds hand-assembled functions chosen to hit
every rule of the release-day procedure (docs/HOOKS.md):

  * Alpha::Init           plain prologue, unique on its own;
  * Beta::Speed/Beta::Rate twins up to a `call rel32`: the call target is
                          wildcarded, so each signature must run past it to the
                          distinguishing `mov eax,[rax+4]` / `[rax+8]`;
  * Gamma::Load/Gamma::Store twins whose prologues load RIP-relative data (the
                          displacements differ but are wildcarded) and hold an
                          absolute `mov rdx, imm64` address; the prologue keeps
                          the RIP-relative bytes exactly, since the detour engine
                          relocates them;
  * Delta::Branchy        a `je` 7 bytes in: refused for a 14-byte steal,
                          accepted for a 5-byte one;
  * Epsilon::Get (x2)     byte-identical functions: never unique, refused.

It then checks the emitted profile (TOML, parsed with tomllib) resolves every
target uniquely to its RVA with its prologue, that the refusals happen, and that
the checked-in fixture the Rust test resolves with hookcore itself
(crates/tpf3mp-hookcore/tests/make_profile_fixture.rs) is what the tool emits
today. `--update` rewrites that fixture.

    python tools/re/test_make_profile.py [--update]

Exit 0 when every check passes, 1 otherwise.
"""

from __future__ import annotations

import json
import re
import struct
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import make_profile as mp  # noqa: E402

REPO = HERE.parent.parent
FIXTURE_DIR = REPO / "crates" / "tpf3mp-hookcore" / "tests" / "data"
FIXTURE_BIN = "make_profile_fixture.pe"
FIXTURE_MAP = "make_profile_fixture.symbols.json"
FIXTURE_TOML = "make_profile_fixture.toml"

IMAGE_BASE = 0x1_4000_0000
TEXT_RVA, RDATA_RVA = 0x1000, 0x2000
SLOT = 0x40
TIMESTAMP = 0x6600_0003


def slot(i: int) -> int:
    return TEXT_RVA + SLOT * i


# name, slot index, source file
FUNCTIONS = [
    ("Alpha::Init", 0, "alpha.cpp"),
    ("Beta::Speed", 1, "beta.cpp"),
    ("Beta::Rate", 2, "beta.cpp"),
    ("Gamma::Load", 3, "gamma.cpp"),
    ("Gamma::Store", 4, "gamma.cpp"),
    ("Delta::Branchy", 5, "delta.cpp"),
    ("Epsilon::Get", 6, "epsilon.cpp"),
    ("Epsilon::Get", 7, "epsilon.cpp"),
]
# The targets the fixture profile holds, and whether each is required.
PROFILE_TARGETS = [("Alpha::Init", True), ("Beta::Speed", True), ("Beta::Rate", True),
                   ("Gamma::Load", True), ("Gamma::Store", False)]


def rip(at_rva: int, insn_len: int, target_rva: int) -> bytes:
    return struct.pack("<i", target_rva - (at_rva + insn_len))


def function_bodies() -> dict[int, bytes]:
    """Code per slot index."""
    bodies: dict[int, bytes] = {}
    # push rbx; sub rsp,0x20; mov rbx,rcx; mov rax,42; add rsp,0x20; pop rbx; ret
    bodies[0] = bytes.fromhex("4053 4883EC20 488BD9 48C7C02A000000 4883C420 5B C3")

    def beta(i: int, disp: int) -> bytes:
        at = slot(i)
        head = bytes.fromhex("48895C2408 4883EC28 488BD9 488B4908")  # 16 bytes
        call = b"\xE8" + rip(at + len(head), 5, slot(0))           # call Alpha::Init
        tail = bytes([0x8B, 0x40, disp]) + bytes.fromhex("488B5C2430 4883C428 C3")
        return head + call + tail
    bodies[1] = beta(1, 0x04)
    bodies[2] = beta(2, 0x08)

    def gamma(i: int, data: int, store: bytes) -> bytes:
        at = slot(i)
        code = bytes.fromhex("4883EC28")                                        # sub rsp,0x28
        code += b"\x48\x8B\x05" + rip(at + len(code), 7, RDATA_RVA + data)       # mov rax,[rip+..]
        code += b"\x48\x8D\x0D" + rip(at + len(code), 7, RDATA_RVA + data + 8)   # lea rcx,[rip+..]
        code += b"\x48\xBA" + struct.pack("<Q", IMAGE_BASE + RDATA_RVA + 0x40)  # mov rdx,abs
        code += store + bytes.fromhex("4883C428 C3")
        return code
    bodies[3] = gamma(3, 0x00, bytes.fromhex("488902"))  # mov [rdx],rax
    bodies[4] = gamma(4, 0x10, bytes.fromhex("48890A"))  # mov [rdx],rcx
    # sub rsp,0x28; test rcx,rcx; je +4; mov eax,1; add rsp,0x28; ret
    bodies[5] = bytes.fromhex("4883EC28 4885C9 7405 B801000000 4883C428 C3")
    twin = bytes.fromhex("4883EC28 488B4110 488B4018 4883C428 C3")
    bodies[6] = twin
    bodies[7] = twin
    return bodies


def build_pe() -> bytes:
    text = bytearray(b"\xCC" * 0x200)
    for i, code in function_bodies().items():
        assert len(code) <= SLOT
        off = slot(i) - TEXT_RVA
        text[off:off + len(code)] = code
    rdata = bytearray(0x200)
    for k in range(8):
        struct.pack_into("<Q", rdata, 8 * k, 0x1111 * (k + 1))

    headers = bytearray(0x200)
    headers[0:2] = b"MZ"
    pe = 0x80
    struct.pack_into("<I", headers, 0x3C, pe)
    headers[pe:pe + 4] = b"PE\0\0"
    # COFF: machine, sections, timestamp, symtab, nsyms, optional size, characteristics
    struct.pack_into("<HHIIIHH", headers, pe + 4, 0x8664, 2, TIMESTAMP, 0, 0, 0xF0, 0x22)
    opt = pe + 24
    struct.pack_into("<HBBIIIII", headers, opt, 0x20B, 14, 0, 0x200, 0x200, 0, TEXT_RVA, TEXT_RVA)
    struct.pack_into("<QII", headers, opt + 24, IMAGE_BASE, 0x1000, 0x200)
    struct.pack_into("<HHHHHH", headers, opt + 40, 6, 0, 0, 0, 6, 0)
    struct.pack_into("<IIIIHH", headers, opt + 52, 0, 0x3000, 0x200, 0, 3, 0x8160)
    struct.pack_into("<QQQQII", headers, opt + 72, 0x100000, 0x1000, 0x100000, 0x1000, 0, 16)
    table = opt + 0xF0
    for n, (name, rva, raw, chars) in enumerate([(b".text", TEXT_RVA, 0x200, 0x60000020),
                                                  (b".rdata", RDATA_RVA, 0x400, 0x40000040)]):
        struct.pack_into("<8sIIIIIIHHI", headers, table + 40 * n, name, 0x200, rva, 0x200, raw,
                         0, 0, 0, 0, chars)
    return bytes(headers + text + rdata)


def symbol_map(binary_name: str) -> dict:
    bodies = function_bodies()
    return {
        "binary": binary_name, "format": "pe", "arch": "x86_64", "image_base": IMAGE_BASE,
        "stats": {},
        "functions": [
            {"rva": slot(i), "va": IMAGE_BASE + slot(i), "size": len(bodies[i]), "name": name,
             "name_kind": "funcsig", "source_file": src, "source_kind": "direct",
             "signatures": []}
            for name, i, src in FUNCTIONS
        ],
    }


def resolve(profile: dict, text: bytes, base: int) -> dict[str, int]:
    """An independent re-implementation of hookcore's resolve: unique match,
    then the prologue re-checked at match + offset."""
    out = {}
    for t in profile["target"]:
        tokens = t["signature"].split()
        body = b"".join(b"." if tok == "??" else re.escape(bytes([int(tok, 16)])) for tok in tokens)
        hits = [m.start() for m in re.finditer(b"(?=" + body + b")", text, re.DOTALL)]
        assert len(hits) == 1, "%s matched %d times" % (t["name"], len(hits))
        at = hits[0] + t.get("offset", 0)
        prologue = bytes.fromhex(t["prologue"].replace(" ", ""))
        assert text[at:at + len(prologue)] == prologue, "%s prologue mismatch" % t["name"]
        out[t["name"]] = base + at
    return out


def _check(cond: bool, msg: str, failures: list[str]) -> None:
    print(("  PASS " if cond else "  FAIL ") + msg)
    if not cond:
        failures.append(msg)


def _refused(fn, needle: str) -> bool:
    try:
        fn()
    except mp.ProfileError as exc:
        if needle not in str(exc):
            print("       (refusal did not mention %r: %s)" % (needle, exc))
            return False
        return True
    return False


def main(argv: list[str]) -> int:
    update = "--update" in argv
    failures: list[str] = []
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        binary = tmp / FIXTURE_BIN
        binary.write_bytes(build_pe())
        smap = tmp / FIXTURE_MAP
        smap.write_text(json.dumps(symbol_map(FIXTURE_BIN), indent=1), encoding="utf-8")
        specs = list(PROFILE_TARGETS)

        print("profile:")
        text = mp.make_profile(binary, smap, specs, name="make_profile synthetic fixture")
        profile = tomllib.loads(text)
        _check(profile["build"]["size"] == binary.stat().st_size, "build size recorded", failures)
        _check(profile["build"]["pe_timestamp"] == TIMESTAMP, "PE timestamp recorded", failures)
        _check(profile["region"] == ".text" and profile["image_base"] == IMAGE_BASE,
               "region and image base recorded", failures)
        targets = {t["name"]: t for t in profile["target"]}
        _check(list(targets) == [n for n, _ in PROFILE_TARGETS], "targets keep their order",
               failures)
        _check(not targets["Gamma::Store"]["required"] and targets["Alpha::Init"]["required"],
               "required flags carried", failures)
        _check(all(len(t["prologue"].split()) >= mp.FAR_STEAL and "?" not in t["prologue"]
                   for t in targets.values()),
               "every prologue is wildcard-free and at least 14 bytes", failures)
        _check(all(t["offset"] == 0 for t in targets.values()), "every offset is 0", failures)

        pe_bytes = binary.read_bytes()
        text_bytes = pe_bytes[0x200:0x400]
        try:
            resolved = resolve(profile, text_bytes, TEXT_RVA)
            expected = {name: slot(i) for name, i, _ in FUNCTIONS if name in targets}
            _check(resolved == expected, "every target resolves uniquely to its RVA", failures)
        except AssertionError as exc:
            _check(False, "every target resolves uniquely: %s" % exc, failures)

        beta = targets["Beta::Speed"]["signature"]
        _check("E8 ?? ?? ?? ?? 8B 40 04" in beta,
               "Beta::Speed: the call rel32 is wildcarded and the signature runs past it",
               failures)
        _check(len(beta.split()) == 24, "Beta::Speed: extended exactly to the distinguishing "
               "instruction (24 bytes)", failures)
        gamma = targets["Gamma::Load"]
        _check(gamma["signature"].startswith("48 83 EC 28 48 8B 05 ?? ?? ?? ?? 48 8D 0D ?? ?? ?? ??"
                                             " 48 BA ?? ?? ?? ?? ?? ?? ?? ?? 48 89 02"),
               "Gamma::Load: RIP-relative displacements and the absolute address are wildcarded",
               failures)
        _check(gamma["prologue"] == mp._hex(pe_bytes[0x200 + slot(3) - TEXT_RVA:][:18]),
               "Gamma::Load: the prologue keeps the RIP-relative bytes exactly (18 bytes)",
               failures)
        _check(targets["Alpha::Init"]["signature"] == targets["Alpha::Init"]["prologue"],
               "Alpha::Init: unique at its prologue, not extended", failures)
        _check(mp.make_profile(binary, smap, specs, name="make_profile synthetic fixture") == text,
               "output is deterministic", failures)

        print("refusals:")
        _check(_refused(lambda: mp.make_profile(binary, smap, [("Delta::Branchy", True)]),
                        "not straight-line"),
               "a branch inside the 14-byte prologue is refused", failures)
        near = tomllib.loads(mp.make_profile(binary, smap, [("Delta::Branchy", True)], steal=5))
        _check(near["target"][0]["prologue"] == "48 83 EC 28 48 85 C9",
               "the same function is accepted with --steal 5 (7-byte prologue)", failures)
        _check(_refused(lambda: mp.make_profile(binary, smap, [("Epsilon::Get", True)]),
                        "names 2 functions"),
               "a name shared by two functions asks for NAME@0xRVA", failures)
        _check(_refused(lambda: mp.make_profile(binary, smap,
                                                [("Epsilon::Get@%#x" % slot(6), True)]),
                        "not unique"),
               "a byte-identical twin is refused as never unique", failures)
        _check(_refused(lambda: mp.make_profile(binary, smap, [("Beta::Speed", True)],
                                                max_length=20),
                        "not unique"),
               "--max-length bounds the extension", failures)
        _check(_refused(lambda: mp.make_profile(binary, smap, [("Nope::Nothing", True)]),
                        "not a named function"),
               "an unknown name is refused", failures)

        print("command line:")
        out = tmp / "cli.toml"
        run = subprocess.run([sys.executable, str(HERE / "make_profile.py"), str(binary), str(smap),
                              "Alpha::Init", "Beta::Speed", "Beta::Rate", "Gamma::Load",
                              "Gamma::Store", "--optional", "Gamma::Store",
                              "--name", "make_profile synthetic fixture", "-o", str(out)],
                             capture_output=True, text=True)
        _check(run.returncode == 0 and out.read_text(encoding="utf-8") == text,
               "the CLI writes the same profile", failures)
        listing = tmp / "targets.txt"
        listing.write_text("# hooks\nAlpha::Init\nGamma::Store optional  # late\n", encoding="utf-8")
        run = subprocess.run([sys.executable, str(HERE / "make_profile.py"), str(binary), str(smap),
                              "--functions", str(listing)], capture_output=True, text=True)
        listed = tomllib.loads(run.stdout) if run.returncode == 0 else {"target": []}
        _check([(t["name"], t["required"]) for t in listed["target"]]
               == [("Alpha::Init", True), ("Gamma::Store", False)],
               "--functions file with an optional entry", failures)
        run = subprocess.run([sys.executable, str(HERE / "make_profile.py"), str(binary), str(smap),
                              "Delta::Branchy"], capture_output=True, text=True)
        _check(run.returncode == 1 and "refused" in run.stderr and not run.stdout,
               "the CLI exits 1 with the reason on a refusal", failures)

        print("fixture (crates/tpf3mp-hookcore/tests/data):")
        if update:
            FIXTURE_DIR.joinpath(FIXTURE_BIN).write_bytes(pe_bytes)
            FIXTURE_DIR.joinpath(FIXTURE_TOML).write_text(text, encoding="utf-8", newline="\n")
            print("  wrote %s and %s" % (FIXTURE_BIN, FIXTURE_TOML))
        _check(FIXTURE_DIR.joinpath(FIXTURE_BIN).read_bytes() == pe_bytes,
               "%s is the synthetic PE built here" % FIXTURE_BIN, failures)
        _check(FIXTURE_DIR.joinpath(FIXTURE_TOML).read_text(encoding="utf-8") == text,
               "%s is what make_profile emits today (run with --update after a "
               "deliberate change)" % FIXTURE_TOML, failures)

    print()
    if failures:
        print("TEST_MAKE_PROFILE FAILED (%d check(s)):" % len(failures))
        for m in failures:
            print("  - " + m)
        return 1
    print("TEST_MAKE_PROFILE PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
