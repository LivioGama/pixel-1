#!/usr/bin/env python3
"""Check archived identities and count recorded judgments; does not judge relevance."""

import gzip
import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent


def read_rows(path):
    text = gzip.decompress(path.read_bytes()).decode() if path.suffix == ".gz" else path.read_text()
    rows = [json.loads(line) for line in text.splitlines()]
    indexed = {row["id"]: row for row in rows}
    assert len(indexed) == len(rows), f"duplicate IDs: {path}"
    return indexed


def check_arm(arm):
    report = json.loads((ROOT / f"{arm}.json").read_text())
    sessions = {row["id"]: row for row in report["sessions"]}
    reviews = read_rows(ROOT / f"{arm}-review.jsonl")
    briefs = read_rows(ROOT / f"{arm}-briefs.jsonl.gz")
    assert len(sessions) == 56
    assert sessions.keys() == reviews.keys() == briefs.keys()
    assert report["result"]["exit_code"] == 0
    for question_id, row in reviews.items():
        brief = briefs[question_id]
        digest = hashlib.sha256(brief["brief"].encode()).hexdigest()
        assert digest == brief["sha256"] == row["brief_sha256"], question_id
        assert row["repo_sha"] == sessions[question_id]["sha"], question_id
        assert type(row["relevant"]) is bool, question_id
        assert row["reason"], question_id
    relevant = sum(row["relevant"] for row in reviews.values())
    print(f"{arm}: {relevant}/{len(reviews)} relevant ({100 * relevant / len(reviews):.2f}%)")
    print(f"  native outcomes: {report['result']['counts']}")
    print(f"  p95: {report['means']['answer']['latency']['p95_ms']} ms")
    return report, reviews


baseline_report, baseline = check_arm("paired-baseline")
candidate_report, candidate = check_arm("paired-candidate")
assert baseline_report["binary"]["sha256"] == candidate_report["binary"]["sha256"]
assert baseline_report["scorer"] == candidate_report["scorer"]
assert baseline_report["questions"] == candidate_report["questions"]
gains = sorted(key for key in baseline if not baseline[key]["relevant"] and candidate[key]["relevant"])
losses = sorted(key for key in baseline if baseline[key]["relevant"] and not candidate[key]["relevant"])
print(f"Gains ({len(gains)}): {', '.join(gains)}")
print(f"Losses ({len(losses)}): {', '.join(losses) or 'none'}")
candidate_briefs = read_rows(ROOT / "paired-candidate-briefs.jsonl.gz")
repeated = read_rows(ROOT / "paired-candidate-repeat-briefs.jsonl.gz")
changed = {key for key in baseline if baseline[key]["brief_sha256"] != candidate[key]["brief_sha256"]}
assert repeated.keys() == changed
for key, row in repeated.items():
    assert row["brief"] == candidate_briefs[key]["brief"], key
repeat_report = json.loads((ROOT / "paired-candidate-repeat.json").read_text())
assert repeat_report["result"]["exit_code"] == 0
assert repeat_report["result"]["counts"] == {"ok": len(repeated)}
print(f"Changed briefs byte-identical on repetition: {len(repeated)}/{len(changed)}")
ci = json.loads((ROOT / "ci-results.json").read_text())
published = json.loads((ROOT / "pr-publication.json").read_text())
published_heads = {row["number"]: row["headRefOid"] for row in published["pull_requests"]}
assert {row["number"] for row in ci["prs"]} == set(published_heads)
for row in ci["prs"]:
    assert row["head_sha"] == row["ci_run"]["head_sha"] == published_heads[row["number"]]
    assert row["ci_run"]["status"] == "completed"
    assert row["ci_run"]["conclusion"] == "success"
    checks = {check["name"]: check["conclusion"] for check in row["checks"]}
    assert checks["Test + Format"] == "success"
    assert all(checks[name] in {"success", "skipped"} for name in ci["required_contexts"])
print(f"Archived successful current-head CI receipts: {len(ci['prs'])}/{len(published_heads)}")
print("Archived hashes and denominators verified; relevance labels require source review.")
