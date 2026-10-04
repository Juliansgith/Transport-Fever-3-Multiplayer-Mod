#!/usr/bin/env python3
"""Regression tests for parsing timestamp-prefixed determinism probe comments."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from compare_runs import compare, parse_log


class CompareRunsTests(unittest.TestCase):
    def test_skipped_step_comment_is_not_a_failed_sample(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            a, b = root / "a.log", root / "b.log"
            sample = "step=100 time=20 v=1 p=2 e=3 c=4 t=5 m=6 n=7\n"
            a.write_text(sample + "[123] mod: [tpf3mp-probe det] # skipped step=200 (this frame saw step 201)\n")
            b.write_text(sample + "step=200 time=40 v=1 p=2 e=3 c=4 t=5 m=6 n=7\n")
            result = compare(parse_log(a), parse_log(b))
            self.assertEqual(result["common"], [100])
            self.assertTrue(all(lane["compared"] == 1 and lane["errors"] == 0
                                for lane in result["lanes"].values()))


if __name__ == "__main__":
    unittest.main()
