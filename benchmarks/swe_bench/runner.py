"""SWE-bench benchmark runner — main entry point.

Usage:
    python -m benchmarks.swe_bench.runner [options]
"""

from __future__ import annotations

import argparse
import logging
import subprocess
import sys
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

from .config import BenchmarkConfig
from .dataset import load_instances
from .instance import InstanceResult, run_instance
from .predictions import append_prediction, load_completed

log = logging.getLogger(__name__)


def parse_args(argv: list[str] | None = None) -> tuple[BenchmarkConfig, bool]:
    """Parse CLI arguments into a BenchmarkConfig and evaluate flag."""
    parser = argparse.ArgumentParser(
        description="Run SWE-bench benchmark with abathur-swarm",
    )
    parser.add_argument(
        "--dataset",
        default="princeton-nlp/SWE-bench_Lite",
        help="HuggingFace dataset name (default: princeton-nlp/SWE-bench_Lite)",
    )
    parser.add_argument(
        "--split",
        default="test",
        help="Dataset split (default: test)",
    )
    parser.add_argument(
        "--instance-ids",
        nargs="*",
        default=None,
        help="Run specific instance IDs only",
    )
    parser.add_argument(
        "--max-workers",
        type=int,
        default=1,
        help="Parallel instances (default: 1)",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=1200,
        help="Per-instance timeout in seconds (default: 1200)",
    )
    parser.add_argument(
        "--max-agents",
        type=int,
        default=1,
        help="Max concurrent agents per instance (default: 1)",
    )
    parser.add_argument(
        "--execution-mode",
        default="direct",
        help="Abathur execution mode (default: direct)",
    )
    parser.add_argument(
        "--abathur-bin",
        default="abathur",
        help="Path to abathur binary (default: abathur)",
    )
    parser.add_argument(
        "--workspace-dir",
        type=Path,
        default=Path("./swe_bench_workspaces"),
        help="Working directory (default: ./swe_bench_workspaces)",
    )
    parser.add_argument(
        "--predictions",
        type=Path,
        default=Path("./predictions.jsonl"),
        help="Output file (default: ./predictions.jsonl)",
    )
    parser.add_argument(
        "--model-name",
        default="abathur-swarm",
        help="Model identifier for predictions (default: abathur-swarm)",
    )
    parser.add_argument(
        "--cleanup",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Clean up worktrees on success (default: --cleanup)",
    )
    parser.add_argument(
        "--evaluate",
        action="store_true",
        help="Run SWE-bench evaluation after predictions",
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Enable verbose logging",
    )

    args = parser.parse_args(argv)

    config = BenchmarkConfig(
        abathur_bin=args.abathur_bin,
        workspace_dir=args.workspace_dir,
        predictions_path=args.predictions,
        dataset_name=args.dataset,
        split=args.split,
        instance_ids=args.instance_ids or [],
        max_workers=args.max_workers,
        instance_timeout_secs=args.timeout,
        max_agents=args.max_agents,
        execution_mode=args.execution_mode,
        cleanup_on_success=args.cleanup,
        cleanup_on_failure=False,
        model_name=args.model_name,
        verbose=args.verbose,
    )
    return config, args.evaluate


def _process_instance(
    instance_data: dict, config: BenchmarkConfig
) -> InstanceResult:
    """Wrapper for ProcessPoolExecutor — reconstructs the instance from dict."""
    import logging as _logging

    # Re-init logging in the child process (ProcessPoolExecutor forks)
    _logging.basicConfig(
        level=_logging.DEBUG if config.verbose else _logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        stream=sys.stderr,
        force=True,
    )

    from .dataset import SWEBenchInstance

    instance = SWEBenchInstance(**instance_data)
    return run_instance(instance, config)


def run(config: BenchmarkConfig) -> None:
    """Main benchmark execution loop."""
    # Load dataset
    log.info("Loading dataset %s (split=%s) ...", config.dataset_name, config.split)
    instances = load_instances(config)
    log.info("Loaded %d instances", len(instances))

    # Filter out already-completed instances (resumability)
    completed = load_completed(config.predictions_path)
    remaining = [i for i in instances if i.instance_id not in completed]
    skipped = len(instances) - len(remaining)
    if skipped:
        log.info("Skipping %d already-completed instances", skipped)

    if not remaining:
        print("All instances already completed.", file=sys.stderr)
        return

    total = len(remaining)
    stats = {"complete": 0, "failed": 0, "timeout": 0, "error": 0}

    config.workspace_dir.mkdir(parents=True, exist_ok=True)

    # Serialize instances to dicts for cross-process pickling
    instance_dicts = [
        {
            "instance_id": i.instance_id,
            "repo": i.repo,
            "base_commit": i.base_commit,
            "problem_statement": i.problem_statement,
            "patch": i.patch,
            "test_patch": i.test_patch,
            "fail_to_pass": i.fail_to_pass,
            "pass_to_pass": i.pass_to_pass,
        }
        for i in remaining
    ]

    start_time = time.monotonic()

    with ProcessPoolExecutor(max_workers=config.max_workers) as executor:
        futures = {
            executor.submit(_process_instance, d, config): d["instance_id"]
            for d in instance_dicts
        }

        done_count = 0
        for future in as_completed(futures):
            instance_id = futures[future]
            done_count += 1

            try:
                result = future.result()
            except Exception as exc:
                log.error("Instance %s raised: %s", instance_id, exc)
                result = InstanceResult(
                    instance_id=instance_id,
                    status="error",
                    patch="",
                    elapsed_secs=0.0,
                    error=str(exc),
                )

            stats[result.status] = stats.get(result.status, 0) + 1

            # Write prediction if we got a patch
            if result.patch:
                append_prediction(
                    config.predictions_path,
                    result.instance_id,
                    config.model_name,
                    result.patch,
                )

            status_str = result.status.upper()
            print(
                f"[{done_count}/{total}] {result.instance_id}: "
                f"{status_str} ({result.elapsed_secs:.1f}s)",
                file=sys.stderr,
            )
            if result.error and config.verbose:
                print(f"  Error: {result.error}", file=sys.stderr)

    elapsed_total = time.monotonic() - start_time

    # Summary
    print("\n--- SWE-bench Benchmark Summary ---", file=sys.stderr)
    print(f"Total instances:  {total}", file=sys.stderr)
    print(f"Skipped (resume): {skipped}", file=sys.stderr)
    print(f"Completed:        {stats['complete']}", file=sys.stderr)
    print(f"Failed:           {stats['failed']}", file=sys.stderr)
    print(f"Timeout:          {stats['timeout']}", file=sys.stderr)
    print(f"Error:            {stats['error']}", file=sys.stderr)
    print(f"Elapsed:          {elapsed_total:.1f}s", file=sys.stderr)
    print(f"Predictions:      {config.predictions_path}", file=sys.stderr)


def run_evaluation(config: BenchmarkConfig) -> None:
    """Run the SWE-bench evaluation harness on the predictions file."""
    print("\n--- Running SWE-bench Evaluation ---", file=sys.stderr)
    subprocess.run(
        [
            sys.executable, "-m",
            "swebench.harness.run_evaluation",
            "--predictions_path", str(config.predictions_path),
            "--dataset_name", config.dataset_name,
            "--split", config.split,
        ],
        check=True,
    )


def main(argv: list[str] | None = None) -> None:
    """Entry point."""
    config, evaluate = parse_args(argv)

    logging.basicConfig(
        level=logging.DEBUG if config.verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        stream=sys.stderr,
    )

    run(config)

    if evaluate:
        run_evaluation(config)


if __name__ == "__main__":
    main()
