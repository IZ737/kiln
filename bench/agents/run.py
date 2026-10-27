#!/usr/bin/env python3
"""Runs coding agents on Kiln's benchmark tasks and scores what they leave.

    python3 bench/agents/run.py --agent 'claude -p "$(cat "$KILN_BENCH_PROMPT")"'
    python3 bench/agents/run.py --agent ./my-agent.sh --tasks repair,http_api --out results.jsonl

For each task, the starting project is copied into a fresh folder and the
agent command runs there with a shell, with:

    KILN_BENCH_PROMPT   the task's prompt (task.md), as a file
    KILN_BENCH_TASK     the task's name
    PATH                with a `kiln` first in it that records every call

Then the task's hidden checks are copied in and run with `kiln check` and
`kiln test --json`, and one JSON line per task is written. See README.md
for what each field means.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
TASKS = HERE / "tasks"

SHIM = """#!/bin/sh
"$KILN_BENCH_REAL" "$@"
code=$?
printf '%s\\t%s\\n' "$code" "$*" >> "$KILN_BENCH_LOG"
exit $code
"""


def overlay(src: Path, dst: Path, ui: str, skip_run: bool = False) -> None:
    for path in sorted(src.iterdir()):
        target = dst / path.name
        if path.is_dir():
            if skip_run and path.name == "run":
                continue
            target.mkdir(parents=True, exist_ok=True)
            overlay(path, target, ui)
        else:
            target.write_text(path.read_text().replace("{{KILN_UI}}", ui))


def files(root: Path) -> dict:
    out = {}
    for path in root.rglob("*"):
        rel = path.relative_to(root)
        if path.is_file() and rel.parts[0] not in (".kiln", ".bench") and path.suffix != ".db":
            out[str(rel)] = path.read_bytes()
    return out


def run_task(task: Path, agent: str, kiln: str, timeout: int, keep: bool) -> dict:
    ui = str((REPO / "wares" / "ui").resolve())
    work = Path(tempfile.mkdtemp(prefix=f"kiln-bench-{task.name}-"))
    project = work / "project"
    project.mkdir()
    overlay(task / "start", project, ui)
    before = files(project)

    bench = work / "bench"
    bench.mkdir()
    prompt = bench / "prompt.md"
    prompt.write_text((task / "task.md").read_text().replace("{{KILN_UI}}", ui))
    shim_dir = bench / "bin"
    shim_dir.mkdir()
    shim = shim_dir / "kiln"
    shim.write_text(SHIM)
    shim.chmod(0o755)
    log = bench / "kiln.log"
    log.touch()

    env = dict(os.environ)
    env.update(
        KILN_BENCH_PROMPT=str(prompt),
        KILN_BENCH_TASK=task.name,
        KILN_BENCH_REAL=kiln,
        KILN_BENCH_LOG=str(log),
        KILN_NO_UPDATE_CHECK="1",
        PATH=f"{shim_dir}{os.pathsep}{env.get('PATH', '')}",
    )
    started = time.time()
    if agent == "oracle":
        # The reference solution, through the shim, to check the harness.
        overlay(task / "solution", project, ui)
        subprocess.run(["kiln", "check", "."], cwd=project, env=env, capture_output=True)
        agent = "true"
    try:
        agent_run = subprocess.run(
            agent, shell=True, cwd=project, env=env, stdin=prompt.open(), timeout=timeout,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        )
        agent_code = agent_run.returncode
        (bench / "agent.log").write_bytes(agent_run.stdout)
    except subprocess.TimeoutExpired:
        agent_code = None
    seconds = round(time.time() - started, 1)

    calls = [line.split("\t", 1) for line in log.read_text().splitlines() if "\t" in line]
    after = files(project)
    changed = sorted(f for f in set(before) | set(after) if before.get(f) != after.get(f))
    # Files the checks bring back must not have been changed by the agent.
    check_files = set(files(task / "check"))
    protected = sorted(f for f in changed if f in check_files and f in before)

    overlay(task / "check", project, ui, skip_run=True)
    quiet = dict(env, PATH=os.environ.get("PATH", ""))
    check = subprocess.run([kiln, "check", "--json", "."], cwd=project, env=quiet,
                           capture_output=True, text=True)
    try:
        errors = json.loads(check.stdout).get("error_count")
    except json.JSONDecodeError:
        errors = None
    test = subprocess.run([kiln, "test", "--json", "."], cwd=project, env=quiet,
                          capture_output=True, text=True, timeout=timeout)
    try:
        report = json.loads(test.stdout)
        summary = report["summary"]
        hidden = [t for t in report["tests"] if "test_bench_" in t["function"] or t["file"] in check_files]
        hidden_passed = sum(1 for t in hidden if t["status"] == "passed")
        failures = [f"{t['id']}: {t.get('failure', {}).get('message', '')}" for t in hidden if t["status"] != "passed"]
    except (json.JSONDecodeError, KeyError):
        summary, hidden, hidden_passed = None, [], 0
        failures = [test.stderr.strip()[-2000:]]

    runs_total = runs_passed = 0
    runs = task / "check" / "run"
    if runs.is_dir():
        for args_file in sorted(runs.glob("*.args")):
            runs_total += 1
            want_code = int((args_file.with_suffix(".code").read_text().strip()
                             if args_file.with_suffix(".code").exists() else "0"))
            out = subprocess.run([kiln, "run", ".", "--", *args_file.read_text().split()],
                                 cwd=project, env=quiet, capture_output=True, text=True)
            want_out = args_file.with_suffix(".out")
            if out.returncode == want_code and (not want_out.exists() or out.stdout == want_out.read_text()):
                runs_passed += 1
            else:
                failures.append(f"run {args_file.stem}: exit {out.returncode}, output {out.stdout[:200]!r}")

    result = {
        "task": task.name,
        "agent_exit": agent_code,
        "seconds": seconds,
        "compiles": check.returncode == 0,
        "check_errors": errors,
        "hidden_tests": sum(text.decode().count("\nfn test_") for text in files(task / "check").values()),
        "hidden_passed": hidden_passed,
        "all_tests": summary,
        "runs": runs_total,
        "runs_passed": runs_passed,
        "solved": check.returncode == 0 and test.returncode == 0 and runs_passed == runs_total,
        "kiln_calls": len(calls),
        "failed_kiln_calls": sum(1 for code, _ in calls if code != "0"),
        "changed_files": changed,
        "protected_changed": protected,
        "failures": failures,
        "folder": str(work) if keep else None,
    }
    if not keep:
        shutil.rmtree(work, ignore_errors=True)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--agent", required=True,
                        help="the shell command that runs the agent; `oracle` applies the reference solutions")
    parser.add_argument("--tasks", help="comma-separated task names (default: all)")
    parser.add_argument("--kiln", default=shutil.which("kiln") or str(REPO / "target" / "debug" / "kiln"),
                        help="the kiln binary to score with")
    parser.add_argument("--timeout", type=int, default=1800, help="seconds per task")
    parser.add_argument("--out", help="also append the JSON lines to this file")
    parser.add_argument("--keep", action="store_true", help="keep each task's folder for inspection")
    args = parser.parse_args()

    names = sorted(p.name for p in TASKS.iterdir() if p.is_dir())
    if args.tasks:
        wanted = args.tasks.split(",")
        unknown = [w for w in wanted if w not in names]
        if unknown:
            print(f"no such task: {', '.join(unknown)}; the tasks are {', '.join(names)}", file=sys.stderr)
            return 2
        names = wanted
    kiln = str(Path(args.kiln).resolve())
    solved = 0
    for name in names:
        result = run_task(TASKS / name, args.agent, kiln, args.timeout, args.keep)
        line = json.dumps(result)
        print(line, flush=True)
        if args.out:
            with open(args.out, "a") as f:
                f.write(line + "\n")
        solved += result["solved"]
    print(f"solved {solved} of {len(names)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
