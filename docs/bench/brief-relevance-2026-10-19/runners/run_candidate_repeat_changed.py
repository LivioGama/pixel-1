from __future__ import annotations

import fcntl
import subprocess
import sys
from pathlib import Path

changed = (
    'excalidraw-02,excalidraw-05,excalidraw-06,excalidraw-07,express-02,express-03,express-06,'
    'fd-05,fd-06,fd-07,fd-08,flask-06,pixel-05,pixel-06,pixel-07,pixel-08,requests-05,requests-06,'
    'ripgrep-05,ripgrep-06'
)
lock_path = Path('/tmp/pixel-benchmark.lock')
with lock_path.open('a+') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    command = [
        sys.executable,
        'scripts/bench/tool-census/score_brief.py',
        '--root', '/tmp/pixel-brief-coverage-evidence/paired-candidate-root',
        '--bin', '/tmp/pixel-brief-coverage-evidence/target/dev-release/pixel',
        '--label', 'paired-candidate-repeat-changed-7dc83944',
        '--probe-home', '/tmp/pixel-brief-coverage-evidence/paired-candidate-home',
        '--questions', 'eval/brief-bench/questions.jsonl',
        '--out', '/tmp/pixel-brief-coverage-evidence/paired-candidate-root/score/runs/paired-candidate-repeat-changed',
        '--route', 'hook',
        '--only', changed,
    ]
    print('command:', ' '.join(command), flush=True)
    print('lock: /tmp/pixel-benchmark.lock acquired', flush=True)
    completed = subprocess.run(command, check=False)
    print('exit:', completed.returncode, flush=True)
    raise SystemExit(completed.returncode)
