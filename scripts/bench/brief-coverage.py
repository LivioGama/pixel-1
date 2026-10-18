#!/usr/bin/env python3
# SPDX-FileCopyrightText: The Pixel contributors
# SPDX-License-Identifier: MIT
"""Deterministic coverage check for `pixel brief` over one prompt per question class.

Each case runs `pixel brief --json <prompt> <repo>` and is scored from the brief text and the
decision record's `coverage` field (`full`, `partial`, `degraded`, `none`):

  served    the brief holds at least one expected marker and none of the forbidden ones
  signaled  the brief did not serve the prompt, but said so (`coverage` is `degraded` or `none`,
            and the brief does not end on the "Answer from this context" directive)
  SILENT    the brief did not serve the prompt and still claimed it did: the failure this check
            exists to catch

Exit status is 1 when any case is SILENT, so the check can gate a change. The corpus targets this
repository; pass `--repo` for a checkout of it and `--binary` for the pixel to measure.

    scripts/bench/brief-coverage.py --binary target/dev-release/pixel
    PIXEL_BRIEF_PROBES=0 scripts/bench/brief-coverage.py   # the Rust pipeline alone
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

DIRECTIVE = "Answer from this context. Use a tool only if something you need is not shown above."

CORPUS = [
    {"class": "exact_symbol", "prompt": "where is pick_sites defined and who calls it",
     "expected_any": ["fn pick_sites"], "forbidden": ["change summary — commit"]},
    {"class": "qualified_symbol", "prompt": "how does QuestionKind::heuristic route prompts and where is it called",
     "expected_any": ["fn heuristic"]},
    {"class": "cli_subcommand", "prompt": "what does pixel prepare-repo do and where is it implemented",
     "expected_any": ["prepare_repo", "prepare-repo"]},
    {"class": "dotted_key", "prompt": "when does doctor report graph.freshness red",
     "expected_any": ["graph.freshness", "freshness"]},
    {"class": "conceptual_flow", "prompt": "walk me through what happens when a prompt is submitted to pixel brief",
     "expected_any": ["execution_brief", "start_in", "finish_with_record"], "forbidden": ["change summary — commit"]},
    {"class": "config_env", "prompt": "which environment variables and flags control the brief parts and pack limits",
     "expected_any": ["PIXEL_BRIEF_PARTS", "PIXEL_BRIEF_PACK", "limit_for"], "forbidden": ["change summary — commit"]},
    {"class": "symptom_review_words",
     "prompt": "check why `pick_sites` in the working tree or before changing code picks import lines instead of call sites",
     "expected_any": ["fn pick_sites"], "forbidden": ["change summary — commit"]},
    {"class": "harness_wrapper",
     "prompt": "<USER_REQUEST>\nWhy does the brief fall back to closest regions when a prompt matches no symbol "
               "definition in execution_brief?\n</USER_REQUEST>",
     "expected_any": ["region_pack.rs", "execution_brief"], "forbidden": ["USER_REQUEST", "change summary — commit"]},
    {"class": "usage_lookup", "prompt": "where is QuestionKind matched or used across execution_brief",
     "expected_any": ["enum QuestionKind"]},
    {"class": "unresolvable_symbol", "prompt": "where is `nonexistent_xyz_symbol_999` defined and who calls it",
     "expected_any": []},
]


def run_case(binary, repo, case):
    proc = subprocess.run([binary, "brief", "--json", case["prompt"], str(repo)],
                          capture_output=True, text=True, cwd=str(repo), timeout=60)
    data = json.loads(proc.stdout) if proc.stdout.strip() else {}
    brief = data.get("brief") or ""
    coverage = data.get("coverage") or ("none" if not brief else None)
    hit = any(marker in brief for marker in case["expected_any"])
    bad = [marker for marker in case.get("forbidden", []) if marker in brief]
    claims = DIRECTIVE in brief or coverage == "full"
    if hit and not bad:
        verdict = "served"
    elif not claims and coverage in ("degraded", "none"):
        verdict = "signaled"
    else:
        verdict = "SILENT"
    why = [f"forbidden {marker!r}" for marker in bad]
    if not hit and case["expected_any"]:
        why.append("no expected marker")
    return {"class": case["class"], "verdict": verdict, "kind": data.get("kind"), "coverage": coverage,
            "bytes": len(brief), "why": why}


def main():
    root = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True,
                               cwd=Path(__file__).parent).stdout.strip() or ".")
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--binary", default="pixel")
    parser.add_argument("--repo", default=str(root))
    parser.add_argument("--json", action="store_true", help="one JSON object per case")
    args = parser.parse_args()
    results = [run_case(args.binary, Path(args.repo), case) for case in CORPUS]
    for row in results:
        if args.json:
            print(json.dumps(row))
        else:
            print(f"{row['verdict']:8} {row['class']:22} kind={row['kind']} coverage={row['coverage']} "
                  f"bytes={row['bytes']} {'; '.join(row['why'])}")
    counts = {v: sum(r["verdict"] == v for r in results) for v in ("served", "signaled", "SILENT")}
    print(f"served {counts['served']}/{len(results)}, signaled {counts['signaled']}, silent {counts['SILENT']}")
    sys.exit(1 if counts["SILENT"] else 0)


main()
