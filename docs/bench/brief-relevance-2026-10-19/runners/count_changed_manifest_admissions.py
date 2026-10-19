from __future__ import annotations

import importlib.util
import json
from pathlib import Path

repo = Path('/Users/livio/Documents/pixel-worktrees/relevant-brief')
negatives = json.loads(Path('/tmp/pixel-brief-coverage-controls/negative-prompts.json').read_text())
baseline_root = Path('/tmp/pixel-brief-coverage-evidence/probe-home/.pixel/plugins')
module_spec = importlib.util.spec_from_file_location('probe_test_admission_count', repo / 'scripts/bench/probe-test.py')
module = importlib.util.module_from_spec(module_spec)
module_spec.loader.exec_module(module)
changed = ['config-registry', 'history-fallback', 'cand-k-commit-patch', 'test-bodies', 'name-files']
for name in changed:
    baseline = module.Probe(baseline_root / name)
    candidate_dir = repo / 'probes' / name
    candidate = module.Probe(candidate_dir)
    print(name)
    for gate in ('open', 'closed'):
        old_rows = [(p, module.gate_runs(gate, baseline) and baseline.applies(None, p)) for p in negatives]
        new_rows = [(p, module.gate_runs(gate, candidate) and candidate.applies(None, p)) for p in negatives]
        old_count = sum(ok for _, ok in old_rows) * 7
        new_count = sum(ok for _, ok in new_rows) * 7
        added = [p for (p, was), (_, now) in zip(old_rows, new_rows) if now and not was]
        print(f'  gate={gate}: baseline={old_count}/112 candidate={new_count}/112 added_prompts={len(added)}')
        for prompt in added:
            print('    added:', prompt)
