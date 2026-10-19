from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import time
from pathlib import Path

repo = Path('/Users/livio/Documents/pixel-worktrees/relevant-brief')
baseline_home = Path('/tmp/pixel-brief-coverage-evidence/paired-baseline-home/.pixel/plugins')
candidate_home = Path('/tmp/pixel-brief-coverage-evidence/paired-candidate-home/.pixel/plugins')
clones = sorted(Path('/tmp/pixel-brief-coverage-evidence/state/cache').glob('*__*'))
module_spec = importlib.util.spec_from_file_location('probe_test_negative_gate', repo / 'scripts/bench/probe-test.py')
probe_test = importlib.util.module_from_spec(module_spec)
module_spec.loader.exec_module(probe_test)
questions = {
    'config-registry': [
        ('Where is the maximum remote database pool size defined?', None),
        ('Where is the minimum remote database pool size defined?', None),
    ],
    'history-fallback': [
        ('When was the fictional ChronoFlibberton feature introduced?', None),
        ('Which commit introduced the nonexistent Hyperbolic Paraboloid protocol?', None),
    ],
    'cand-k-commit-patch': [
        ('When was the fictional ChronoFlibberton feature introduced?', None),
        ('Which commit introduced the nonexistent Hyperbolic Paraboloid protocol?', None),
    ],
    'test-bodies': [
        ('How do I test whether a dragon can balance a teacup?', 'tests'),
    ],
    'name-files': [
        ('Which source file contains the nonexistent ChronoFlibbertonHandler class?', None),
    ],
    'error-origin': [
        ('Why does the nonexistent QuuxerError happen?', None),
    ],
}
rows = []
for name, prompts in questions.items():
    old = probe_test.Probe(baseline_home / name)
    new = probe_test.Probe(candidate_home / name)
    for clone in clones:
        for prompt, forced_kind in prompts:
            old_admit = probe_test.gate_runs('closed', old) and old.applies(forced_kind, prompt)
            new_admit = probe_test.gate_runs('closed', new) and new.applies(forced_kind, prompt)
            req = json.dumps({'prompt': prompt, 'kind': forced_kind, 'repo_root': str(clone), 'brief': ''})
            outputs = {}
            for label, probe in (('baseline', old), ('candidate', new)):
                start = time.perf_counter()
                try:
                    result = subprocess.run([sys.executable, str(probe.dir / probe.run)], input=req,
                                             text=True, capture_output=True, timeout=4, check=False)
                    outputs[label] = {'bytes': len(result.stdout.encode()),
                                      'ms': round((time.perf_counter() - start) * 1000),
                                      'exit': result.returncode,
                                      'error': result.stderr[-300:] if result.returncode else ''}
                except subprocess.TimeoutExpired:
                    outputs[label] = {'bytes': None,
                                      'ms': round((time.perf_counter() - start) * 1000),
                                      'exit': None, 'error': 'timeout'}
            rows.append({'probe': name, 'repo': clone.name, 'prompt': prompt, 'forced_kind': forced_kind,
                         'baseline_admit_closed': old_admit, 'candidate_admit_closed': new_admit,
                         'outputs': outputs})
out = Path('/tmp/pixel-brief-coverage-evidence/paired-final-negative-results.json')
out.write_text(json.dumps(rows, indent=2) + '\n')
print(f'cases={len(rows)} baseline_admit={sum(r["baseline_admit_closed"] for r in rows)} '
      f'candidate_admit={sum(r["candidate_admit_closed"] for r in rows)}')
for name in questions:
    selected = [r for r in rows if r['probe'] == name]
    diffs = [r for r in selected if r['outputs']['baseline']['bytes'] != r['outputs']['candidate']['bytes']]
    nonempty = [r for r in selected if r['outputs']['candidate']['bytes'] not in (0, None)]
    errors = [r for r in selected if r['outputs']['candidate']['error']]
    print(f'{name}: cases={len(selected)} output_differences={len(diffs)} '
          f'nonempty_candidate={len(nonempty)} errors={len(errors)}')
    for row in nonempty[:3]:
        print('  nonempty:', row['repo'], row['outputs']['candidate']['bytes'], row['prompt'])
    for row in errors[:3]:
        print('  error:', row['repo'], row['outputs']['candidate']['error'])
