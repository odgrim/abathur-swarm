"""Per-instance lifecycle: clone, run abathur, capture patch."""

from __future__ import annotations

import json
import logging
import subprocess
import textwrap
import time
from dataclasses import dataclass
from pathlib import Path
from threading import Lock

from .config import BenchmarkConfig
from .dataset import SWEBenchInstance

log = logging.getLogger(__name__)

# Per-repo locks to serialize git worktree operations (not concurrent-safe).
_repo_locks: dict[str, Lock] = {}
_repo_locks_lock = Lock()


def _get_repo_lock(repo: str) -> Lock:
    with _repo_locks_lock:
        if repo not in _repo_locks:
            _repo_locks[repo] = Lock()
        return _repo_locks[repo]


@dataclass
class InstanceResult:
    """Result of running a single SWE-bench instance."""

    instance_id: str
    status: str  # complete, failed, timeout, error
    patch: str
    elapsed_secs: float
    error: str = ""


def run_instance(instance: SWEBenchInstance, config: BenchmarkConfig) -> InstanceResult:
    """Execute the full lifecycle for one SWE-bench instance."""
    start = time.monotonic()
    worktree_path: Path | None = None

    try:
        # 1. Repo setup: bare clone cache + worktree
        worktree_path = _setup_worktree(instance, config)

        # 2. Write benchmark abathur.toml + init
        _init_abathur(worktree_path, config)

        # 3. Submit task
        task_id = _submit_task(worktree_path, instance, config)

        # 4-5. Start swarm + poll
        status = _run_swarm(worktree_path, task_id, config)

        # 7. Capture patch
        patch = _capture_patch(worktree_path, instance.base_commit)

        elapsed = time.monotonic() - start
        return InstanceResult(
            instance_id=instance.instance_id,
            status=status,
            patch=patch,
            elapsed_secs=elapsed,
        )

    except Exception as exc:
        elapsed = time.monotonic() - start
        log.error("Instance %s error: %s", instance.instance_id, exc)
        return InstanceResult(
            instance_id=instance.instance_id,
            status="error",
            patch="",
            elapsed_secs=elapsed,
            error=str(exc),
        )
    finally:
        # 8. Cleanup worktree
        if worktree_path is not None:
            should_cleanup = (
                config.cleanup_on_success or config.cleanup_on_failure
            )
            if should_cleanup:
                _cleanup_worktree(worktree_path, instance)


def _setup_worktree(
    instance: SWEBenchInstance, config: BenchmarkConfig
) -> Path:
    """Bare-clone the repo (cached) and create a worktree at base_commit."""
    repo_slug = instance.repo.replace("/", "__")
    cache_dir = config.workspace_dir / ".repo_cache"
    cache_dir.mkdir(parents=True, exist_ok=True)
    bare_path = cache_dir / f"{repo_slug}.git"

    repo_lock = _get_repo_lock(instance.repo)

    with repo_lock:
        # Mirror clone if not cached (gets all refs/objects unlike --bare)
        if not bare_path.exists():
            log.info("Cloning %s (mirror) ...", instance.repo)
            subprocess.run(
                [
                    "git", "clone", "--mirror",
                    f"https://github.com/{instance.repo}.git",
                    str(bare_path),
                ],
                check=True,
                capture_output=True,
            )

        # Ensure the specific base_commit is available
        check = subprocess.run(
            ["git", "cat-file", "-t", instance.base_commit],
            cwd=bare_path,
            capture_output=True,
        )
        if check.returncode != 0:
            log.info("Fetching missing commit %s ...", instance.base_commit[:12])
            subprocess.run(
                ["git", "fetch", "origin", instance.base_commit],
                cwd=bare_path,
                capture_output=True,
            )
            # If direct fetch fails (server may not allow it), do a full fetch
            check2 = subprocess.run(
                ["git", "cat-file", "-t", instance.base_commit],
                cwd=bare_path,
                capture_output=True,
            )
            if check2.returncode != 0:
                subprocess.run(
                    ["git", "fetch", "origin", "+refs/*:refs/*"],
                    cwd=bare_path,
                    check=True,
                    capture_output=True,
                )

        # Create worktree
        safe_id = instance.instance_id.replace("/", "_")
        worktree_path = (config.workspace_dir / "instances" / safe_id).resolve()
        if worktree_path.exists():
            # Remove stale worktree
            subprocess.run(
                ["git", "worktree", "remove", "--force", str(worktree_path)],
                cwd=bare_path,
                capture_output=True,
            )

        worktree_path.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [
                "git", "worktree", "add",
                str(worktree_path),
                instance.base_commit,
            ],
            cwd=bare_path,
            check=True,
            capture_output=True,
        )

    return worktree_path


def _init_abathur(worktree_path: Path, config: BenchmarkConfig) -> None:
    """Write a benchmark-tuned abathur.toml and run init."""
    toml_content = textwrap.dedent(f"""\
        [limits]
        max_depth = 3
        max_subtasks = 5
        max_descendants = 20
        max_concurrent_tasks = 1
        max_retries = 0
        task_timeout_secs = {config.instance_timeout_secs}

        [spawn_limits]
        max_subtask_depth = 3
        max_subtasks_per_task = 5
        max_total_descendants = 20
        allow_limit_extensions = false

        [worktrees]
        enabled = false

        [a2a]
        enabled = false

        [logging]
        level = "info"
        format = "json"
    """)
    (worktree_path / "abathur.toml").write_text(toml_content)

    subprocess.run(
        [config.abathur_bin, "init", "--force", "--json"],
        cwd=worktree_path,
        check=True,
        capture_output=True,
    )


def _submit_task(
    worktree_path: Path,
    instance: SWEBenchInstance,
    config: BenchmarkConfig,
) -> str:
    """Submit the problem statement as a task and return the task ID."""
    result = subprocess.run(
        [
            config.abathur_bin, "task", "submit",
            instance.problem_statement,
            "--json",
        ],
        cwd=worktree_path,
        check=True,
        capture_output=True,
        text=True,
    )
    output = json.loads(result.stdout)
    # The CLI outputs the TaskOutput struct — extract the id field.
    task_id = output.get("id") or output.get("task", {}).get("id")
    if not task_id:
        raise RuntimeError(f"Could not parse task ID from: {result.stdout[:200]}")
    return task_id


def _run_swarm(
    worktree_path: Path,
    task_id: str,
    config: BenchmarkConfig,
) -> str:
    """Start the swarm, poll until the task completes, then shut down."""
    # Start swarm as a background process
    swarm_proc = subprocess.Popen(
        [
            config.abathur_bin, "swarm", "start",
            "--foreground",
            "--max-agents", str(config.max_agents),
            "--default-execution-mode", config.execution_mode,
            "--dangerously-skip-permissions",
        ],
        cwd=worktree_path,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    try:
        deadline = time.monotonic() + config.instance_timeout_secs
        status = "timeout"

        while time.monotonic() < deadline:
            time.sleep(config.poll_interval_secs)

            # Check if swarm process died unexpectedly
            if swarm_proc.poll() is not None:
                log.warning(
                    "Swarm process exited early with code %d",
                    swarm_proc.returncode,
                )
                status = "error"
                break

            try:
                result = subprocess.run(
                    [
                        config.abathur_bin, "task", "show",
                        task_id, "--json",
                    ],
                    cwd=worktree_path,
                    capture_output=True,
                    text=True,
                    timeout=30,
                )
                if result.returncode != 0:
                    continue

                output = json.loads(result.stdout)
                task_status = (
                    output.get("task", {}).get("status")
                    or output.get("status", "")
                )

                if task_status in ("complete", "completed"):
                    status = "complete"
                    break
                elif task_status in ("failed", "canceled"):
                    status = "failed"
                    break

            except (subprocess.TimeoutExpired, json.JSONDecodeError):
                continue

        return status

    finally:
        _stop_swarm(worktree_path, swarm_proc, config)


def _stop_swarm(
    worktree_path: Path,
    swarm_proc: subprocess.Popen,
    config: BenchmarkConfig,
) -> None:
    """Gracefully stop the swarm process."""
    # Try graceful stop command
    try:
        subprocess.run(
            [config.abathur_bin, "swarm", "stop"],
            cwd=worktree_path,
            capture_output=True,
            timeout=10,
        )
    except (subprocess.TimeoutExpired, OSError):
        pass

    # Terminate the process
    if swarm_proc.poll() is None:
        swarm_proc.terminate()
        try:
            swarm_proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            swarm_proc.kill()
            swarm_proc.wait(timeout=5)


def _capture_patch(worktree_path: Path, base_commit: str) -> str:
    """Generate the diff from base_commit to current HEAD."""
    result = subprocess.run(
        ["git", "diff", base_commit, "HEAD"],
        cwd=worktree_path,
        capture_output=True,
        text=True,
    )
    return result.stdout


def _cleanup_worktree(
    worktree_path: Path, instance: SWEBenchInstance
) -> None:
    """Remove the git worktree."""
    try:
        repo_slug = instance.repo.replace("/", "__")
        bare_path = worktree_path.parent.parent / ".repo_cache" / f"{repo_slug}.git"
        subprocess.run(
            ["git", "worktree", "remove", "--force", str(worktree_path)],
            cwd=bare_path,
            capture_output=True,
            timeout=30,
        )
    except Exception as exc:
        log.warning("Failed to clean up worktree %s: %s", worktree_path, exc)
