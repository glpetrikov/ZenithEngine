#!/usr/bin/env python3
import subprocess
import sys

sys.stdout.reconfigure(encoding="utf-8")
sys.stderr.reconfigure(encoding="utf-8")

red = "\033[31m"
green = "\033[32m"
reset = "\033[0m"

def run(label, cmd):
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"{red}X {label}{reset}")
        if result.stdout:
            print(result.stdout)
        if result.stderr:
            print(result.stderr)
        sys.exit(1)
    else:
        print(f"{green}V {label}{reset}")

run("fmt",     ["cargo", "+nightly", "fmt", "--all", "--", "--check"])
run("check",   ["cargo", "check", "--workspace", "--all-targets"])
run("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])
run("test",    ["cargo", "test", "--workspace"])
run("deny",    ["cargo", "deny", "check"])
