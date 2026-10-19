# Deterministic brief relevance, 2026-10-19

The matched public benchmark improved from **38/56 relevant briefs (67.9%) to
45/56 (80.4%)**: seven gains and no relevance regressions. The candidate produced
56 complete briefs; the baseline produced 53 and abstained on three. This is a
development-set result, not an estimate for unseen user prompts.

Recount the archived judgments and verify their brief hashes and pinned source
commits from the repository root:

```sh
rtk python3 docs/bench/brief-relevance-2026-10-19/verify.py
```

| Matched native-hook measurement | Baseline | Candidate |
| --- | ---: | ---: |
| Source-reviewed relevant briefs | 38/56 (67.86%) | 45/56 (80.36%) |
| Complete / partial / abstained | 53 / 0 / 3 | 56 / 0 / 0 |
| p95 elapsed time | 1,091 ms | 1,193 ms |
| Mean exact oracle-line coverage | 11% | 11% |
| Mean oracle-file recall | 42% | 45% |

Exact metrics, question IDs, source commits, binary and probe hashes are in the
two scorer reports. Latencies are observations from one warm run per arm, not a
statistically established performance change. Oracle-line overlap is separate
from relevance and does not explain the headline percentage.

## What counts as relevant

The criterion was fixed before reviewing candidate output: a brief passes when
it contains an exact substantive excerpt that answers a requested fact or
correctly locates the requested source (architecture anchor, value/configuration
behavior, test case, caller, error-producing path, flow boundary, or requested
commit). Lexical neighbors, unrelated setup, wrong features/commits, and uncertain
cases fail. Partial useful evidence may pass; nonempty output alone does not.

This is nonblinded assistant source review, not independent human evaluation.
The baseline ledger inherits 51 byte-identical previously reviewed briefs and
reviews five changed briefs against pinned source. The candidate ledger inherits
36 byte-identical baseline judgments and reviews all 20 changed briefs. A second
assistant cross-checked the eight initially claimed gains. Each ledger row
records evidence, reason, review basis, brief hash, and source commit.

That cross-check rejected `fd-06`: its excerpt exercises `--exec-batch`, while
the prompt requests the distinct `--exec` option. The initial candidate review
counted it as exec-family evidence. The conservative final review excludes it,
correcting 46/56 to 45/56; both ledgers and the correction are retained.
The relevance criterion did not change.

The same public questions guided the improvements. No private held-out data was
available. Precision across all text within a passing brief, complete answers,
zero-tool-call answers, and relevance on unfamiliar prompts are not established.

## Seven recovered questions

| Question | Relevant evidence in the candidate | Change |
| --- | --- | --- |
| `requests-06` | Redirect tests, `tests/test_requests.py:2286-2325` | Admit explicit test-location questions after gate abstention |
| `excalidraw-02` | `MAX_ZOOM = 30`, `packages/common/src/constants.ts:367` | Admit bounds intent and require a matching bound declaration |
| `express-02` | `this.set('etag', 'weak')`, `lib/application.js:93-96` | Quote literal setting-call source with surrounding control flow |
| `pixel-05` | Region-pack introduction, commit `20435b5b0a3d7179c79ed45a9f2c0ee60588ebf9` | Prefer specific commit-subject matches before message fallback |
| `excalidraw-05` | Wrap/unwrap shortcut, commit `704db6803c4371fa59e06030f1ca4323ed580386` | Same history selection in both readers |
| `fd-07` | Existing-directory guard and Search path diagnostic, `src/cli.rs:710-722` | Quote bounded diagnostic-emitter candidates |
| `fd-08` | `src/exit_codes.rs:1-24` and imports in exec/main/walk | Match compound module names and quote source plus references |

Source retrieval uses deterministic token rules, tracked local source reads,
and git history. No LLM call or new model dependency was added. The optional LLM
intent classifier was disabled in both benchmark homes. The existing local
embedding/ranking model remained present and was held identical between arms.
Time budgets and host scheduling can still affect which probes finish.

Diagnostic and setter excerpts are explicitly candidates: they do not assert the
runtime cause of an error or the effective value after all configuration layers.
The additions retain tracked-file, symlink, size, process, and output bounds.

## Frozen experiment

- Native binary source: `1d1c063de058fb132d84623e67a199aaf3308904`.
- Binary SHA-256: `0772041bc814c6bfd65c76c5a70c29fe60537dd94649fec143cc48849385a38c`.
- Candidate source: `7dc83944d88d91d7eb170d6860c9d964d5d4acef`.
- Both arms use that native binary. Candidate probes are overlaid in an isolated
  home; no Rust change is involved. `candidate-identity.json` records their hashes.
- All 56 questions and seven source commits are identical. Source clones were
  verified clean at their pinned commits.
- Separate baseline and candidate roots were copied from one prepared cache/index
  seed; their isolated homes use the same model files and configuration. The
  candidate root was untouched until its run. Runs were serial.
- `classify.enabled: false` is identical in both homes.

Full invocations and environment fingerprints are in `reproducibility.json` and
the exact executed scripts in `runners/`. Those scripts preserve this run's local
paths and assume the matching binary, isolated homes, and prepared roots exist.
The native route is the existing
`scripts/bench/tool-census/score_brief.py --route hook`, using frozen probe homes.
These are native-hook results, not substituted direct-probe results.

## Checks and limits

At the frozen candidate, 68 focused tests passed:

```sh
rtk python3 probes/history-fallback/tests/test_subjects.py    # 4, both readers
rtk python3 probes/config-registry/tests/test_probe.py       # 29
rtk python3 probes/error-origin/tests/test_probe.py          # 23
rtk python3 probes/name-files/tests/test_probe.py            # 4
rtk python3 probes/test-bodies/tests/test_probe.py            # 8
rtk python3 -I scripts/bench/probe-test.py --check-all        # manifest contract
rtk python3 -I scripts/bench/probe-test.py --self-test        # 24 harness tests
```

`focused-checks.json` records the source hashes, commands, and exits for the
focused suites and manifest check. History tests preserve exact quoted titles
and tags while rejecting an incidental-body commit. Direct repeated history
checks select the expected identity for all seven questions in both readers
(14/14 pairs, byte-identical on repetition); this does not prove whole-hook byte
stability under every workload. Repeating all 20 native-hook questions whose
briefs changed produced byte-identical output in 20/20 cases; `verify.py` checks
the archived repeat against the first candidate.

Negative controls cover unrelated bound subjects, fictional test topics, noisy
module overlaps, quoted-error precedence, untracked/symlink files, and candidate
context. Broad workflow-prompt admission controls are not a precision estimate:
many never reach a probe. Existing fallback noise and false positives must still
be considered when using the emitted candidates. The direct near-miss sweep
(`runners/check_final_changed_probe_negatives.py`, 63 cases across seven repos)
emits zero bytes for all 14 bound, seven fictional-module, and seven forced-kind
fictional-test cases. Sampled fictional-history/commit and nonexistent-error
outputs remain byte-identical to baseline; these are retained limitations, not
claimed precision wins. The forced-kind Pixel test example drops from 8,483 bytes
of unrelated tests to zero. Full results and sampled text are archived.

No managed installation, release, or mutation campaign was performed. Five
independently reviewable PRs are open, and the combined measured snapshot is
pushed. All five current-head CI runs passed. The `Test + Format` CI job executes the probe
manifest/harness checks and every `probes/*/tests/test_*.py` file, including all
five focused suites above. These results do not claim the installed binary changed.

| Concern | Pull request | Tested branch head |
| --- | --- | --- |
| Test admission | [#1255](https://github.com/Pixel-CLI/pixel/pull/1255) | `c96bed60b230d21beceb047677b925590f1b8087` |
| History subjects | [#1256](https://github.com/Pixel-CLI/pixel/pull/1256) | `72bd82d7e6f8584f36e4b0caf133c6a23a6ed50e` |
| Configuration source | [#1257](https://github.com/Pixel-CLI/pixel/pull/1257) | `95bb09d6deb9cf0d78f18af5b49dfbdbefcfeec6` |
| Diagnostic emitters | [#1258](https://github.com/Pixel-CLI/pixel/pull/1258) | `72680bd49d62d56f736b24bb51b7d60a6d8b77e4` |
| Compound modules | [#1259](https://github.com/Pixel-CLI/pixel/pull/1259) | `866aa962e53a8757b512f17849cab9464e39cb59` |

Each PR references the still-open [#906](https://github.com/Pixel-CLI/pixel/issues/906)
epic; these changes do not claim its broader equal-answer/zero-call acceptance.
`ci-results.json` records each exact head, workflow run, and check conclusion.
Required checks have no failures or pending results; dependency policy is
skipped because these PRs change no dependencies. All five PRs are conflict-free
and await the repository's required human review. No merge was performed.

## Earlier runs and discarded hypotheses

The approximately 58% figure found during investigation is hit@8 (22/38) in
`docs/bench/brief-recall-2026-10-18-unit3/README.md`, not a confirmed rate of
emitted briefs for 50 prompts. Its private data was unavailable here. The public
56-question measurement has a different, explicit denominator.

The original baseline and first candidate used different model/cache availability.
An apparent `excalidraw-03` regression disappeared in a matched single-question
control: both arms emitted the same 3,442-byte brief, SHA-256
`46c6922eab01223b3f3937f1511142b02612a59a581f70819700a6447ba29743`.
The full matched runs above replace that confounded comparison. Historical
`main-1d1c063d*.json` reports and `baseline-review*.jsonl` are retained only for
audit; do not mix their labels or timings with the paired result.

The initial historical review missed the exact `excalidraw-07` throw site and was
corrected from 37/56 to 38/56. The original ledger and correction record are kept.

Dropped prototypes: broad lexical source windows (no recovery and excessive cold
latency), a dotted-member caller shortcut (examples/tests instead of library
callers), and translation-key expansion (no incremental relevant brief). None
was ported into the candidate.

## Artifacts

- `paired-baseline.json`, `paired-candidate.json`: native scorer reports.
- `paired-*-review.jsonl`: all 56 source judgments per arm.
- `paired-candidate-review-initial.jsonl`: pre-cross-check judgment audit.
- `paired-*-briefs.jsonl.gz`: complete emitted public-source text, with IDs/hashes.
- `brief-archive-manifest.json`: deterministic compressed archive hashes.
- `candidate-identity.json`, `focused-checks.json`: code and local check identities.
- `pr-publication.json`, `ci-results.json`: PR identities and current-head CI receipts.
- `reproducibility.json`, `runners/`: cache/model/config fingerprints and commands.
- `paired-candidate-repeat.json`, `paired-candidate-repeat-briefs.jsonl.gz`:
  native repeat of all 20 changed briefs.
- `paired-final-negative-results.json`, `negative-output-samples.jsonl.gz`:
  near-miss output checks and representative before/after text.
- `verify.py`: checks archive/ledger/source identities and recounts the labels;
  it does not automatically decide relevance.

Independent branches: `fix/brief-test-probe-admission`,
`fix/brief-history-subjects`, `fix/brief-config-evidence`,
`fix/brief-diagnostic-evidence`, and `fix/brief-module-evidence`.
The combined code and this report are on `feat/relevant-brief`.
