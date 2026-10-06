"""Posts a published release on Discord, mentioning the announcement role.

Run by .github/workflows/announce.yml with the release as `gh release view
--json name,tagName,url,body,isDraft,isPrerelease,assets` wrote it:

    DISCORD_RELEASE_WEBHOOK=... DISCORD_ANNOUNCE_ROLE_ID=... \\
        python tools/github/announce_release.py release.json

Fails closed: it posts nothing for a draft or a release not signed yet (no
release.json.sig, so launchers would not offer it), and nothing with a
webhook or role ID that does not look like Discord's. A prerelease is left
unannounced. The message may mention that role and no one else, whatever
the release notes say. Tests: python tools/github/test_announce_release.py
"""

import json
import re
import sys
import urllib.error
import urllib.request

# Discord takes 4096 characters in an embed; the notes stay a summary, the
# release page has the rest.
NOTES_LIMIT = 1500
WEBHOOK = re.compile(
    r"^https://(?:ptb\.|canary\.)?discord(?:app)?\.com/api/(?:v\d+/)?webhooks/\d+/[\w-]+$"
)
ROLE = re.compile(r"^\d{1,20}$")
# The launcher's accent colour.
COLOUR = 0x2F80ED


def refusal(release):
    """Why the release is not announced, and whether that is an error."""
    if release.get("isDraft") is not False:
        return "is a draft", True
    if release.get("isPrerelease") is not False:
        return "is a prerelease", False
    names = {asset.get("name") for asset in release.get("assets") or []}
    if "release.json.sig" not in names:
        return "is not signed yet (no release.json.sig)", True
    return None, False


def notes(body):
    body = (body or "").strip()
    if len(body) <= NOTES_LIMIT:
        return body
    # Cut at a line, so no link or list item ends halfway.
    cut = body[:NOTES_LIMIT].rsplit("\n", 1)[0].rstrip()
    return cut + "\n…"


def payload(release, role_id):
    name = (release.get("name") or release["tagName"])[:200]
    url = release["url"]
    embed = {"title": name, "url": url, "color": COLOUR}
    summary = notes(release.get("body"))
    if summary:
        embed["description"] = summary
    return {
        # <url> keeps Discord from adding a preview of its own beside ours.
        "content": f"<@&{role_id}> **{name}** is out: <{url}>",
        "allowed_mentions": {"parse": [], "roles": [role_id]},
        "embeds": [embed],
    }


def post(webhook, message):
    request = urllib.request.Request(
        webhook + "?wait=true",
        data=json.dumps(message).encode("utf-8"),
        headers={"Content-Type": "application/json", "User-Agent": "tpf3mp-announce"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            response.read()
    except urllib.error.HTTPError as error:
        # Never print the webhook: it is the secret.
        detail = error.read().decode("utf-8", "replace")[:500]
        raise SystemExit(f"::error::Discord refused the message: {error.code} {detail}")


def main(argv, env, send=post):
    webhook = env.get("DISCORD_RELEASE_WEBHOOK", "").strip()
    role_id = env.get("DISCORD_ANNOUNCE_ROLE_ID", "").strip()
    if not WEBHOOK.match(webhook):
        print("::error::DISCORD_RELEASE_WEBHOOK is not a Discord webhook URL")
        return 1
    if not ROLE.match(role_id):
        print("::error::DISCORD_ANNOUNCE_ROLE_ID is not a role ID (digits only)")
        return 1
    with open(argv[1], encoding="utf-8") as file:
        release = json.load(file)
    why, error = refusal(release)
    if why:
        print(f"::{'error' if error else 'notice'}::{release.get('tagName')} {why}: not announced")
        return 1 if error else 0
    send(webhook, payload(release, role_id))
    print(f"announced {release['tagName']} on Discord")
    return 0


if __name__ == "__main__":
    import os

    sys.exit(main(sys.argv, os.environ))
