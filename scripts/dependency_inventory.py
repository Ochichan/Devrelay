#!/usr/bin/env python3
"""Generate lockfile-based dependency and license inventory artifacts."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent


def run(command: list[str]) -> str:
    result = subprocess.run(
        command,
        cwd=REPO_ROOT,
        check=True,
        stdout=subprocess.PIPE,
        stderr=None,
        text=True,
    )
    return result.stdout


def output_dir(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        path = REPO_ROOT / path
    return path


def cell(value: Any, default: str = "") -> str:
    if value is None:
        return default
    return str(value).replace("\t", " ").replace("\n", " ")


def write_license_inventory(metadata: dict[str, Any], path: Path) -> None:
    workspace_members = set(metadata.get("workspace_members", []))
    packages = sorted(
        metadata["packages"],
        key=lambda package: (
            package["name"],
            package["version"],
            package.get("source") or "",
        ),
    )

    with path.open("w", encoding="utf-8", newline="") as inventory:
        inventory.write("name\tversion\tsource\tlicense\tlicense_file\n")
        for package in packages:
            package_id = package["id"]
            source = package.get("source")
            if source is None:
                source = "workspace" if package_id in workspace_members else "path"
            inventory.write(
                "\t".join(
                    [
                        cell(package["name"]),
                        cell(package["version"]),
                        cell(source),
                        cell(package.get("license"), "UNKNOWN"),
                        cell(package.get("license_file")),
                    ]
                )
                + "\n"
            )


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate cargo metadata, dependency tree, and license inventory artifacts."
    )
    parser.add_argument(
        "--out",
        default="target/dependency-inventory",
        help="output directory for generated artifacts",
    )
    args = parser.parse_args()

    out = output_dir(args.out)
    out.mkdir(parents=True, exist_ok=True)

    metadata_json = run(["cargo", "metadata", "--locked", "--format-version", "1"])
    (out / "cargo-metadata.json").write_text(metadata_json, encoding="utf-8")

    metadata = json.loads(metadata_json)
    write_license_inventory(metadata, out / "license-inventory.tsv")

    cargo_tree = run(
        ["cargo", "tree", "--locked", "--workspace", "--edges", "normal,build,dev"]
    )
    (out / "cargo-tree.txt").write_text(cargo_tree, encoding="utf-8")


if __name__ == "__main__":
    main()
