#!/usr/bin/env python3
"""
Real-time Process-Tree Memory (RSS) and CPU Profiler.

Accurately monitors memory consumption of a command and ALL its recursively spawned
child processes (essential for Remotion which spawns Chromium renderer and GPU processes).
"""

import argparse
import json
import os
import subprocess
import sys
import time
from typing import Any, Dict, List

try:
    import psutil
except ImportError:
    print("Error: psutil is required. Run 'pip3 install psutil'", file=sys.stderr)
    sys.exit(1)


def profile_process_tree(cmd: List[str], interval_sec: float = 0.05, env: Dict[str, str] = None) -> Dict[str, Any]:
    """
    Spawns cmd and profiles RSS memory (in MB) and CPU usage across the entire process tree.
    """
    process_env = os.environ.copy()
    if env:
        process_env.update(env)

    start_time = time.time()
    proc = psutil.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=process_env)

    timeline: List[Dict[str, Any]] = []
    peak_rss_bytes = 0
    total_rss_samples = []
    exit_code = None

    try:
        while True:
            if proc.poll() is not None:
                exit_code = proc.returncode
                break

            current_tree_rss = 0
            current_tree_vms = 0
            child_count = 0

            try:
                # Main process
                main_mem = proc.memory_info()
                current_tree_rss += main_mem.rss
                current_tree_vms += main_mem.vms

                # Children recursively (Chromium processes, ffmpeg, etc.)
                children = proc.children(recursive=True)
                child_count = len(children)
                for child in children:
                    try:
                        cmem = child.memory_info()
                        current_tree_rss += cmem.rss
                        current_tree_vms += cmem.vms
                    except (psutil.NoSuchProcess, psutil.AccessDenied):
                        pass
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                break

            elapsed = time.time() - start_time
            peak_rss_bytes = max(peak_rss_bytes, current_tree_rss)
            total_rss_samples.append(current_tree_rss)

            timeline.append({
                "time_sec": round(elapsed, 3),
                "rss_mb": round(current_tree_rss / (1024 * 1024), 2),
                "vms_mb": round(current_tree_vms / (1024 * 1024), 2),
                "process_count": 1 + child_count,
            })

            time.sleep(interval_sec)

        stdout, stderr = proc.communicate()
    except KeyboardInterrupt:
        proc.kill()
        raise

    duration = time.time() - start_time
    peak_rss_mb = round(peak_rss_bytes / (1024 * 1024), 2)
    avg_rss_mb = round(
        (sum(total_rss_samples) / len(total_rss_samples) / (1024 * 1024))
        if total_rss_samples
        else 0.0,
        2
    )

    # Concurrency capacity modeling on standard servers (reserving 1GB for OS/overhead)
    def calc_capacity(total_ram_mb: int) -> int:
        usable = max(0, total_ram_mb - 1024)
        if peak_rss_mb <= 0:
            return 0
        return int(usable // peak_rss_mb)

    capacity = {
        "server_4gb_ram": calc_capacity(4096),
        "server_8gb_ram": calc_capacity(8192),
        "server_16gb_ram": calc_capacity(16384),
    }

    return {
        "command": cmd,
        "exit_code": exit_code,
        "duration_sec": round(duration, 3),
        "peak_rss_mb": peak_rss_mb,
        "avg_rss_mb": avg_rss_mb,
        "sample_count": len(timeline),
        "concurrency_capacity": capacity,
        "timeline": timeline,
        "stdout": stdout,
        "stderr": stderr,
    }


def main():
    parser = argparse.ArgumentParser(description="Profile memory (RSS) and process count of a process tree.")
    parser.add_argument("--output", "-o", help="Path to write JSON profile result", default=None)
    parser.add_argument("--interval", "-i", type=float, default=0.05, help="Sampling interval in seconds (default: 0.05)")
    parser.add_argument("cmd", nargs=argparse.REMAINDER, help="Command to run and profile")

    args = parser.parse_args()

    cmd = args.cmd
    if cmd and cmd[0] == "--":
        cmd = cmd[1:]

    if not cmd:
        parser.print_help()
        sys.exit(1)

    print(f"[*] Profiling process tree: {' '.join(cmd)}")
    result = profile_process_tree(cmd, interval_sec=args.interval)

    print(f"[+] Finished in {result['duration_sec']}s (Exit code: {result['exit_code']})")
    print(f"[+] Peak RSS: {result['peak_rss_mb']} MB")
    print(f"[+] Avg RSS:  {result['avg_rss_mb']} MB")
    print(f"[+] Concurrency on 4GB RAM Server (usable 3GB):  {result['concurrency_capacity']['server_4gb_ram']} concurrent renders")
    print(f"[+] Concurrency on 8GB RAM Server (usable 7GB):  {result['concurrency_capacity']['server_8gb_ram']} concurrent renders")
    print(f"[+] Concurrency on 16GB RAM Server (usable 15GB): {result['concurrency_capacity']['server_16gb_ram']} concurrent renders")

    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            json.dump(result, f, indent=2)
        print(f"[+] Report written to {args.output}")

    if result["exit_code"] != 0:
        print(f"[!] Warning: Command exited with non-zero code {result['exit_code']}", file=sys.stderr)
        if result["stderr"]:
            print(f"STDERR:\n{result['stderr']}", file=sys.stderr)
        sys.exit(result["exit_code"] or 1)


if __name__ == "__main__":
    main()
