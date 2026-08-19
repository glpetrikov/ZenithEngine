#!/usr/bin/env python3

import subprocess
import sys

TARGETS = [
    {
        "name": "ZenithEngine",
        "manifest_path": "Cargo.toml",
        "output_path": "NOTICE",
    },
    {
        "name": "zenithpack CLI",
        "manifest_path": "cli/zenithpack/Cargo.toml",
        "output_path": "cli/zenithpack/NOTICE",
    },
]


def update_notice(target: dict[str, str]) -> bool:
    command = [
        "cargo",
        "about",
        "generate",
        "about.hbs",
        "-o",
        target["output_path"],
        "--manifest-path",
        target["manifest_path"],
    ]

    print(f"Generating {target['name']} NOTICE...")
    result = subprocess.run(command, check=False)
    if result.returncode == 0:
        print(f"Generated {target['name']} NOTICE: {target['output_path']}")
        return True

    print(f"Failed to generate {target['name']} NOTICE", file=sys.stderr)
    return False


def main() -> int:
    failed = False
    for target in TARGETS:
        if not update_notice(target):
            failed = True

    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
