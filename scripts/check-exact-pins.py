#!/usr/bin/env python3
"""Fails when a Cargo manifest declares a third-party dependency without an exact (=) version (D-396)."""
import glob
import sys
import tomllib

bad = []
for path in ["Cargo.toml", *sorted(glob.glob("apps/*/Cargo.toml")), *sorted(glob.glob("crates/*/Cargo.toml"))]:
    with open(path, "rb") as f:
        doc = tomllib.load(f)
    tables = [doc.get("workspace", {}).get("dependencies", {})]
    for key in ("dependencies", "dev-dependencies", "build-dependencies"):
        tables.append(doc.get(key, {}))
    for target in doc.get("target", {}).values():
        for key in ("dependencies", "dev-dependencies", "build-dependencies"):
            tables.append(target.get(key, {}))
    for table in tables:
        for name, spec in table.items():
            version = spec if isinstance(spec, str) else spec.get("version") if isinstance(spec, dict) else None
            if version is not None and not version.startswith("="):
                bad.append(f"{path}: {name} = {version!r}")
if bad:
    print("Pin these with =x.y.z (the version in Cargo.lock):\n" + "\n".join(bad))
    sys.exit(1)
print("all dependency versions are exact pins")
