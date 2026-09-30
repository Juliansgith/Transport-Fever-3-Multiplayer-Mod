#!/usr/bin/env python3
"""Diff the lane dumps in two or more games' hook.logs (docs/HOOKS.md, "Lane dumps").

After the room says a game diverged, every game in the room writes the
diverged lanes to its hook.log at the next two checkpoints, one line per
entry:

    [1759240000] lane 3 step 300 vehicle-12 state=2 stop=1 line=line-0 edge=4 pos=17.25 speed=8.5 entity=4711 row=2:1:4@17.25 v8.50

This pairs the entries of each lane and step across the logs by their key
(`vehicle-12`, `line-3`, `town-0`, or `row:<row>` for an entry with no id)
and prints, per lane and step, the entries that differ: which games agree
on what, field by field. The `summary` entry is the lane's text the hook
hashed: when it agrees, the lane agreed at that step.

    python tools/lane_diff.py james=hook.log bob=bob/hook.log cat=cat/hook.log
    python tools/lane_diff.py --lane 3 --step 300 --max 20 a.log b.log
    python tools/lane_diff.py --ignore entity a.log b.log

A log is NAME=PATH or a PATH (named after its folder). Exits 1 when an
entry differs, 0 when every dump common to the logs agrees. Standard
library only.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections import OrderedDict, defaultdict
from pathlib import Path

LANES = {0: "network", 1: "constructions", 2: "lines", 3: "vehicles", 4: "economy", 5: "towns", 6: "people"}

# `[secs] lane <n> step <step> <key> <rest>`; the timestamp is optional.
ENTRY = re.compile(r"^(?:\[\d+\] )?lane (\d+) step (\d+) (\S+)(?: (.*))?$")


def read_dumps(lines):
    """{(lane, step): [(key, rest), ...]} from a log's lines, in order. A lane
    and step dumped twice (the world reloaded and ran the step again) keeps
    the last dump."""
    dumps: dict[tuple[int, int], list[tuple[str, str]]] = OrderedDict()
    closed: set[tuple[int, int]] = set()
    for line in lines:
        match = ENTRY.match(line.rstrip("\r\n"))
        if not match:
            continue
        lane, step, key, rest = int(match[1]), int(match[2]), match[3], match[4] or ""
        at = (lane, step)
        if at in closed:
            closed.discard(at)
            dumps[at] = []
        dumps.setdefault(at, []).append((key, rest))
        if key in ("summary", "err"):
            closed.add(at)
    return dumps


def fields(rest: str) -> "OrderedDict[str, str]":
    """An entry's fields: `k=v` words, then `row=` to the end of the line."""
    out: OrderedDict[str, str] = OrderedDict()
    head, sep, row = rest.partition(" row=")
    if not sep and rest.startswith("row="):
        head, row, sep = "", rest[4:], "row="
    for word in head.split():
        name, eq, value = word.partition("=")
        out[name if eq else word] = value if eq else ""
    if sep:
        out["row"] = row
    return out


def strip(rest: str, ignore: set[str]) -> str:
    if not ignore:
        return rest
    return " ".join(f"{k}={v}" if v or k == "row" else k for k, v in fields(rest).items() if k not in ignore)


def keyed(entries):
    """{key: [rest, ...]}: a key given twice (two edges alike) keeps both."""
    out: dict[str, list[str]] = OrderedDict()
    for key, rest in entries:
        out.setdefault(key, []).append(rest)
    return out


def compare(dumps_by_game, lane, step, ignore, most, out):
    """Prints how the games' dumps of `lane` at `step` differ; returns the
    number of keys that differ."""
    games = [g for g, dumps in dumps_by_game.items() if (lane, step) in dumps]
    by_game = {g: keyed(dumps_by_game[g][(lane, step)]) for g in games}
    keys: list[str] = []
    seen = set()
    for g in games:
        for key in by_game[g]:
            if key not in seen:
                seen.add(key)
                keys.append(key)
    # The summary last, the way the dump writes it.
    keys.sort(key=lambda k: k in ("summary", "err"))
    differ = []
    for key in keys:
        values = {g: tuple(strip(r, ignore) for r in by_game[g].get(key, [])) for g in games}
        if len(set(values.values())) > 1:
            differ.append((key, values))
    name = LANES.get(lane, "?")
    missing = [g for g in dumps_by_game if g not in games]
    head = f"lane {lane} ({name}) step {step}: {', '.join(games)}"
    if missing:
        head += f" (not dumped by {', '.join(missing)})"
    if not differ:
        out.write(f"{head}: all {len(keys)} entries agree\n")
        return 0
    summary = next((v for k, v in differ if k == "summary"), None)
    verdict = "the lane's text differs" if summary else "the lane's text agrees (the differences are below its rounding)"
    out.write(f"{head}: {len(differ)} of {len(keys)} entries differ; {verdict}\n")
    for key, values in [d for d in differ if d[0] != "summary"][:most]:
        out.write(f"  {key}\n")
        groups: dict[tuple, list[str]] = defaultdict(list)
        for g in games:
            groups[values[g]].append(g)
        parsed = {v: [fields(r) for r in v] for v in groups}
        # The fields that differ between the groups, for the first entry.
        names: list[str] = []
        firsts = [p[0] if p else OrderedDict() for p in parsed.values()]
        for f in firsts:
            for n in f:
                if n not in names:
                    names.append(n)
        changed = [n for n in names if len({f.get(n) for f in firsts}) > 1]
        for value, members in groups.items():
            who = ",".join(members)
            if not value:
                out.write(f"    {who}: (no such entry)\n")
                continue
            if len(value) > 1:
                out.write(f"    {who}: {len(value)} entries with this key\n")
            first = parsed[value][0]
            shown = " ".join(f"{n}={first[n]}" for n in changed if n in first) or value[0]
            out.write(f"    {who}: {shown}\n")
    rest = len([d for d in differ if d[0] != "summary"]) - most
    if rest > 0:
        out.write(f"  ... and {rest} more (--max)\n")
    if summary:
        for g in games:
            out.write(f"  summary {g}: {' '.join(summary[g]) or '(none)'}\n")
    return len(differ)


def name_of(arg: str, taken: set[str]) -> tuple[str, Path]:
    if "=" in arg and not Path(arg).exists():
        name, path = arg.split("=", 1)
        return name, Path(path)
    path = Path(arg)
    name = path.parent.name or path.stem
    while name in taken:
        name += "'"
    return name, path


def main(argv=None, out=sys.stdout) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("logs", nargs="+", help="NAME=PATH or PATH of a hook.log")
    parser.add_argument("--lane", type=int, action="append", help="only this lane (repeatable)")
    parser.add_argument("--step", type=int, action="append", help="only this step (repeatable)")
    parser.add_argument("--max", type=int, default=10, help="differing entries shown per lane and step (10)")
    parser.add_argument("--ignore", action="append", default=[], help="a field not to compare, such as entity")
    args = parser.parse_args(argv)
    if len(args.logs) < 2:
        parser.error("give two or more logs")
    dumps_by_game: dict[str, dict] = OrderedDict()
    for arg in args.logs:
        name, path = name_of(arg, set(dumps_by_game))
        with open(path, encoding="utf-8", errors="replace") as f:
            dumps_by_game[name] = read_dumps(f)
    points = sorted({at for dumps in dumps_by_game.values() for at in dumps}, key=lambda at: (at[1], at[0]))
    if args.lane:
        points = [p for p in points if p[0] in args.lane]
    if args.step:
        points = [p for p in points if p[1] in args.step]
    if not points:
        out.write("no lane dumps in these logs (lines 'lane <n> step <step> ...')\n")
        return 0
    differ = 0
    for lane, step in points:
        common = [g for g, d in dumps_by_game.items() if (lane, step) in d]
        if len(common) < 2:
            out.write(f"lane {lane} ({LANES.get(lane, '?')}) step {step}: dumped by {', '.join(common)} only\n")
            continue
        differ += compare(dumps_by_game, lane, step, set(args.ignore), args.max, out)
    return 1 if differ else 0


if __name__ == "__main__":
    sys.exit(main())
