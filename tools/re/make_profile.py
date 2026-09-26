#!/usr/bin/env python3
"""make_profile.py -- write a hookcore profile (TOML) for named functions of a build.

This is step 3-4 of the release-day procedure in docs/HOOKS.md, mechanised: given
the game binary, the symbol map `name_functions.py` recovered from it, and the
names of the functions to hook, it emits the build's `[build]` identity and one
`[[target]]` per function, ready for `tpf3mp_hookcore::profile::Profile::from_toml`.

For each function, starting at its first byte (so `offset` is always 0):

  * the opening instructions are disassembled with capstone (x86-64 only: the
    detour engine is x86-64 only);
  * every relative or absolute displacement becomes `??`: rel8/rel32 branch and
    call targets, RIP-relative memory displacements, and immediates or absolute
    memory displacements whose value is an address inside the image;
  * the signature starts as the prologue's instructions and is extended one
    instruction at a time only until it matches exactly once across the section
    the resolver scans (the section holding the function, as on-disk bytes). It
    never runs past the function's end and gives up at `--max-length` bytes;
  * the `prologue` is the exact, wildcard-free bytes of the first instructions
    covering at least `--steal` bytes (14, the detour engine's far `jmp [rip+0]`;
    5 is enough only for a detour known to be within 2 GiB), ending on an
    instruction boundary. As in the engine's `decode_prologue`, a branch, call,
    return, interrupt or undecodable byte inside it is refused; a RIP-relative
    data operand is accepted, because the engine relocates it.

Anything that cannot meet these rules is refused with the reason; nothing is
guessed. The output is deterministic: targets keep the order they were given.

    python tools/re/make_profile.py <binary> <symbols.json> NAME [NAME ...] -o build.toml
    python tools/re/make_profile.py <binary> <symbols.json> --functions targets.txt

A NAME is a function's recovered name from the symbol map, or `NAME@0xRVA` to
pick one of several functions sharing a name (or one the map does not name).
In a `--functions` file, one per line; `#` starts a comment; a trailing word
`optional` makes that target `required = false` (as does `--optional NAME`).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

sys.path.insert(0, str(Path(__file__).resolve().parent))
import tpfbin  # noqa: E402

try:
    import capstone
    from capstone import x86_const
except ImportError as exc:  # pragma: no cover - environment guard
    raise SystemExit(
        "make_profile requires the 'capstone' package (pip install -r tools/requirements.txt): %s"
        % exc
    )

# The detour engine's patch sizes (crates/tpf3mp-hookcore/src/detour/x86_64.rs).
FAR_STEAL = 14   # `FF 25 00000000` + 8-byte address: reaches anywhere
NEAR_STEAL = 5   # `E9` + rel32: only when the detour is within 2 GiB
DEFAULT_MAX_LENGTH = 128

# Instructions the engine refuses to steal: anything that is not straight-line
# (iced-x86 FlowControl other than Next).
_NOT_STRAIGHT = {
    capstone.CS_GRP_JUMP, capstone.CS_GRP_CALL, capstone.CS_GRP_RET,
    capstone.CS_GRP_INT, capstone.CS_GRP_IRET, capstone.CS_GRP_BRANCH_RELATIVE,
}
_NOT_STRAIGHT_MNEMONICS = {
    "ud0", "ud1", "ud2", "hlt", "syscall", "sysenter", "sysexit", "sysret",
    "xbegin", "xabort", "xend",
}


class ProfileError(Exception):
    """A target that cannot be turned into a signature under the rules."""


@dataclass
class Target:
    name: str
    rva: int
    size: Optional[int]           # function size from the symbol map, if known
    required: bool = True
    source_file: str = ""


@dataclass
class Insn:
    rva: int
    raw: bytes
    mask: list[bool]              # True = fixed byte, False = wildcard
    text: str
    straight: bool                # safe to steal (relocatable, no control flow)


@dataclass
class Built:
    target: Target
    section: str
    signature: str
    prologue: str
    sig_len: int
    prologue_len: int


# --------------------------------------------------------------------------- #
# Decoding and wildcarding                                                    #
# --------------------------------------------------------------------------- #

def _wildcard(mask: list[bool], start: int, size: int) -> None:
    for k in range(start, min(start + size, len(mask))):
        mask[k] = False


def _wildcard_value(raw: bytes, mask: list[bool], value: int, sizes=(8, 4)) -> bool:
    """Wildcard the last little-endian occurrence of `value` in the encoding.
    A fallback for operand forms capstone reports no offset for."""
    for size in sizes:
        try:
            pattern = (value & ((1 << (8 * size)) - 1)).to_bytes(size, "little")
        except OverflowError:
            continue
        at = raw.rfind(pattern)
        if at > 0:
            _wildcard(mask, at, size)
            return True
    return False


class Decoder:
    def __init__(self, image: tpfbin.Image) -> None:
        if image.arch != "x86_64":
            raise ProfileError(
                "the detour engine is x86-64 only; %s is %s" % (image.path.name, image.arch))
        self.md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
        self.md.detail = True
        self.image = image
        # Absolute addresses a displacement or immediate could hold: the whole
        # mapped image, at its preferred base.
        span = max((s.vaddr + max(s.vsize, s.file_size) for s in image.sections), default=0)
        self.addr_lo = image.image_base
        self.addr_hi = image.image_base + span

    def _is_address(self, value: int) -> bool:
        # Only an image with a real preferred base (PE, non-PIE ELF, Mach-O)
        # has absolute addresses worth recognising; for a PIE (base 0) every
        # small constant would look like one.
        value &= (1 << 64) - 1
        return self.addr_lo != 0 and self.addr_lo <= value < self.addr_hi

    def insn(self, code: bytes, rva: int) -> Optional[Insn]:
        """Decode one instruction at `rva` (RVA addresses: relative operands
        compute the same either way)."""
        found = next(self.md.disasm(code, rva, 1), None)
        if found is None:
            return None
        raw = bytes(found.bytes)
        mask = [True] * len(raw)
        groups = set(found.groups)
        branch_rel = capstone.CS_GRP_BRANCH_RELATIVE in groups
        for op in found.operands:
            if op.type == x86_const.X86_OP_IMM:
                if branch_rel or self._is_address(op.imm):
                    if found.imm_size:
                        _wildcard(mask, found.imm_offset, found.imm_size)
                    else:
                        _wildcard_value(raw, mask, op.imm)
            elif op.type == x86_const.X86_OP_MEM:
                mem = op.mem
                rip = mem.base == x86_const.X86_REG_RIP
                absolute = (mem.base == 0 and mem.index == 0 and self._is_address(mem.disp))
                if rip or absolute:
                    if found.disp_size:
                        _wildcard(mask, found.disp_offset, found.disp_size)
                    elif not _wildcard_value(raw, mask, mem.disp):
                        raise ProfileError(
                            "cannot locate the displacement of `%s %s` at RVA %#x"
                            % (found.mnemonic, found.op_str, rva))
        straight = not (groups & _NOT_STRAIGHT) and found.mnemonic not in _NOT_STRAIGHT_MNEMONICS
        return Insn(rva=rva, raw=raw, mask=mask,
                    text=("%s %s" % (found.mnemonic, found.op_str)).strip(), straight=straight)


def _hex(raw: bytes, mask: Optional[list[bool]] = None) -> str:
    if mask is None:
        mask = [True] * len(raw)
    return " ".join("%02X" % b if keep else "??" for b, keep in zip(raw, mask))


def _regex(raw: bytes, mask: list[bool]) -> "re.Pattern[bytes]":
    body = b"".join(re.escape(bytes([b])) if keep else b"." for b, keep in zip(raw, mask))
    return re.compile(b"(?=" + body + b")", re.DOTALL)


# --------------------------------------------------------------------------- #
# Signature construction                                                      #
# --------------------------------------------------------------------------- #

def _section_of(image: tpfbin.Image, rva: int) -> tpfbin.Section:
    for s in image.sections:
        if s.executable and s.vaddr <= rva < s.vaddr + s.file_size:
            return s
    raise ProfileError("RVA %#x is not in the on-disk bytes of an executable section" % rva)


def build_target(image: tpfbin.Image, decoder: Decoder, target: Target,
                 steal: int = FAR_STEAL, max_length: int = DEFAULT_MAX_LENGTH) -> Built:
    section = _section_of(image, target.rva)
    blob = image.data[section.file_off:section.file_off + section.file_size]
    index = target.rva - section.vaddr
    # The signature may not leave the function (or, when its size is unknown,
    # the section), and never exceeds max_length.
    limit = len(blob) - index
    if target.size:
        limit = min(limit, target.size)
    limit = min(limit, max_length)
    where = "%s at RVA %#x" % (target.name, target.rva)

    insns: list[Insn] = []
    covered = 0

    def decode_next() -> Insn:
        nonlocal covered
        if covered >= limit:
            raise ProfileError("%s: ran out of bytes after %d (function end or --max-length)"
                               % (where, covered))
        found = decoder.insn(blob[index + covered:index + limit], target.rva + covered)
        if found is None:
            raise ProfileError("%s: no instruction decodes at +%d within the %d-byte limit "
                               "(function end or --max-length)" % (where, covered, limit))
        insns.append(found)
        covered += len(found.raw)
        return found

    # Prologue: straight-line instructions covering at least `steal` bytes.
    while covered < steal:
        try:
            found = decode_next()
        except ProfileError as exc:
            raise ProfileError("%s; the prologue needs %d bytes" % (exc, steal)) from None
        if not found.straight:
            hint = ""
            if steal > NEAR_STEAL and covered - len(found.raw) >= NEAR_STEAL:
                hint = (" (the first %d bytes would do for a near detour: --steal %d)"
                        % (covered - len(found.raw), NEAR_STEAL))
            raise ProfileError(
                "%s: the prologue must cover %d bytes but `%s` at +%d is not straight-line "
                "code, which the detour engine refuses to steal%s"
                % (where, steal, found.text, covered - len(found.raw), hint))
    prologue_len = covered
    prologue = blob[index:index + prologue_len]

    # Signature: the prologue's instructions, extended one instruction at a time
    # until exactly one place in the section matches.
    raw = b"".join(i.raw for i in insns)
    mask = [m for i in insns for m in i.mask]
    candidates = [m.start() for m in _regex(raw, mask).finditer(blob)]
    while True:
        if index not in candidates:  # pragma: no cover - a pattern always matches itself
            raise ProfileError("%s: internal error, the signature does not match itself" % where)
        if len(candidates) == 1:
            break
        try:
            found = decode_next()
        except ProfileError as exc:
            others = ", ".join("%#x" % (section.vaddr + c) for c in candidates if c != index)
            raise ProfileError(
                "%s: the signature is still not unique (%d matches; also at RVA %s): %s. "
                "Is the function an identical twin of another? Hook a caller or a "
                "different function instead." % (where, len(candidates), others, exc)) from None
        start = len(raw)
        raw += found.raw
        mask += found.mask
        candidates = [
            c for c in candidates
            if all(not keep or blob[c + start + k:c + start + k + 1] == bytes([b])
                   for k, (b, keep) in enumerate(zip(found.raw, found.mask)))
        ]
    return Built(target=target, section=section.name, signature=_hex(raw, mask),
                 prologue=_hex(prologue), sig_len=len(raw), prologue_len=prologue_len)


# --------------------------------------------------------------------------- #
# Inputs                                                                      #
# --------------------------------------------------------------------------- #

_SPEC = re.compile(r"^(?P<name>.+?)(?:@(?P<rva>0[xX][0-9a-fA-F]+|\d+))?$")


def parse_spec(text: str) -> tuple[str, Optional[int]]:
    m = _SPEC.match(text.strip())
    if not m or not m.group("name"):
        raise ProfileError("bad function spec %r (want NAME or NAME@0xRVA)" % text)
    rva = m.group("rva")
    return m.group("name").strip(), (int(rva, 0) if rva else None)


def read_functions_file(path: Path) -> list[tuple[str, bool]]:
    """(spec, required) per non-comment line."""
    out = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        required = True
        head, _, last = line.rpartition(" ")
        if head and last.lower() == "optional":
            line, required = head.strip(), False
        out.append((line, required))
    return out


def lookup(symbols: Optional[dict], image: tpfbin.Image, spec: str, required: bool) -> Target:
    name, rva = parse_spec(spec)
    functions = symbols["functions"] if symbols else []
    if rva is not None:
        match = [f for f in functions if f["rva"] == rva]
        size = match[0].get("size") if match else image.function_end(rva)
        if not match and size is not None:
            size -= rva
        src = match[0].get("source_file", "") if match else ""
        if match and match[0].get("name") and match[0]["name"] != name:
            print("make_profile: note: RVA %#x is named %r in the symbol map; using %r"
                  % (rva, match[0]["name"], name), file=sys.stderr)
        return Target(name=name, rva=rva, size=size or None, required=required, source_file=src)
    if symbols is None:
        raise ProfileError("%r has no @RVA and no symbol map was given" % name)
    match = [f for f in functions if f.get("name") == name]
    if not match:
        raise ProfileError("%r is not a named function in the symbol map" % name)
    if len(match) > 1:
        raise ProfileError(
            "%r names %d functions in the symbol map (RVAs %s); pick one as NAME@0xRVA"
            % (name, len(match), ", ".join("%#x" % f["rva"] for f in match)))
    f = match[0]
    return Target(name=name, rva=f["rva"], size=f.get("size") or None, required=required,
                  source_file=f.get("source_file", ""))


def identity(image: tpfbin.Image) -> dict:
    """What `BuildIdentity::of_file` computes: SHA-256, size, and the PE
    timestamp for a PE32+ image (absent otherwise)."""
    out = {"sha256": hashlib.sha256(image.data).hexdigest(), "size": len(image.data)}
    if image.fmt == "pe" and image.pe is not None and image.pe.OPTIONAL_HEADER.Magic == 0x20B:
        out["pe_timestamp"] = image.pe.FILE_HEADER.TimeDateStamp
    return out


# --------------------------------------------------------------------------- #
# Output                                                                      #
# --------------------------------------------------------------------------- #

def _toml_str(text: str) -> str:
    # A JSON string with ASCII escapes is a valid TOML basic string.
    return json.dumps(text, ensure_ascii=True)


def render(image: tpfbin.Image, built: list[Built], name: str, steal: int,
           map_name: str = "") -> str:
    ident = identity(image)
    region = built[0].section
    lines = [
        "# Generated by tools/re/make_profile.py from %s%s." % (
            image.path.name, (" and " + map_name) if map_name else ""),
        "#",
        "# Signatures wildcard every relative or absolute displacement and match",
        "# exactly once across the on-disk %s; each prologue is the exact bytes the" % region,
        "# detour engine steals (at least %d, on an instruction boundary). Addresses" % steal,
        "# are RVAs from image base %#x." % image.image_base,
        "",
        "name = %s" % _toml_str(name),
        "image_base = %#x" % image.image_base,
        "region = %s" % _toml_str(region),
        "",
        "[build]",
        'sha256 = "%s"' % ident["sha256"],
        "size = %d" % ident["size"],
    ]
    if "pe_timestamp" in ident:
        lines.append("pe_timestamp = 0x%08X" % ident["pe_timestamp"])
    for b in built:
        t = b.target
        lines.append("")
        note = "RVA %#x" % t.rva
        if t.source_file:
            note += ", %s" % t.source_file
        lines.append("# %s: signature %d bytes, prologue %d bytes." % (note, b.sig_len, b.prologue_len))
        lines += [
            "[[target]]",
            "name = %s" % _toml_str(t.name),
            'signature = "%s"' % b.signature,
            "offset = 0",
            'prologue = "%s"' % b.prologue,
            "required = %s" % ("true" if t.required else "false"),
        ]
    return "\n".join(lines) + "\n"


def make_profile(binary: Path, symbols_path: Optional[Path], specs: list[tuple[str, bool]],
                 name: Optional[str] = None, steal: int = FAR_STEAL,
                 max_length: int = DEFAULT_MAX_LENGTH) -> str:
    """The whole pipeline; raises ProfileError with every refusal found."""
    if steal < NEAR_STEAL:
        raise ProfileError("--steal must be at least %d (the near jump)" % NEAR_STEAL)
    if not specs:
        raise ProfileError("no functions given")
    image = tpfbin.Image.load(binary)
    symbols = None
    if symbols_path is not None:
        symbols = json.loads(symbols_path.read_text(encoding="utf-8"))
        if "functions" not in symbols:
            raise ProfileError("%s is not a name_functions symbol map" % symbols_path)
        if symbols.get("image_base", image.image_base) != image.image_base:
            raise ProfileError("%s was made for image base %#x, but %s has %#x"
                               % (symbols_path, symbols["image_base"], binary, image.image_base))
        if symbols.get("binary") and symbols["binary"] != image.path.name:
            print("make_profile: note: the symbol map is for %r, not %r"
                  % (symbols["binary"], image.path.name), file=sys.stderr)
    decoder = Decoder(image)
    errors: list[str] = []
    built: list[Built] = []
    seen: set[str] = set()
    for spec, required in specs:
        try:
            target = lookup(symbols, image, spec, required)
            if target.name in seen:
                raise ProfileError("%r is listed twice" % target.name)
            seen.add(target.name)
            built.append(build_target(image, decoder, target, steal, max_length))
        except ProfileError as exc:
            errors.append(str(exc))
    if not errors:
        regions = sorted({b.section for b in built})
        if len(regions) > 1:
            errors.append("targets lie in several sections (%s); a profile scans one region"
                          % ", ".join(regions))
    if errors:
        raise ProfileError("\n".join(errors))
    if name is None:
        name = "%s %s (%s %s)" % (image.path.stem, identity(image)["sha256"][:12],
                                  image.fmt.upper(), image.arch)
    return render(image, built, name, steal, symbols_path.name if symbols_path else "")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("binary", type=Path, help="the game executable (read only)")
    ap.add_argument("symbols", type=Path,
                    help="the name_functions.py symbol map (<stem>.symbols.json)")
    ap.add_argument("functions", nargs="*", help="NAME or NAME@0xRVA, one per target")
    ap.add_argument("--functions", dest="functions_file", type=Path,
                    help="a file of targets, one per line ('# comment', 'NAME optional')")
    ap.add_argument("--optional", action="append", default=[], metavar="NAME",
                    help="emit this target with required = false (repeatable)")
    ap.add_argument("--name", help="the profile's name (default: binary, hash prefix, format)")
    ap.add_argument("--steal", type=int, default=FAR_STEAL,
                    help="minimum prologue bytes (default %d, the far jump; %d only for a "
                         "detour known to be near)" % (FAR_STEAL, NEAR_STEAL))
    ap.add_argument("--max-length", type=int, default=DEFAULT_MAX_LENGTH,
                    help="give up when a signature is not unique by this many bytes "
                         "(default %d)" % DEFAULT_MAX_LENGTH)
    ap.add_argument("-o", "--output", type=Path, help="write the TOML here (default: stdout)")
    args = ap.parse_args(argv)

    specs: list[tuple[str, bool]] = [(s, True) for s in args.functions]
    if args.functions_file:
        specs += read_functions_file(args.functions_file)
    optional = set(args.optional)
    specs = [(s, req and parse_spec(s)[0] not in optional) for s, req in specs]
    try:
        text = make_profile(args.binary, args.symbols, specs, args.name, args.steal,
                            args.max_length)
    except (ProfileError, ValueError, OSError) as exc:
        print("make_profile: refused:\n  " + str(exc).replace("\n", "\n  "), file=sys.stderr)
        return 1
    if args.output:
        args.output.write_text(text, encoding="utf-8", newline="\n")
        print("make_profile: wrote %d target(s) to %s" % (text.count("[[target]]"), args.output),
              file=sys.stderr)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
