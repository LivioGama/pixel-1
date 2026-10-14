#!/usr/bin/env python3
# SPDX-FileCopyrightText: The Pixel contributors
# SPDX-License-Identifier: MIT

"""Generate routing index from plan-routing-real-requests.json."""
import json
import re
from pathlib import Path


def baseline(text):
    """Name-set transcription of plan.rs; tag variants intentionally collapse."""
    words = re.findall(r'[^\W_]+', text.lower())

    def has(word):
        return any(w == word or (w.endswith('s') and w[:-1] == word) for w in words)

    out = set()
    if any(has(w) for w in ['clickable', 'interactive', 'button', 'link', 'navigation', 'click']):
        out.add('dead-interactive')
    if ' dead code ' in ' ' + ' '.join(words) + ' ' or has('unused') or has('remove'):
        out.add('dead-code')
    if any(has(w) for w in ['refactor', 'hotspot', 'priority']):
        out.add('hotspots')
    if any(has(w) for w in ['recent', 'bug', 'regression']):
        out.add('recent-changes')

    if not out:
        out.add('by-concept')
    return sorted(out)


def main():
    requests_path = Path(__file__).parent / 'fixtures' / 'plan-routing-real-requests.json'
    index_path = Path(__file__).parent / 'fixtures' / 'plan-routing-real-index.json'

    data = json.loads(requests_path.read_text())
    index = {}

    for req in data['requests']:
        text = req['text']
        labels = baseline(text)
        index[req['id']] = labels

    with index_path.open('w') as f:
        json.dump(index, f, indent=2)
        f.write('\n')

    print(f'Generated index for {len(index)} requests')


if __name__ == '__main__':
    main()
