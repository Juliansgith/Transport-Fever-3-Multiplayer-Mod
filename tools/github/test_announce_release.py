"""Tests of tools/github/announce_release.py: python tools/github/test_announce_release.py"""

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import announce_release  # noqa: E402

WEBHOOK = "https://discord.com/api/webhooks/123456789012345678/abc_DEF-123"
ROLE = "987654321098765432"


def release(**changes):
    value = {
        "name": "TPF3-MP v1.2.8",
        "tagName": "v1.2.8",
        "url": "https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/releases/tag/v1.2.8",
        "body": "## What's Changed\n* Fix stuck joins by @someone in #1\n@everyone look",
        "isDraft": False,
        "isPrerelease": False,
        "assets": [{"name": "release.json"}, {"name": "release.json.sig"}],
    }
    value.update(changes)
    return value


class AnnounceTest(unittest.TestCase):
    def run_main(self, data, **env):
        env = {"DISCORD_RELEASE_WEBHOOK": WEBHOOK, "DISCORD_ANNOUNCE_ROLE_ID": ROLE, **env}
        path = Path(tempfile.mkdtemp()) / "release.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        sent = []
        code = announce_release.main(
            ["announce_release.py", str(path)], env, lambda url, msg: sent.append((url, msg))
        )
        return code, sent

    def test_signed_release_mentions_the_role_and_no_one_else(self):
        code, sent = self.run_main(release())
        self.assertEqual(code, 0)
        self.assertEqual(len(sent), 1)
        url, message = sent[0]
        self.assertEqual(url, WEBHOOK)
        self.assertTrue(message["content"].startswith(f"<@&{ROLE}> **TPF3-MP v1.2.8** is out"))
        # The notes' @everyone stays text: only the role may ping.
        self.assertEqual(message["allowed_mentions"], {"parse": [], "roles": [ROLE]})
        embed = message["embeds"][0]
        self.assertEqual(embed["url"], release()["url"])
        self.assertIn("Fix stuck joins", embed["description"])

    def test_unsigned_release_is_refused(self):
        code, sent = self.run_main(release(assets=[{"name": "release.json"}]))
        self.assertEqual((code, sent), (1, []))

    def test_draft_is_refused(self):
        code, sent = self.run_main(release(isDraft=True))
        self.assertEqual((code, sent), (1, []))

    def test_missing_fields_are_refused(self):
        data = release()
        del data["isDraft"]
        self.assertEqual(self.run_main(data), (1, []))

    def test_prerelease_is_skipped_without_failing(self):
        self.assertEqual(self.run_main(release(isPrerelease=True)), (0, []))

    def test_bad_settings_post_nothing(self):
        for env in (
            {"DISCORD_RELEASE_WEBHOOK": ""},
            {"DISCORD_RELEASE_WEBHOOK": "https://example.com/api/webhooks/1/x"},
            {"DISCORD_ANNOUNCE_ROLE_ID": ""},
            {"DISCORD_ANNOUNCE_ROLE_ID": "everyone"},
            {"DISCORD_ANNOUNCE_ROLE_ID": "123> @everyone"},
        ):
            with self.subTest(env=env):
                self.assertEqual(self.run_main(release(), **env), (1, []))

    def test_long_notes_are_cut_at_a_line(self):
        body = "\n".join(f"* change number {i}" for i in range(500))
        summary = announce_release.notes(body)
        self.assertLessEqual(len(summary), announce_release.NOTES_LIMIT + 2)
        self.assertTrue(summary.endswith("\n…"))
        self.assertRegex(summary.splitlines()[-2], r"^\* change number \d+$")

    def test_empty_notes_leave_no_description(self):
        message = announce_release.payload(release(body=""), ROLE)
        self.assertNotIn("description", message["embeds"][0])


if __name__ == "__main__":
    unittest.main()
