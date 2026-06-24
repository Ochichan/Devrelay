#!/usr/bin/env python3
"""Run a small DevRelay agent resource benchmark and write a Markdown report."""

from __future__ import annotations

import argparse
import json
import os
import plistlib
import platform
import statistics
import subprocess
import tempfile
import threading
import time
from dataclasses import dataclass
from datetime import datetime
from functools import cache
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT_DIR = REPO_ROOT / "target" / "resource-benchmarks"


@dataclass
class Sample:
    elapsed_seconds: float
    cpu_percent: float
    rss_kib: int


def run(
    command: list[str],
    *,
    cwd: Path = REPO_ROOT,
    env: dict[str, str] | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=check,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def output_path(value: str | None) -> Path:
    if value:
        path = Path(value)
        return path if path.is_absolute() else REPO_ROOT / path
    stamp = datetime.now().strftime("%Y-%m-%d-%H%M%S")
    platform_key = platform.system().lower() or "unknown"
    return DEFAULT_OUT_DIR / f"{stamp}-{platform_key}.md"


def build_binaries(skip_build: bool) -> None:
    if skip_build:
        return
    run(["cargo", "build", "-p", "devrelay-cli", "-p", "devrelay-agent"])


def ensure_binary(path: str | None, default: Path, label: str) -> Path:
    resolved = Path(path) if path else default
    if not resolved.is_absolute():
        resolved = REPO_ROOT / resolved
    if not resolved.exists():
        raise SystemExit(f"{label} binary not found at {resolved}")
    return resolved


def git(root: Path, args: list[str]) -> str:
    return run(["git", "-C", str(root), *args]).stdout


def write_manifest(repo: Path, project_id: str) -> Path:
    manifest = repo / "devrelay.toml"
    manifest.write_text(
        f"""
schema = 1
project_id = "{project_id}"
name = "Resource Benchmark Project"

[workspace]
untracked = "safe"
portable_paths = "strict"
""".lstrip(),
        encoding="utf-8",
    )
    return manifest


@cache
def rust_sysroot_lib() -> str:
    return str(Path(command_output(["rustc", "--print", "sysroot"])) / "lib")


def binary_env(home: Path) -> dict[str, str]:
    env = os.environ.copy()
    env["DEVRELAY_HOME"] = str(home)
    fallback_parts = [
        str(REPO_ROOT / "target" / "debug" / "deps"),
        rust_sysroot_lib(),
        str(Path.home() / "lib"),
        "/usr/local/lib",
        "/usr/lib",
    ]
    existing = env.get("DYLD_FALLBACK_LIBRARY_PATH")
    if existing:
        fallback_parts.append(existing)
    env["DYLD_FALLBACK_LIBRARY_PATH"] = ":".join(fallback_parts)
    return env


def create_benchmark_repo(root: Path, tracked_files: int) -> tuple[Path, Path]:
    repo = root / "benchmark-repo"
    repo.mkdir(parents=True)
    git(repo, ["init", "-b", "main"])
    git(repo, ["config", "user.name", "DevRelay Benchmark"])
    git(repo, ["config", "user.email", "devrelay-benchmark@example.local"])
    (repo / "src").mkdir()
    (repo / "README.md").write_text("resource benchmark\n", encoding="utf-8")
    for index in range(tracked_files):
        (repo / "src" / f"file-{index:04}.txt").write_text(
            f"tracked file {index}\n", encoding="utf-8"
        )
    manifest = write_manifest(repo, "90000001")
    git(repo, ["add", "."])
    git(repo, ["commit", "-m", "benchmark base"])
    return repo, manifest


def make_checkpoint_dirty(repo: Path, iteration: int) -> None:
    readme = repo / "README.md"
    with readme.open("a", encoding="utf-8") as handle:
        handle.write(f"checkpoint iteration {iteration}\n")
    (repo / "notes.md").write_text(
        f"safe untracked note {iteration}\n", encoding="utf-8"
    )


def process_sample(pid: int, start: float) -> Sample | None:
    result = run(
        ["ps", "-p", str(pid), "-o", "%cpu=", "-o", "rss="],
        check=False,
    )
    if result.returncode != 0 or not result.stdout.strip():
        return None
    fields = result.stdout.strip().split()
    if len(fields) < 2:
        return None
    return Sample(
        elapsed_seconds=time.monotonic() - start,
        cpu_percent=float(fields[0]),
        rss_kib=int(fields[1]),
    )


def sample_for(pid: int, duration_seconds: float, interval_seconds: float) -> list[Sample]:
    samples: list[Sample] = []
    start = time.monotonic()
    while time.monotonic() - start < duration_seconds:
        sample = process_sample(pid, start)
        if sample is not None:
            samples.append(sample)
        time.sleep(interval_seconds)
    return samples


def sample_until(
    pid: int,
    done: threading.Event,
    interval_seconds: float,
) -> list[Sample]:
    samples: list[Sample] = []
    start = time.monotonic()
    while not done.is_set():
        sample = process_sample(pid, start)
        if sample is not None:
            samples.append(sample)
        time.sleep(interval_seconds)
    sample = process_sample(pid, start)
    if sample is not None:
        samples.append(sample)
    return samples


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    values = sorted(values)
    index = min(len(values) - 1, max(0, int(round((len(values) - 1) * pct))))
    return values[index]


def summarize(samples: list[Sample]) -> dict[str, float]:
    cpu = [sample.cpu_percent for sample in samples]
    rss = [float(sample.rss_kib) for sample in samples]
    return {
        "sample_count": float(len(samples)),
        "cpu_p50": statistics.median(cpu) if cpu else 0.0,
        "cpu_p95": percentile(cpu, 0.95),
        "cpu_peak": max(cpu) if cpu else 0.0,
        "rss_mib": (statistics.median(rss) / 1024.0) if rss else 0.0,
        "rss_peak_mib": (max(rss) / 1024.0) if rss else 0.0,
    }


def wait_for_socket(socket: Path, process: subprocess.Popen[str]) -> None:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if process.poll() is not None:
            stderr = process.stderr.read() if process.stderr else ""
            raise RuntimeError(
                f"agent exited early with code {process.returncode}: {stderr.strip()}"
            )
        if socket.exists():
            return
        time.sleep(0.05)
    raise TimeoutError(f"agent socket was not created: {socket}")


def start_agent(agent_bin: Path, home: Path, socket: Path) -> subprocess.Popen[str]:
    process = subprocess.Popen(
        [
            str(agent_bin),
            "--foreground",
            "--config",
            str(home / "config.toml"),
            "--socket-path",
            str(socket),
            "--log-level",
            "error",
        ],
        cwd=REPO_ROOT,
        env=binary_env(home),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    wait_for_socket(socket, process)
    return process


def stop_agent(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def cli_call(
    cli_bin: Path,
    home: Path,
    socket: Path,
    args: list[str],
) -> subprocess.CompletedProcess[str]:
    return run(
        [str(cli_bin), "--agent-socket", str(socket), *args],
        env=binary_env(home),
    )


def checkpoint_once(
    cli_bin: Path,
    home: Path,
    socket: Path,
    repo: Path,
    manifest: Path,
    iteration: int,
) -> dict[str, Any]:
    make_checkpoint_dirty(repo, iteration)
    start = time.monotonic()
    result = cli_call(
        cli_bin,
        home,
        socket,
        [
            "checkpoint",
            "--repo",
            str(repo),
            "--manifest",
            str(manifest),
            "--label",
            f"resource-benchmark-{iteration}",
            "--json",
        ],
    )
    elapsed = time.monotonic() - start
    payload = json.loads(result.stdout)
    return {
        "elapsed_seconds": elapsed,
        "snapshot_id": payload["checkpoint"]["snapshot_id"],
        "included_untracked": len(
            payload["checkpoint"]["metadata"].get("included_untracked", [])
        ),
        "excluded": len(payload["checkpoint"]["metadata"].get("excluded", [])),
    }


def repo_metadata(repo: Path) -> dict[str, Any]:
    du = run(["du", "-sk", str(repo)], check=False)
    tracked = git(repo, ["ls-files"]).splitlines()
    return {
        "repository_size_kib": int(du.stdout.split()[0]) if du.returncode == 0 else None,
        "tracked_file_count": len(tracked),
        "filesystem_type": filesystem_type(repo),
    }


def filesystem_type(path: Path) -> str:
    if platform.system() == "Darwin":
        disk = run(["stat", "-f", "%Sd", str(path)], check=False).stdout.strip()
        if disk:
            info = run(["diskutil", "info", "-plist", disk], check=False)
            if info.returncode == 0:
                try:
                    plist = plistlib.loads(info.stdout.encode("utf-8"))
                    for key in ("FilesystemName", "FilesystemType", "Content"):
                        if plist.get(key):
                            return str(plist[key])
                except plistlib.InvalidFileException:
                    pass
        mounted_on = run(["stat", "-f", "%T", str(path)], check=False).stdout.strip()
        return mounted_on or "unknown"

    fs_type = run(["stat", "-f", "-c", "%T", str(path)], check=False).stdout.strip()
    return fs_type or "unknown"


def command_output(command: list[str]) -> str:
    result = run(command, check=False)
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def worktree_dirty() -> bool:
    return bool(command_output(["git", "status", "--short"]))


def power_source() -> str:
    result = run(["pmset", "-g", "batt"], check=False)
    if result.returncode != 0:
        return "unknown"
    first_line = result.stdout.splitlines()[0] if result.stdout.splitlines() else ""
    return first_line.strip() or "unknown"


def format_table(rows: list[tuple[str, str]]) -> str:
    lines = ["| Metric | Value |", "| --- | --- |"]
    lines.extend(f"| {name} | {value} |" for name, value in rows)
    return "\n".join(lines)


def write_report(
    path: Path,
    *,
    args: argparse.Namespace,
    git_commit: str,
    git_version: str,
    repo_info: dict[str, Any],
    idle: dict[str, float],
    burst: dict[str, float],
    checkpoints: list[dict[str, Any]],
    default_wrapper_failed: bool,
) -> None:
    checkpoint_elapsed = [item["elapsed_seconds"] for item in checkpoints]
    report = f"""# Resource Benchmark Results

Date: {datetime.now().astimezone().isoformat(timespec="seconds")}

Scope: initial macOS smoke run for the benchmark harness. These numbers prove
the harness can record idle agent CPU/RSS and checkpoint burst behavior; they
are not release budgets.

## Environment

{format_table([
    ("DevRelay git commit", git_commit),
    ("Worktree dirty during measurement", "yes" if worktree_dirty() else "no"),
    ("OS", f"{platform.system()} {platform.release()}"),
    ("Architecture", platform.machine()),
    ("Git version", git_version),
    ("Filesystem type", str(repo_info["filesystem_type"])),
    ("Power source", power_source()),
    ("Resource profile", "default"),
    ("Watcher backend", "agent foreground / no watcher workload"),
    ("Project count", "1"),
    ("Repository size", f"{repo_info['repository_size_kib']} KiB"),
    ("Tracked file count", str(repo_info["tracked_file_count"])),
    ("Accepted untracked file count", str(checkpoints[-1]["included_untracked"] if checkpoints else 0)),
    ("Sidecar byte count", "0"),
])}

## Idle Agent

{format_table([
    ("Duration", f"{args.idle_seconds:.1f}s"),
    ("Samples", str(int(idle["sample_count"]))),
    ("CPU p50", f"{idle['cpu_p50']:.2f}%"),
    ("CPU p95", f"{idle['cpu_p95']:.2f}%"),
    ("RSS median", f"{idle['rss_mib']:.2f} MiB"),
    ("RSS peak", f"{idle['rss_peak_mib']:.2f} MiB"),
])}

## Checkpoint Burst

{format_table([
    ("Iterations", str(len(checkpoints))),
    ("Agent CPU peak", f"{burst['cpu_peak']:.2f}%"),
    ("Agent RSS peak", f"{burst['rss_peak_mib']:.2f} MiB"),
    ("Checkpoint elapsed p50", f"{statistics.median(checkpoint_elapsed):.3f}s" if checkpoint_elapsed else "0.000s"),
    ("Checkpoint elapsed p95", f"{percentile(checkpoint_elapsed, 0.95):.3f}s" if checkpoint_elapsed else "0.000s"),
    ("Last snapshot id", checkpoints[-1]["snapshot_id"] if checkpoints else "none"),
])}

## Notes

- Default `cargo test --workspace` currently fails in this environment when the
  configured sccache wrapper compiles Tauri proc macros; verification commands
  use `RUSTC_WRAPPER=` until that wrapper issue is fixed.
- Git status frequency, checkpoints per hour, transfer bytes per hour, watcher
  event counts, sidecar hashing time, and SQLite transaction time still require
  internal instrumentation before they can be treated as release evidence.
- Default wrapper build failure observed before this run: {"yes" if default_wrapper_failed else "no"}.
"""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(report, encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", help="Markdown report path")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--cli-bin", help="Path to devrelay CLI binary")
    parser.add_argument("--agent-bin", help="Path to devrelay-agent binary")
    parser.add_argument("--idle-seconds", type=float, default=5.0)
    parser.add_argument("--sample-interval", type=float, default=0.25)
    parser.add_argument("--checkpoint-iterations", type=int, default=5)
    parser.add_argument("--tracked-files", type=int, default=100)
    parser.add_argument(
        "--default-wrapper-failed",
        action="store_true",
        help="Annotate report when the default cargo wrapper is known to fail.",
    )
    args = parser.parse_args()

    build_binaries(args.skip_build)
    cli_bin = ensure_binary(args.cli_bin, REPO_ROOT / "target" / "debug" / "devrelay", "CLI")
    agent_bin = ensure_binary(
        args.agent_bin,
        REPO_ROOT / "target" / "debug" / "devrelay-agent",
        "agent",
    )

    with tempfile.TemporaryDirectory(prefix="devrelay-resource-benchmark-") as temp_raw:
        temp = Path(temp_raw)
        home = temp / "home"
        socket = home / "agent.sock"
        home.mkdir(parents=True)
        repo, manifest = create_benchmark_repo(temp, args.tracked_files)
        process = start_agent(agent_bin, home, socket)
        try:
            cli_call(
                cli_bin,
                home,
                socket,
                [
                    "project",
                    "add",
                    str(repo),
                    "--manifest",
                    str(manifest),
                    "--json",
                ],
            )
            idle_samples = sample_for(
                process.pid,
                args.idle_seconds,
                args.sample_interval,
            )
            done = threading.Event()
            burst_samples: list[Sample] = []

            def sampler() -> None:
                burst_samples.extend(sample_until(process.pid, done, args.sample_interval))

            thread = threading.Thread(target=sampler)
            thread.start()
            checkpoints = [
                checkpoint_once(cli_bin, home, socket, repo, manifest, iteration)
                for iteration in range(1, args.checkpoint_iterations + 1)
            ]
            done.set()
            thread.join()
        finally:
            stop_agent(process)

        report_path = output_path(args.out)
        write_report(
            report_path,
            args=args,
            git_commit=run(["git", "rev-parse", "HEAD"]).stdout.strip(),
            git_version=command_output(["git", "--version"]),
            repo_info=repo_metadata(repo),
            idle=summarize(idle_samples),
            burst=summarize(burst_samples),
            checkpoints=checkpoints,
            default_wrapper_failed=args.default_wrapper_failed,
        )
        print(report_path)


if __name__ == "__main__":
    main()
