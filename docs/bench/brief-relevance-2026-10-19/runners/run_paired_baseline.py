from __future__ import annotations

import fcntl
import subprocess
import sys
from pathlib import Path

lock_path = Path('/tmp/pixel-benchmark.lock')
with lock_path.open('a+') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    command = [
        sys.executable,
        'scripts/bench/tool-census/score_brief.py',
        '--root', '/tmp/pixel-brief-coverage-evidence/paired-baseline-root',
        '--bin', '/tmp/pixel-brief-coverage-evidence/target/dev-release/pixel',
        '--label', 'paired-baseline-1d1c063d',
        '--probe-home', '/tmp/pixel-brief-coverage-evidence/paired-baseline-home',
        '--questions', 'eval/brief-bench/questions.jsonl',
        '--out', '/tmp/pixel-brief-coverage-evidence/paired-baseline-root/score/runs/paired-baseline',
        '--route', 'hook',
    ]
    print('command:', ' '.join(command), flush=True)
    print('lock: /tmp/pixel-benchmark.lock acquired', flush=True)
    completed = subprocess.run(command, check=False)
    print('exit:', completed.returncode, flush=True)
    raise SystemExit(completed.returncode)
