#!/usr/bin/env python3
"""Fetch the model artifacts named by lanthorn's runtime manifest.

Every file is bounded by its declared size and verified before it becomes
visible in the output directory. The Docker build uses this directory as a
read-only source for the final image.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
from urllib.parse import quote
from urllib.request import Request, urlopen


CHUNK = 1024 * 1024


def safe_name(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", value):
        raise ValueError(f"unsafe artifact name: {value!r}")
    return value


def artifact(record):
    name = safe_name(record["name"])
    digest = record["sha256"]
    size = record["size"]
    if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise ValueError(f"invalid SHA-256 for {name}")
    if isinstance(size, bool) or not isinstance(size, int) or size <= 0:
        raise ValueError(f"invalid size for {name}")
    return name, digest, size


def verified(path, digest, size):
    if not path.is_file() or path.stat().st_size != size:
        return False
    sha = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(CHUNK), b""):
            sha.update(block)
    return sha.hexdigest() == digest


def fetch(url, record, directory):
    name, digest, size = artifact(record)
    path = directory / name
    if verified(path, digest, size):
        print(f"verified cached {name} ({size} bytes)", flush=True)
        return
    if not url.startswith("https://"):
        raise ValueError(f"artifact URL must use HTTPS: {name}")
    print(f"fetching {name} ({size} bytes)", flush=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=directory, prefix=f".{name}.", delete=False) as sink:
            temporary = Path(sink.name)
            sha = hashlib.sha256()
            count = 0
            request = Request(url, headers={"User-Agent": "lanthorn-model-bundle/1"})
            with urlopen(request, timeout=120) as source:
                while True:
                    block = source.read(min(CHUNK, size + 1 - count))
                    if not block:
                        break
                    count += len(block)
                    if count > size:
                        raise ValueError(f"{name}: download exceeds pinned size")
                    sha.update(block)
                    sink.write(block)
            if count != size or sha.hexdigest() != digest:
                raise ValueError(f"{name}: size or SHA-256 differs from manifest")
        temporary.chmod(0o644)
        os.replace(temporary, path)
        print(f"verified {name}", flush=True)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    model = manifest["model"]
    revision = manifest["revision"]
    if not isinstance(model, str) or not re.fullmatch(r"[A-Za-z0-9._-]+/[A-Za-z0-9._-]+", model):
        raise ValueError("invalid model repository in manifest")
    if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("model revision must be a pinned commit")
    files = manifest["files"]
    if not isinstance(files, list) or not files:
        raise ValueError("manifest has no model files")
    license_record = manifest["license"]
    license_url = license_record["url"]
    if not isinstance(license_url, str) or not license_url.startswith("https://www.apache.org/licenses/"):
        raise ValueError("model license URL is not the pinned Apache source")
    names = [artifact(record)[0] for record in [*files, license_record]]
    if len(set(names)) != len(names) or "MODEL.txt" in names:
        raise ValueError("duplicate artifact name in manifest")

    args.output.mkdir(parents=True, exist_ok=True)
    base = f"https://huggingface.co/{model}/resolve/{revision}"
    for record in files:
        fetch(f"{base}/{quote(record['name'], safe='')}", record, args.output)
    fetch(license_url, license_record, args.output)

    attribution = [
        f"Model: {model}",
        f"Revision: {revision}",
        f"Model card: https://huggingface.co/{model}",
        "License: Apache-2.0 (LICENSE in this directory)",
        "Artifacts (SHA-256, bytes):",
    ]
    attribution.extend(f"  {name}: {digest} {size}" for name, digest, size in map(artifact, files))
    (args.output / "MODEL.txt").write_text("\n".join(attribution) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
