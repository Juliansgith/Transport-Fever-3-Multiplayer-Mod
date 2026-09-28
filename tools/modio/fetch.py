# Downloads Transport Fever 3's mods from mod.io, the backend of the game's
# Mod Hub, for study: what the game's script API looks like in use. The
# mods are other people's work: they stay in the ignored folder .modio/
# and are never committed or redistributed.
#
# Usage: python tools/modio/fetch.py [--tag "Script Mod"] [--limit 100]
#                                     [--game transportfever3] [--list]
#
# The mod.io API key is read from MODIO_API_KEY, or from the file named by
# MODIO_API_KEY_FILE, and never written anywhere. Anyone can make one, for
# reading public mods, at https://mod.io/me/access. Mods land in
# .modio/<game>/<mod id>-<name_id>/: the mod's metadata as mod.json, its
# scripts and text unpacked in files/ (everything with --all-files). A mod already downloaded at the same file is
# skipped. Standard library only.
import argparse
import io
import json
import os
import pathlib
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
UA = {"User-Agent": "tpf3mp-mod-study/1.0 (tools/modio/fetch.py)"}
# The game ids mod.io gives games, from their name ids; the legacy host
# still answers these lookups.
LOOKUP = "https://api.mod.io"
# What is unpacked unless --all-files: scripts and text, not models,
# textures or sounds.
TEXT = {".lua", ".tl", ".json", ".txt", ".md", ".xml", ".ini", ".cfg", ".csv", ".yml", ".yaml"}


def api_key():
    key = os.environ.get("MODIO_API_KEY")
    if not key and os.environ.get("MODIO_API_KEY_FILE"):
        key = pathlib.Path(os.environ["MODIO_API_KEY_FILE"]).read_text().strip()
    if not key:
        sys.exit("set MODIO_API_KEY or MODIO_API_KEY_FILE (a key from https://mod.io/me/access)")
    return key


def get(key, base, path, **params):
    params["api_key"] = key
    url = f"{base}/v1/{path}?{urllib.parse.urlencode(params)}"
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=60) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        try:
            message = json.load(e).get("error", {}).get("message")
        except ValueError:
            message = None
        sys.exit(f"{path}: HTTP {e.code} {message or e.reason}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--game", default="transportfever3", help="mod.io name id of the game")
    ap.add_argument("--tag", help='only mods with this tag, such as "Script Mod"')
    ap.add_argument("--limit", type=int, default=0, help="at most this many mods (0: all)")
    ap.add_argument("--sort", default="-popular", help="mod.io sort; -popular is the most popular first")
    ap.add_argument("--out", type=pathlib.Path, default=ROOT / ".modio")
    ap.add_argument("--list", action="store_true", help="list, do not download")
    ap.add_argument("--all-files", action="store_true", help="unpack models, textures and sounds too")
    a = ap.parse_args()
    key = api_key()

    game = get(key, LOOKUP, f"games/@{a.game}")
    gid = game["id"]
    base = f"https://g-{gid}.modapi.io"
    print(f"{game['name']}: mod.io game {gid}")

    out = a.out / a.game
    seen, total = 0, None
    while total is None or seen < total:
        params = {"_limit": 100, "_offset": seen, "_sort": a.sort}
        if a.tag:
            params["tags"] = a.tag
        page = get(key, base, f"games/{gid}/mods", **params)
        total = page["result_total"] if not a.limit else min(a.limit, page["result_total"])
        if not page["data"]:
            break
        for m in page["data"][: total - seen]:
            f = m.get("modfile") or {}
            tags = ", ".join(t["name"] for t in m.get("tags", []))
            print(f"{m['id']:>9}  {m['name'][:50]:50}  {f.get('filesize', 0) // 1024:>7} KiB  {tags}")
            seen += 1
            if a.list or not f:
                continue
            d = out / f"{m['id']}-{m['name_id']}"
            meta = d / "mod.json"
            if meta.exists() and json.loads(meta.read_text()).get("modfile", {}).get("id") == f["id"]:
                continue
            d.mkdir(parents=True, exist_ok=True)
            request = urllib.request.Request(f["download"]["binary_url"], headers=UA)
            with urllib.request.urlopen(request, timeout=300) as r:
                archive = zipfile.ZipFile(io.BytesIO(r.read()))
                members = [
                    name
                    for name in archive.namelist()
                    if a.all_files or pathlib.PurePosixPath(name).suffix.lower() in TEXT
                ]
                archive.extractall(d / "files", members)
            meta.write_text(json.dumps(m, indent=1))
            time.sleep(0.5)
    print(f"{seen} mods")


if __name__ == "__main__":
    main()
