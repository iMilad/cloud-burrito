#!/usr/bin/env python3
"""Replay local, synthetic producer/browser fixtures without AWS or native launch.

Raw measurements are intentionally committed separately from generated logs.
This records a warm-filesystem debug harness, not native release performance.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time
import tempfile

try:
    import resource
except ImportError:  # Windows: unavailable is not a zero-memory observation.
    resource = None

ROOT = Path(__file__).resolve().parents[1]


def measurement_precision(value):
    """Discard meaningless float tails; keep every trial and integer count."""
    if isinstance(value, float):
        return round(value, 6)
    if isinstance(value, list):
        return [measurement_precision(item) for item in value]
    if isinstance(value, dict):
        return {key: measurement_precision(item) for key, item in value.items()}
    return value


def capture(args, cwd=ROOT):
    return subprocess.check_output(args, cwd=cwd, text=True, stderr=subprocess.DEVNULL).strip()


def optional(args):
    try:
        return capture(args)
    except (OSError, subprocess.CalledProcessError):
        return None


def metadata():
    production = [path for path in (ROOT / "src-tauri/src").rglob("*.rs")
                  if not path.name.startswith(("benchmarks", "test_"))]
    production += list((ROOT / "frontend").glob("*.*"))
    return {
        "source_revision": capture(["git", "rev-parse", "HEAD"]),
        "working_tree_dirty": bool(capture(["git", "status", "--porcelain"])),
        "production_sha256": {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                              for p in sorted(production) if p.is_file()},
        "os": platform.system(), "os_release": platform.release(),
        "os_version": optional(["sw_vers", "-productVersion"]) if sys.platform == "darwin" else platform.version(),
        "architecture": platform.machine(), "logical_cpu_count": os.cpu_count(),
        "machine_class": optional(["sysctl", "-n", "hw.model"]) if sys.platform == "darwin" else None,
        "physical_memory_bytes": optional(["sysctl", "-n", "hw.memsize"]) if sys.platform == "darwin" else None,
        "power_mode": "unmeasured", "machine_cold": False, "os_cache_flush": False,
        "toolchain": {"rustc": capture(["rustc", "--version"]), "cargo": capture(["cargo", "--version"]), "node": capture(["node", "--version"])},
        "backend_profile": "debug", "browser": "installed Chrome, frontend-only synthetic IPC",
        "native_webview_version": None,
    }


def run_backend(test, environment, progress_path):
    before = resource.getrusage(resource.RUSAGE_CHILDREN) if resource else None
    started = time.monotonic()
    command = ["cargo", "test", "-p", "cloud-burrito", "--locked", "--offline", "--lib", test,
               "--", "--ignored", "--exact", "--nocapture"]
    measurements = []
    with tempfile.TemporaryFile(mode="w+") as errors, progress_path.open("a") as progress:
        process = subprocess.Popen(command, cwd=ROOT / "src-tauri", env=environment,
                                   text=True, stdout=subprocess.PIPE, stderr=errors)
        for line in process.stdout:
            if "CB_BENCH_JSON " in line:
                value = json.loads(line.split("CB_BENCH_JSON ", 1)[1])
                measurements.append(value)
                progress.write(json.dumps(value) + "\n")
                progress.flush()
                if value.get("kind") == "summary":
                    print(f"Recorded {value['scenario']}: {value['trials']} trials", flush=True)
        status = process.wait()
        if status:
            errors.seek(0)
            sys.stderr.write(errors.read()[-12000:])
            raise SystemExit(status)
    if not measurements:
        raise SystemExit("benchmark returned no measurements")
    after = resource.getrusage(resource.RUSAGE_CHILDREN) if resource else None
    return {"test": test, "elapsed_seconds_including_cargo": time.monotonic() - started,
            "child_user_cpu_seconds": after.ru_utime - before.ru_utime if after else None,
            "child_system_cpu_seconds": after.ru_stime - before.ru_stime if after else None,
            "cumulative_child_peak_rss_bytes": int(after.ru_maxrss * (1 if sys.platform == "darwin" else 1024)) if after else None,
            "memory_scope": "cumulative child high-water mark includes Cargo/compiler, test harness and fixtures; not native app memory or retained memory",
            "measurements": measurements}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--suite", choices=["backend", "browser", "all"], default="all")
    parser.add_argument("--chrome", default="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
    args = parser.parse_args()
    result = {"schema_version": 1, "recorded_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "metadata": metadata(), "limitations": ["No AWS or native app was launched.",
                "Backend and browser fixtures are separate; durations must not be added as native end-to-end evidence.",
                "Native launch, verified connection time, total backend+webview memory, idle CPU and supported-device journeys remain unmeasured."],
              "backend": [], "browser": None}
    environment = os.environ.copy()
    environment["CLOUD_BURRITO_BENCH_SOURCE"] = result["metadata"]["source_revision"]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    def save():
        args.output.write_text(json.dumps(measurement_precision(result), indent=2) + "\n")
    save()
    progress_path = args.output.with_suffix(".progress.jsonl")
    if args.suite in ("backend", "all"):
        for test in ["benchmarks::replay_backend_baseline", "benchmarks_cli::replay_cli_baseline"]:
            print(f"Replaying {test}", flush=True)
            result["backend"].append(run_backend(test, environment, progress_path))
            save()
    if args.suite in ("browser", "all"):
        browser_output = args.output.with_suffix(".browser.json")
        subprocess.run(["node", "tests/performance/browser-benchmark.mjs", "--chrome", args.chrome,
                        "--output", str(browser_output)], cwd=ROOT, check=True)
        result["browser"] = json.loads(browser_output.read_text())
        save()
        browser_output.unlink()
    save()
    if progress_path.exists():
        progress_path.unlink()
    print(f"Saved {args.output.relative_to(ROOT) if args.output.is_relative_to(ROOT) else args.output.name}", flush=True)


if __name__ == "__main__":
    main()
