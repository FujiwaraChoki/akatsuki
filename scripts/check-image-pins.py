#!/usr/bin/env python3
"""Fails when a Dockerfile FROM, a Dockerfile *_IMAGE default or a Compose image is not pinned by digest (D-409).

Locally built images (`akatsuki-*:local` defaults) and stage names are exempt. The readable tag stays next to the
digest (`name:tag@sha256:...`); bump both in one reviewed change.
"""
import glob
import re
import sys

bad = []
files = sorted(glob.glob("apps/*/Dockerfile")) + sorted(glob.glob("infra/**/Dockerfile", recursive=True))
for path in files:
    stages = set()
    args = {}
    for n, line in enumerate(open(path), 1):
        m = re.match(r"\s*ARG\s+([A-Z_]*IMAGE)=(\S+)", line)
        if m:
            args[m.group(1)] = m.group(2)
            if "@sha256:" not in m.group(2):
                bad.append(f"{path}:{n}: ARG {m.group(1)}={m.group(2)}")
        m = re.match(r"\s*FROM\s+(?:--platform=\S+\s+)?(\S+)(?:\s+AS\s+(\S+))?", line, re.I)
        if m:
            ref = m.group(1)
            if m.group(2):
                stages.add(m.group(2))
            if ref in stages:
                continue
            var = re.fullmatch(r"\$\{?([A-Z_]+)\}?", ref)
            if var:
                continue  # checked through its ARG default
            if "@sha256:" not in ref:
                bad.append(f"{path}:{n}: FROM {ref}")
composes = sorted(glob.glob("infra/**/docker-compose*.yml", recursive=True))
for path in composes:
    for n, line in enumerate(open(path), 1):
        m = re.match(r"\s*image:\s*(\S+)", line)
        if not m:
            continue
        ref = m.group(1).strip("\"'")
        default = re.fullmatch(r"\$\{[A-Z_]+:-(.+)\}", ref)
        if default:
            ref = default.group(1)
        if re.fullmatch(r"akatsuki-[a-z-]+:local", ref):
            continue
        if "@sha256:" not in ref:
            bad.append(f"{path}:{n}: image {ref}")
if bad:
    print("Pin these images by digest (name:tag@sha256:...):\n" + "\n".join(bad))
    sys.exit(1)
print(f"all images are pinned by digest ({len(files)} Dockerfiles, {len(composes)} Compose files)")
