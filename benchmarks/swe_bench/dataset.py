"""SWE-bench dataset loading."""

from __future__ import annotations

from dataclasses import dataclass

from .config import BenchmarkConfig


@dataclass
class SWEBenchInstance:
    """A single SWE-bench problem instance."""

    instance_id: str
    repo: str
    base_commit: str
    problem_statement: str
    patch: str
    test_patch: str
    fail_to_pass: str
    pass_to_pass: str


def load_instances(config: BenchmarkConfig) -> list[SWEBenchInstance]:
    """Load SWE-bench instances, optionally filtered by instance_ids."""
    from datasets import load_dataset

    ds = load_dataset(config.dataset_name, split=config.split)
    instances = []
    filter_ids = set(config.instance_ids) if config.instance_ids else None

    for row in ds:
        if filter_ids and row["instance_id"] not in filter_ids:
            continue
        instances.append(
            SWEBenchInstance(
                instance_id=row["instance_id"],
                repo=row["repo"],
                base_commit=row["base_commit"],
                problem_statement=row["problem_statement"],
                patch=row["patch"],
                test_patch=row.get("test_patch", ""),
                fail_to_pass=row.get("FAIL_TO_PASS", ""),
                pass_to_pass=row.get("PASS_TO_PASS", ""),
            )
        )

    return instances
