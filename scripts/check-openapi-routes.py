#!/usr/bin/env python3
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[1]
api = (root / "services/hearthdeck-daemon/src/api.rs").read_text()
route_pattern = re.compile(
    r'\.route\(\s*"([^"]+)"\s*,(.*?)(?=\n\s*\.route\(|\n\s*\.with_state\(state\)|\n\})',
    re.DOTALL,
)
method_pattern = re.compile(r"\b(get|post|put|delete|patch|head|options|trace)\s*\(")


def routes_built_by(marker: str) -> set[tuple[str, str]]:
    """The (path, method) pairs one `Router::new()` chain registers.

    Every router is read: the network listener's public routes, the protected ones
    behind a bearer token, and the loopback admin listener's own. A route on the admin port is as much a part of
    this contract as one behind a bearer token — it is how the host itself is
    configured — so leaving it undocumented would be the same gap.
    """
    if marker not in api:
        return set()
    # Keep the closing brace: the last route's pattern needs it to terminate.
    section = api.split(marker, 1)[1].split("\n}\n", 1)[0] + "\n}\n"
    return {
        (path, method)
        for path, body in route_pattern.findall(section)
        for method in method_pattern.findall(body)
    }


registered = (
    routes_built_by("pub fn router(")
    | routes_built_by("fn protected_routes(")
    | routes_built_by("pub fn local_router(")
)

documented = set()
in_paths = False
current_path = None
for line in (root / "contracts/openapi.yaml").read_text().splitlines():
    if line == "paths:":
        in_paths = True
        continue
    if line == "components:":
        break
    if not in_paths:
        continue
    if line.startswith("  /") and line.endswith(":"):
        current_path = line[2:-1]
    elif current_path is not None:
        match = re.fullmatch(r"    (get|post|put|delete|patch|head|options|trace):", line)
        if match:
            documented.add((current_path, match.group(1)))

missing = registered - documented
stale = documented - registered
if missing or stale:
    for path, method in sorted(missing):
        print(f"missing from OpenAPI: {method.upper()} {path}", file=sys.stderr)
    for path, method in sorted(stale):
        print(f"not registered by daemon: {method.upper()} {path}", file=sys.stderr)
    sys.exit(1)

print(f"OpenAPI matches {len(registered)} daemon routes")
