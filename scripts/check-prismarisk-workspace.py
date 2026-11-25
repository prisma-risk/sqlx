"""Check crate versions and registry rules for the SQLx workspace."""

import json
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
PUBLISHABLE = {
    "sqlx", "sqlx-core", "sqlx-macros", "sqlx-macros-core",
    "sqlx-mysql", "sqlx-postgres", "sqlx-sqlite",
}
REGISTRY = "prismarisk"


def read_toml(path):
    return tomllib.loads(path.read_text(encoding="utf-8"))


def dev_dependency_has_version(dependency, manifest, workspace_dependencies):
    if dependency["target"] is not None:
        manifest = manifest["target"][dependency["target"]]
    name = dependency["rename"] or dependency["name"]
    declaration = manifest["dev-dependencies"][name]
    if isinstance(declaration, dict) and declaration.get("workspace"):
        declaration = workspace_dependencies[name]
    return isinstance(declaration, str) or "version" in declaration


def main():
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
    ))
    member_ids = set(metadata["workspace_members"])
    members = {pkg["name"]: pkg for pkg in metadata["packages"] if pkg["id"] in member_ids}
    missing = PUBLISHABLE - members.keys()
    if missing:
        raise SystemExit(f"Missing workspace crates: {', '.join(sorted(missing))}")

    workspace = read_toml(ROOT / "Cargo.toml")["workspace"]
    version = workspace["package"]["version"]
    index = read_toml(ROOT / ".cargo/config.toml")["registries"][REGISTRY]["index"]
    for name, package in sorted(members.items()):
        expected_publish = [REGISTRY] if name in PUBLISHABLE else []
        if package["publish"] != expected_publish:
            raise SystemExit(f"{name}: publish must be {expected_publish!r}")
        if name not in PUBLISHABLE:
            continue
        if package["version"] != version:
            raise SystemExit(f"{name}: version must be {version}")

        manifest = read_toml(Path(package["manifest_path"]))
        for dependency in package["dependencies"]:
            if dependency["name"] not in members:
                continue
            if dependency["kind"] == "dev" and not dev_dependency_has_version(
                dependency, manifest, workspace["dependencies"],
            ):
                continue
            if dependency["registry"] != index:
                raise SystemExit(f"{name}: {dependency['name']} must name registry {REGISTRY}")

    print(f"Checked {len(PUBLISHABLE)} crates at {version} for {REGISTRY}.")


if __name__ == "__main__":
    main()
