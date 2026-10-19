from __future__ import annotations

import importlib.util
import hashlib
import json
import subprocess
import sys
import difflib
from pathlib import Path

repo = Path('/Users/livio/Documents/pixel-worktrees/relevant-brief')
candidate_home = Path('/tmp/pixel-brief-coverage-evidence/paired-candidate-home/.pixel/plugins')
baseline_home = Path('/tmp/pixel-brief-coverage-evidence/paired-baseline-home/.pixel/plugins')
clones = {p.name: p for p in Path('/tmp/pixel-brief-coverage-evidence/state/cache').glob('*__*')}
module_spec = importlib.util.spec_from_file_location('probe_test_negative_gate', repo / 'scripts/bench/probe-test.py')
probe_test = importlib.util.module_from_spec(module_spec)
module_spec.loader.exec_module(probe_test)
samples = [
    ('history-fallback', 'BurntSushi__ripgrep', 'When was the fictional ChronoFlibberton feature introduced?', None),
    ('history-fallback', 'BurntSushi__ripgrep', 'Which commit introduced the nonexistent Hyperbolic Paraboloid protocol?', None),
    ('cand-k-commit-patch', 'BurntSushi__ripgrep', 'When was the fictional ChronoFlibberton feature introduced?', None),
    ('error-origin', 'pallets__flask', 'Why does the nonexistent QuuxerError happen?', None),
    ('test-bodies', 'Pixel-CLI__pixel', 'How do I test whether a dragon can balance a teacup?', 'tests'),
]
outdir = Path('/tmp/pixel-brief-coverage-evidence/negative-output-samples')
outdir.mkdir(parents=True, exist_ok=True)
rows = []
for name, repo_name, prompt, kind in samples:
    body = json.dumps({'prompt': prompt, 'kind': kind, 'repo_root': str(clones[repo_name]), 'brief': ''})
    outputs = {}
    texts = {}
    for label, home in (('baseline', baseline_home), ('candidate', candidate_home)):
        probe = probe_test.Probe(home / name)
        result = subprocess.run([sys.executable, str(probe.dir / probe.run)], input=body, text=True,
                                capture_output=True, timeout=6, check=False)
        arm_stem = f'{name}__{repo_name}__{len(rows):02d}__{label}'
        path = outdir / f'{arm_stem}.txt'
        path.write_text(result.stdout)
        texts[label] = result.stdout
        outputs[label] = {
            'gate_closed_admits': bool(probe_test.gate_runs('closed', probe) and probe.applies(kind, prompt)),
            'exit': result.returncode,
            'stdout_bytes': len(result.stdout.encode()),
            'stdout_sha256': hashlib.sha256(result.stdout.encode()).hexdigest(),
            'stderr': result.stderr[-500:] if result.returncode else '',
            'output_path': str(path),
        }
    stem = f'{name}__{repo_name}__{len(rows):02d}'
    diff = ''.join(difflib.unified_diff(texts['baseline'].splitlines(keepends=True),
                                        texts['candidate'].splitlines(keepends=True),
                                        fromfile='baseline', tofile='candidate'))
    diff_path = outdir / f'{stem}.diff'
    diff_path.write_text(diff)
    rows.append({'probe': name, 'repo': repo_name, 'prompt': prompt, 'kind': kind,
                 'byte_identical': texts['baseline'] == texts['candidate'],
                 'outputs': outputs, 'diff_path': str(diff_path)})
(outdir / 'index.json').write_text(json.dumps(rows, indent=2) + '\n')
print(json.dumps(rows, indent=2))
