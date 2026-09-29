# Determinism comparison: `determinism_probe_a.log` vs `determinism_probe_b.log`

Produced by `tools/probe/compare_runs.py` from two determinism_probe logs.

- samples in A: 15, in B: 15, common steps: 15
- common step range: 500 .. 1900

## Per-lane first divergence

| lane | description | compared | errors | first differing step | A time | B time |
|---|---|---|---|---|---|---|
| `v` | vehicle count | 15 | 0 | identical | | |
| `p` | vehicle positions (1 m) | 15 | 0 | identical | | |
| `e` | edge geometry (0.1 m) | 0 | 15 | no common samples | | |
| `c` | construction list | 15 | 0 | identical | | |
| `t` | town building counts | 15 | 0 | identical | | |
| `m` | money per player | 15 | 0 | identical | | |
| `n` | people count | 15 | 0 | identical | | |

All lanes with common samples are identical across the compared runs. For the same PC / same binary this is the expected TPF2 result; for a cross-platform pair it is the finding docs/DAY_ONE.md section 4 needs.

> Lanes with no comparable samples (the mod logged `err` -- the API was absent or failed on this build): `e`. Treat these as UNKNOWN, not as agreement.

