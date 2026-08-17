# #!/usr/bin/env python3
# """Build and package ZenithEngine for Windows and Linux.

# Usage:
#     python scripts/build_dist.py

# Requires:
#     - rustup targets for cross-compilation (auto-installed if missing)
#     - cargo-about for NOTICE generation
# """

# import os
# import platform
# import subprocess
# import sys
# from pathlib import Path
# from zipfile import ZipFile

# REPO_ROOT = Path(__file__).resolve().parent.parent
# API_CS_DIR = REPO_ROOT / "ZenithEngine" / "api" / "cs"
# PROFILE = "dist"
# CARGO = os.environ.get("CARGO", "cargo")
# PYTHON = os.environ.get("PYTHON", sys.executable)
# TARGETS = {
#     "linux": "x86_64-unknown-linux-gnu",
#     "windows": {
#         "Linux": "x86_64-pc-windows-gnu",
#         "Windows": "x86_64-pc-windows-msvc",
#     },
# }

# PACKAGES = ["ZenithEngine", "ZenithEditor"]


# def host_os() -> str:
#     s = platform.system()
#     if s in ("Linux", "Windows"):
#         return s
#     print(f"Unsupported host OS: {s}", file=sys.stderr)
#     sys.exit(1)


# def ensure_target(target: str) -> None:
#     result = subprocess.run(
#         ["rustup", "target", "list", "--installed"],
#         capture_output=True,
#         text=True,
#         check=True,
#     )
#     if target not in result.stdout.splitlines():
#         print(f"Adding rustup target: {target}")
#         subprocess.run(["rustup", "target", "add", target], check=True)


# def cargo_build(target: str, packages: list[str]) -> None:
#     ensure_target(target)
#     for pkg in packages:
#         print(f"  Building {pkg} for {target}...")
#         subprocess.run(
#             [CARGO, "build", "--profile", PROFILE, "--target", target, "-p", pkg],
#             cwd=REPO_ROOT,
#             check=True,
#         )


# def binary_path(pkg: str, target: str) -> Path:
#     base = REPO_ROOT / "target" / target / PROFILE
#     name = pkg
#     if "win" in target:
#         name += ".exe"
#     return base / name


# def update_notice() -> None:
#     subprocess.run(
#         [PYTHON, str(REPO_ROOT / "scripts" / "update_notice.py")],
#         cwd=REPO_ROOT,
#         check=True,
#     )


# def main() -> int:
#     host = host_os()
#     linux_target = TARGETS["linux"]
#     windows_target = TARGETS["windows"][host]
#     print(f"Host: {host}")
#     print(f"  Linux target: {linux_target}")
#     print(f"  Windows target: {windows_target}")

#     cargo_build(linux_target, PACKAGES)
#     cargo_build(windows_target, PACKAGES)

#     update_notice()

#     zip_path = REPO_ROOT / "ZenithEngine.zip"
#     print(f"\nPackaging {zip_path}...")
#     with ZipFile(zip_path, "w") as zf:
#         for pkg in PACKAGES:
#             for target, label in [(linux_target, "Linux"), (windows_target, "Windows")]:
#                 src = binary_path(pkg, target)
#                 if src.is_file():
#                     zf.write(src, src.name)
#                     print(f"  {src.name} ({label})")

#         notice = REPO_ROOT / "NOTICE"
#         if notice.is_file():
#             zf.write(notice, "NOTICE")
#             print("  NOTICE")

#         if API_CS_DIR.is_dir():
#             for cs_file in API_CS_DIR.rglob("*.cs"):
#                 zf.write(cs_file, f"api/cs/{cs_file.relative_to(API_CS_DIR)}")
#             print("  api/cs/ (C# scripting API source files)")

#     print(f"\nDone: {zip_path}")
#     return 0


# if __name__ == "__main__":
#     raise SystemExit(main())
