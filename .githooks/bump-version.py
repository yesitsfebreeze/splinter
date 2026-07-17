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

# JSON manifests — every version field is set to the new version outright, so a
# manifest that drifted behind Cargo.toml self-heals instead of silently staying
# stale (0.1.13-0.1.16 shipped no release because of exactly that).
# The launcher carries no version of its own; it reads plugin.json at runtime.
version_field = re.compile(r'"version":\s*"\d+\.\d+\.\d+"')
for rel in (".claude-plugin/plugin.json", ".claude-plugin/marketplace.json"):
    p = root / rel
    text = version_field.sub(f'"version": "{new}"', p.read_text())
    if f'"version": "{new}"' not in text:
        fail(f"no version field found in {rel}")
    p.write_text(text)

print(new)
