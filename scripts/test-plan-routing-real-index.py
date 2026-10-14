#!/usr/bin/env python3
# SPDX-FileCopyrightText: The Pixel contributors
# SPDX-License-Identifier: MIT

"""Contract tests for plan-routing-real index wiring."""
import importlib.util
from pathlib import Path
import json
import unittest

spec = importlib.util.spec_from_file_location('pilot', Path(__file__).with_name('bench-plan-routing.py'))
pilot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pilot)


class RealRoutingIndexContract(unittest.TestCase):
    def test_index_exists_for_all_requests(self):
        requests = json.loads((Path(__file__).parent / 'fixtures' / 'plan-routing-real-requests.json').read_text())
        index = json.loads((Path(__file__).parent / 'fixtures' / 'plan-routing-real-index.json').read_text())
        request_ids = {r['id'] for r in requests['requests']}
        self.assertEqual(set(index.keys()), request_ids)

    def test_index_labels_are_valid(self):
        index = json.loads((Path(__file__).parent / 'fixtures' / 'plan-routing-real-index.json').read_text())
        valid_labels = set(pilot.LABELS) | {'by-concept'}
        for req_id, labels in index.items():
            self.assertIsInstance(labels, list)
            for label in labels:
                self.assertIn(label, valid_labels, f"Invalid label {label} for {req_id}")

    def test_baseline_matches_index_labels(self):
        requests = json.loads((Path(__file__).parent / 'fixtures' / 'plan-routing-real-requests.json').read_text())
        index = json.loads((Path(__file__).parent / 'fixtures' / 'plan-routing-real-index.json').read_text())
        for req in requests['requests']:
            expected = pilot.baseline(req['text'])
            actual = index[req['id']]
            self.assertEqual(expected, actual, f"Label mismatch for {req['id']}")


if __name__ == '__main__':
    unittest.main()
