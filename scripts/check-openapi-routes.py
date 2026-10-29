#!/usr/bin/env python3
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[1]
api = (root / "services/hearthdeck-daemon/src/api.rs").read_text()
router = api.split("pub fn router(", 1)[1].split("/// Routes served only", 1)[0]
route_pattern = re.compile(
    r'\.route\(\s*"([^"]+)"\s*,(.*?)(?=\n\s*\.route\(|\n\s*\.with_state\(state\))',
    re.DOTALL,
)
method_pattern = re.compile(r"\b(get|post|put|delete|patch|head|options|trace)\s*\(")
registered = {
    (path, method)
    for path, body in route_pattern.findall(router)
    for method in method_pattern.findall(body)
}

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
