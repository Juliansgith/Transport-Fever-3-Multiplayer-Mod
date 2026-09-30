"""Tests of tools/lane_diff.py: python tools/test_lane_diff.py"""

import io
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import lane_diff  # noqa: E402

JAMES = """\
[1759240000] the room says: Diverged { step: 250, lanes: [3] }
[1759240010] dumping lanes 3 at the checkpoint after step 300 (step 250 diverged): lines "lane <n> step 300"
[1759240010] lane 3 step 300 vehicle-0 state=1 stop=0 line=line-0 edge=3 pos=10.199999999999999 speed=5 entity=401 row=1:0:3@10.20 v5.00
[1759240010] lane 3 step 300 vehicle-1 state=2 stop=1 line=nil edge=0 pos=0 speed=0 entity=402 row=2:1:0@0.00 v0.00
[1759240010] lane 3 step 300 summary 2:0000000001-0000000002
[1759240010] lane dump at step 300: lanes 3, 2 entries written
"""


class LaneDiffTest(unittest.TestCase):
    def logs(self, **games):
        folder = Path(tempfile.mkdtemp())
        args = []
        for name, text in games.items():
            path = folder / f"{name}.log"
            path.write_text(text, encoding="utf-8")
            args.append(f"{name}={path}")
        return args

    def run_diff(self, *args):
        out = io.StringIO()
        code = lane_diff.main(list(args), out)
        return code, out.getvalue()

    def test_logs_that_agree_say_so(self):
        code, text = self.run_diff(*self.logs(james=JAMES, bob=JAMES))
        self.assertEqual(code, 0)
        self.assertIn("lane 3 (vehicles) step 300: james, bob: all 3 entries agree", text)

    def test_the_differing_vehicle_is_named_with_the_fields_that_differ(self):
        bob = JAMES.replace("pos=0 speed=0", "pos=0.001 speed=0")
        cat = JAMES.replace("state=2 stop=1", "state=3 stop=2").replace(
            "row=2:1:0", "row=3:2:0").replace("summary 2:0000000001", "summary 2:0000000009")
        code, text = self.run_diff(*self.logs(james=JAMES, bob=bob, cat=cat))
        self.assertEqual(code, 1)
        self.assertIn("2 of 3 entries differ; the lane's text differs", text)
        self.assertIn("  vehicle-1\n", text)
        self.assertIn("    james: state=2 stop=1 pos=0 row=2:1:0@0.00 v0.00\n", text)
        self.assertIn("    bob: state=2 stop=1 pos=0.001 row=2:1:0@0.00 v0.00\n", text)
        self.assertIn("    cat: state=3 stop=2 pos=0 row=3:2:0@0.00 v0.00\n", text)
        self.assertNotIn("  vehicle-0\n", text)
        self.assertIn("  summary cat: 2:0000000009-0000000002", text)

    def test_a_difference_below_the_rounding_leaves_the_lanes_text_alike(self):
        bob = JAMES.replace("pos=0 speed=0", "pos=0.001 speed=0")
        code, text = self.run_diff(*self.logs(james=JAMES, bob=bob))
        self.assertEqual(code, 1)
        self.assertIn("1 of 3 entries differ; the lane's text agrees", text)

    def test_an_entry_one_game_lacks_and_ignored_fields(self):
        bob = "\n".join(l for l in JAMES.splitlines() if "vehicle-0" not in l).replace(
            "entity=402", "entity=999")
        code, text = self.run_diff(*self.logs(james=JAMES, bob=bob))
        self.assertIn("    bob: (no such entry)", text)
        self.assertIn("entity=402", text)
        code, text = self.run_diff("--ignore", "entity", *self.logs(james=JAMES, bob=bob))
        self.assertNotIn("entity=", text)
        self.assertIn("1 of 3 entries differ", text)

    def test_a_step_dumped_again_keeps_the_last_dump_and_filters_apply(self):
        again = JAMES + JAMES.replace("pos=0 speed=0", "pos=7 speed=0")
        code, text = self.run_diff(*self.logs(james=again, bob=JAMES.replace("pos=0 speed=0", "pos=7 speed=0")))
        self.assertEqual(code, 0, text)
        code, text = self.run_diff("--lane", "0", *self.logs(james=JAMES, bob=JAMES))
        self.assertIn("no lane dumps", text)

    def test_a_dump_only_one_game_wrote(self):
        code, text = self.run_diff(*self.logs(james=JAMES, bob="nothing here\n"))
        self.assertIn("dumped by james only", text)


if __name__ == "__main__":
    unittest.main()
