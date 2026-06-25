#!/usr/bin/env python3
"""Patch-bump the version across every file that must stay in lockstep.
Reads the current version from Cargo.toml, increments patch, writes it back to
Cargo.toml, Cargo.lock (the splinter package), the plugin + marketplace manifests,
and the launcher. Prints the new version."""
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parent.parent


def fail(msg):
    sys.exit(f"bump-version: {msg}")


cargo_path = root / "Cargo.toml"
cargo = cargo_path.read_text()
m = re.search(r'(?m)^version = "(\d+)\.(\d+)\.(\d+)"', cargo)
if not m:
    fail("could not find package version in Cargo.toml")
major, minor, patch = int(m[1]), int(m[2]), int(m[3])
old = f"{major}.{minor}.{patch}"
new = f"{major}.{minor}.{patch + 1}"

# Cargo.toml — the package version line only.
cargo_path.write_text(re.sub(rf'(?m)^version = "{re.escape(old)}"', f'version = "{new}"', cargo, count=1))

# Cargo.lock — only the splinter package block (other deps may share a version).
lock_path = root / "Cargo.lock"
lock = lock_path.read_text()
lock_new = re.sub(
    rf'(name = "splinter"\nversion = ")({re.escape(old)})(")',
    lambda mm: mm.group(1) + new + mm.group(3),
    lock,
    count=1,
)
if lock_new == lock:
    fail("could not find splinter package in Cargo.lock")
lock_path.write_text(lock_new)

# JSON manifests — anchored on the old version string (only plugin versions match).
# The launcher carries no version of its own; it reads plugin.json at runtime.
for rel in (".claude-plugin/plugin.json", ".claude-plugin/marketplace.json"):
    p = root / rel
    p.write_text(p.read_text().replace(f'"version": "{old}"', f'"version": "{new}"'))

print(new)
