"""Prediction file management with atomic appends and resumability."""

from __future__ import annotations

import fcntl
import json
from pathlib import Path


def load_completed(path: Path) -> set[str]:
    """Read existing predictions JSONL and return completed instance_ids."""
    completed = set()
    if not path.exists():
        return completed

    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                record = json.loads(line)
                completed.add(record["instance_id"])
            except (json.JSONDecodeError, KeyError):
                continue

    return completed


def append_prediction(
    path: Path,
    instance_id: str,
    model_name: str,
    model_patch: str,
) -> None:
    """Atomically append a prediction record with file locking."""
    record = {
        "instance_id": instance_id,
        "model_name_or_path": model_name,
        "model_patch": model_patch,
    }
    line = json.dumps(record, sort_keys=True) + "\n"

    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "a") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        try:
            f.write(line)
            f.flush()
        finally:
            fcntl.flock(f, fcntl.LOCK_UN)
