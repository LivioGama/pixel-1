#!/usr/bin/env python3
# SPDX-FileCopyrightText: The RTO contributors
# SPDX-License-Identifier: MIT
"""The preserved Pixel site keeps its attribution without relaxing other files."""
import contextlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "check_spdx", Path(__file__).parent / "ci" / "check-spdx.py"
)
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)
PIXEL = "# SPDX-FileCopyrightText: The Pixel contributors\n"
LICENSE = "# SPDX-License-Identifier: MIT\n"


class PreservedAttribution(unittest.TestCase):
    def check_file(self, path, body):
        with tempfile.TemporaryDirectory() as tmp:
            file = Path(tmp) / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(body)
            with patch.object(CHECK, "tracked", return_value=[path]):
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    return CHECK.main([tmp])

    def test_preserved_site_files_accept_their_pixel_attribution(self):
        for path in (
            ".github/workflows/pages.yml",
            "website/og/render.sh",
            "docs/motion/prompt-submit/render-recording.ts",
        ):
            with self.subTest(path=path):
                self.assertEqual(self.check_file(path, PIXEL + LICENSE), 0)

    def test_other_files_still_require_rto_attribution(self):
        self.assertEqual(self.check_file("scripts/other.py", PIXEL + LICENSE), 1)

    def test_preserved_files_still_require_the_license(self):
        self.assertEqual(self.check_file("website/og/render.sh", PIXEL), 1)

    def test_preserved_files_reject_unknown_attribution(self):
        self.assertEqual(self.check_file("website/og/render.sh", "# SPDX-FileCopyrightText: Unknown\n" + LICENSE), 1)


if __name__ == "__main__":
    unittest.main()
