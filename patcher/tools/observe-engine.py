#!/usr/bin/env python3
"""Observe an already-built Rust CLI on fresh synthetic content; never builds or publishes."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import time
import os


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine", type=Path)
    parser.add_argument("output", type=Path, help="New directory for fixture, logs and evidence.json")
    args = parser.parse_args()
    engine = args.engine.resolve(strict=True)
    output = args.output.absolute()
    output.mkdir(parents=True, exist_ok=False)
    fixture = output / "fixture"
    config = fixture / "config.json"
    game = fixture / "game"
    commands = []

    def run(name, arguments, expected=0):
        started = time.monotonic()
        command = [str(engine), *map(str, arguments)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=120)
        (output / f"{name}.stdout.jsonl").write_text(result.stdout)
        (output / f"{name}.stderr.txt").write_text(result.stderr)
        commands.append({"name": name, "argv": command, "exit_code": result.returncode,
                         "elapsed_seconds": round(time.monotonic() - started, 3)})
        require(result.returncode == expected, f"{name}: unexpected exit {result.returncode}: {result.stderr}")
        return result

    def policy():
        return json.loads((fixture / "state/policy.json").read_text())["payload"]

    def report(result):
        records = [json.loads(line) for line in result.stdout.splitlines()]
        require(any(item.get("phase") == "ready" for item in records), "No Ready progress")
        return records[-1]

    run("create", ["fixture", fixture])
    original_policy = policy()
    unrelated = game / "unmanaged.txt"
    unrelated.write_bytes(b"unrelated fixture data must survive\n")
    interrupted = run("interrupted", ["run", "--config", config, "--interrupt-after-rename", "1"], 1)
    require("Injected interruption" in interrupted.stderr, "Run did not reach the injected boundary")
    require(policy()["applied"] is None, "Interrupted run claimed applied policy")
    require('"phase":"ready"' not in interrupted.stdout, "Interrupted run claimed Ready")
    committed = game / "Data/base.package"
    require(committed.read_bytes() == (fixture / "input/base").read_bytes(), "Interrupted commit bytes differ")
    committed_mtime = committed.stat().st_mtime_ns
    recovered = report(run("recovered", ["run", "--config", config]))
    require(recovered["replaced"] == ["EP01/content.package"], "Recovery repeated the committed replacement")
    require(committed.stat().st_mtime_ns == committed_mtime, "Recovery rewrote the committed file")
    expected = {"Data/base.package": "base", "EP01/content.package": "EP01"}
    comparisons = []
    for destination, source in expected.items():
        actual = game / destination
        original = fixture / "input" / source
        require(actual.read_bytes() == original.read_bytes(), f"Recovery bytes differ: {destination}")
        comparisons.append({"path": destination, "bytes": actual.stat().st_size,
                            "actual_sha256": sha256(actual), "source_sha256": sha256(original)})
    require(not (game / "SP01/content.package").exists(), "Disabled DLC was installed")
    # Damage content while retaining size and mtime, then request explicit full verification.
    damaged = game / "EP01/content.package"
    before = damaged.stat()
    damaged.write_bytes(b"x" * before.st_size)
    os.utime(damaged, ns=(before.st_atime_ns, before.st_mtime_ns))
    repaired = report(run("repaired", ["run", "--config", config, "--verify"]))
    require(repaired["replaced"] == ["EP01/content.package"], "Repair changed unexpected files")
    require(damaged.read_bytes() == (fixture / "input/EP01").read_bytes(), "Repair bytes differ")
    mtimes = {path: (game / path).stat().st_mtime_ns for path in expected}
    noop_result = run("noop", ["run", "--config", config])
    noop = report(noop_result)
    require(noop["replaced"] == [] and noop["deleted"] == [], "No-op mutated files")
    require(not any(json.loads(line).get("phase") == "downloading"
                    for line in noop_result.stdout.splitlines()), "No-op downloaded objects")
    require(mtimes == {path: (game / path).stat().st_mtime_ns for path in expected}, "No-op changed mtimes")
    require(unrelated.read_bytes() == b"unrelated fixture data must survive\n", "Unrelated file changed")
    final_policy = policy()
    require(final_policy["desired"] == original_policy["desired"], "Choices changed")
    require(final_policy["applied"]["revision"] == final_policy["revision"], "Applied revision differs")
    evidence = {
        "schema": 1, "observed_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "platform": platform.platform(), "engine_sha256": sha256(engine),
        "scope": "Synthetic signed CLI fixture; no real game, platform unlock or online handoff established",
        "commands": commands, "recovery": recovered, "repair": repaired, "noop": noop,
        "byte_comparisons": comparisons, "disabled_pack_absent": True,
        "choices_preserved": True, "unrelated_file_preserved": True,
        "applied": final_policy["applied"],
    }
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    main()
