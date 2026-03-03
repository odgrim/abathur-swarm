"""Benchmark configuration."""

from __future__ import annotations

import shutil
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class BenchmarkConfig:
    """Configuration for the SWE-bench benchmark runner."""

    abathur_bin: str = "abathur"

    def __post_init__(self) -> None:
        resolved = shutil.which(self.abathur_bin)
        if resolved:
            self.abathur_bin = resolved
    workspace_dir: Path = Path("./swe_bench_workspaces")
    predictions_path: Path = Path("./predictions.jsonl")

    dataset_name: str = "princeton-nlp/SWE-bench_Lite"
    split: str = "test"
    instance_ids: list[str] = field(default_factory=list)

    max_workers: int = 1
    instance_timeout_secs: int = 600
    poll_interval_secs: int = 5

    max_agents: int = 1
    execution_mode: str = "direct"

    cleanup_on_success: bool = True
    cleanup_on_failure: bool = False

    model_name: str = "abathur-swarm"
    verbose: bool = False
