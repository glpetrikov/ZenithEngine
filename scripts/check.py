#!/usr/bin/env python3
import subprocess
import sys

def run(label, cmd):
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"✗ {label}")
        if result.stdout:
            print(result.stdout)
        if result.stderr:
            print(result.stderr)
        sys.exit(1)
    else:
        print(f"✓ {label}")

run("fmt",     ["cargo", "+nightly", "fmt", "--all", "--", "--check"])
run("check",   ["cargo", "check", "--workspace", "--all-targets"])
run("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])
run("test",    ["cargo", "test", "--workspace"])
