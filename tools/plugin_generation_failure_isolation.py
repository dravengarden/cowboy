#!/usr/bin/env python3
"""Observe broken-generation failure isolation; never accept a Plugin release.

This separate diagnostic starts a healthy candidate before an exact historical
generation that must explicitly reject startup before native-session allocation.
It proves neither successful coexistence nor migration of an existing session.
"""

import argparse
import json
from pathlib import Path
import tempfile

from plugin_runtime_conformance import (
    Candidate, Worker, WorkerStartupRejected, cleanup_workers, digest, evidence_identity, require,
    require_worker_isolation, validate_receipt_paths, write_receipt,
)


def run(args):
    with tempfile.TemporaryDirectory(prefix="cw-failure-isolation-", dir="/tmp") as temporary:
        root = Path(temporary).resolve()
        candidate = Candidate(args.release.resolve(), args.artifacts.resolve(), root / "candidate")
        previous = Candidate(args.previous.resolve(), args.previous_artifacts.resolve(), root / "previous")
        require(candidate.id == previous.id
                and candidate.release["artifact_digest"] != previous.release["artifact_digest"],
                "diagnostic requires two distinct exact releases of one Plugin")
        workers = []
        try:
            current = Worker(candidate, args.worker, root / "new")
            workers.append(current)
            current.assert_alive()
            try:
                workers.append(Worker(previous, args.worker, root / "old"))
            except WorkerStartupRejected:
                # Only a terminal pre-native-session event followed by completed
                # cleanup qualifies. Timeout, handshake, probe, sidecar and
                # cleanup errors are not an expected historical rejection.
                pass
            else:
                raise RuntimeError("previous generation became ready; use the normal coexistence gate")
            current.assert_alive()
            current.stop()
        finally:
            cleanup_workers(workers)
        return {
            "schema": "dravengarden.cowboy.plugin-generation-failure-isolation/v1",
            "status": "observed_failure_isolation",
            "candidate": evidence_identity(candidate), "previous": evidence_identity(previous),
            "worker_executable_digest": digest(args.worker),
            "platform": {"os": candidate.target["os"], "architecture": candidate.target["architecture"]},
            "checks": {
                "both_artifact_sets_probed": True,
                "candidate_initialize_and_session_new": True,
                "previous_rejected_before_native_allocation": True,
                "previous_recorded_descendants_drained": True,
                "candidate_survived_previous_failure_and_cleanup": True,
                "candidate_stop_and_descendant_drain": True,
            },
            "release_accepted": False, "distinct_generation_coexistence": "not_proven",
            "uses_service_credentials": False, "sends_prompt": False,
            "signature_verification": "separate_required_gate",
            "not_checked": ["existing_session_resume", "native_history_preservation",
                            "machine_installation", "service_authentication_migration",
                            "catalog_retirement", "publication"],
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("release", type=Path)
    parser.add_argument("artifacts", type=Path)
    parser.add_argument("--worker", required=True, type=Path)
    parser.add_argument("--previous", required=True, type=Path)
    parser.add_argument("--previous-artifacts", required=True, type=Path)
    parser.add_argument("--receipt", required=True, type=Path)
    args = parser.parse_args()
    validate_receipt_paths(args.receipt, None)
    require_worker_isolation()
    require(args.worker.is_absolute(), "worker must be an exact absolute build result")
    digest(args.worker)
    report = run(args)
    write_receipt(args.receipt, report)
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
