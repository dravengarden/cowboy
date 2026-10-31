#!/usr/bin/env python3
"""Hibernate one Machine's idle sessions before a worker-generation rollout.

Runs on Hawk through this computer's `cowboy login`, as the Web UI does. Every
idle (`running`) session of the Machine, default or named Provider account, is
hibernated so the rollout has no worker to replace for it; it resumes on the
new generation when next opened. A session the Controller or Machine refuses
(a turn, permission or background task started meanwhile) is left retained:
the rollout drains it at its native safe boundary and acceptance still requires
its exact worker or one replacement. Busy, crashed and dormant sessions are not
touched. The summary is written for the maintenance receipt.

See docs/ovh-machine-maintenance.md.
"""

import argparse
import json
import subprocess
import sys
import time

SCHEMA = 1
SETTLE_SECONDS = 90


def cowboy(*args):
    return subprocess.run(["cowboy", *args], capture_output=True, text=True, timeout=60)


def list_sessions():
    result = cowboy("session", "list", "--json")
    if result.returncode != 0:
        raise RuntimeError("cowboy session list failed: " + result.stderr.strip())
    rows = json.loads(result.stdout)
    if not isinstance(rows, list):
        raise RuntimeError("cowboy session list returned no session array")
    return rows


def idle_sessions(rows, machine):
    """Sessions whose worker on `machine` is live and idle, in ID order."""
    return sorted(
        row["id"]
        for row in rows
        if row.get("machine_id") == machine
        and row.get("status") == "running"
        and not row.get("system", False)
    )


def unsettled(rows, hibernated):
    """Hibernated sessions whose worker stop the Controller has not observed."""
    status = {row["id"]: row.get("status") for row in rows}
    return sorted(id for id in hibernated if status.get(id) not in ("exited", None))


def hibernate_idle(machine, *, run=cowboy, sessions=list_sessions,
                   sleep=time.sleep, settle_seconds=SETTLE_SECONDS):
    hibernated, retained = [], []
    for session in idle_sessions(sessions(), machine):
        result = run("session", "hibernate", session)
        if result.returncode == 0:
            hibernated.append(session)
        else:
            reason = (result.stderr or result.stdout).strip()[-400:]
            retained.append({"id": session, "reason": reason})
    # Each worker must be gone before the receipt records live workers:
    # a stop completing after that would read as a retained worker changing.
    deadline = time.monotonic() + settle_seconds
    while pending := unsettled(sessions(), hibernated):
        if time.monotonic() >= deadline:
            raise RuntimeError("hibernated sessions did not stop in time: " + ", ".join(pending))
        sleep(2)
    return {"schema": SCHEMA, "machine": machine, "hibernated": hibernated, "retained": retained}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--machine", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    summary = hibernate_idle(args.machine)
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(summary, handle)
    for item in summary["retained"]:
        print(f"retained {item['id']}: {item['reason']}", file=sys.stderr)
    print(f"hibernated {len(summary['hibernated'])} idle sessions on {args.machine}, "
          f"{len(summary['retained'])} refused and retained")


if __name__ == "__main__":
    main()
