#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ ! "${REQUESTED_VERSION:-}" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo 'REQUESTED_VERSION must be stable SemVer X.Y.Z' >&2
  exit 1
fi

python3 - <<'PY'
import os
from pathlib import Path
import re

version = os.environ["REQUESTED_VERSION"]
manifest = Path("Cargo.toml")
text = manifest.read_text(encoding="utf-8")


def replace_in_section(text, section_name, pattern, expected_count):
    section = re.search(rf"(?ms)^\[{re.escape(section_name)}\]\n(.*?)(?=^\[|\Z)", text)
    if section is None:
        raise SystemExit(f"Missing [{section_name}] in Cargo.toml")
    body, count = re.subn(pattern, lambda match: match[1] + version, section[1], flags=re.M)
    if count != expected_count:
        raise SystemExit(f"[{section_name}] has {count} version placeholders, expected {expected_count}")
    return text[:section.start(1)] + body + text[section.end(1):]


text = replace_in_section(text, "workspace.package", r'^(version\s*=\s*")0\.0\.0-dev(?="$)', 1)
text = replace_in_section(
    text, "workspace.dependencies", r'^(sqlx(?:-[\w-]+)?\s*=\s*\{\s*version\s*=\s*"=)0\.0\.0-dev(?=")', 7,
)
manifest.write_text(text, encoding="utf-8")
PY

cargo update --workspace
python3 scripts/check-prismarisk-workspace.py
