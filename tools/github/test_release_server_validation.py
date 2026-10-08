"""Keep release packaging wired to the launcher's server-list validator."""

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[2]
workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
lines = workflow.splitlines()

try:
    start = lines.index("  package:")
except ValueError as error:
    raise AssertionError("release workflow has no package job") from error

end = next(
    (
        index
        for index in range(start + 1, len(lines))
        if re.match(r"^  [A-Za-z0-9_-]+:$", lines[index])
    ),
    len(lines),
)
package = lines[start:end]

assert "      TPF3MP_SERVERS: ${{ vars.TPF3MP_SERVERS }}" in package, (
    "the package job must pass the release's additional servers to the runtime validator"
)

try:
    validate = package.index("      - name: Validate the release's server list")
    build = package.index("      - name: Build")
except ValueError as error:
    raise AssertionError("the package job must validate its server list before building") from error

assert validate < build, "server-list validation must stop invalid packages before build"
assert "        run: cargo run --locked -p tpf3mp-agent -- validate-release-servers" in package[
    validate : build
], "the release job must use the launcher's runtime validation command"

print("release package passes TPF3MP_SERVERS to the runtime validator before building")
