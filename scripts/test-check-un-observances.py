#!/usr/bin/env python3
"""Offline regressions for capture, validation, drift and exact serialization."""

from contextlib import redirect_stdout, redirect_stderr
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("observances", ROOT / "scripts/check-un-observances.py")
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


def no_socket(*args, **kwargs):
    raise AssertionError("offline tests must not open a socket")


CHECK.urlopen = no_socket
FIXTURE = ROOT / "tests/fixtures/un-observances-page.html"
CALENDAR = ROOT / "assets/un-observances.json"


class CalendarTests(unittest.TestCase):
    def run_check(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            status = CHECK.main(["--page", str(FIXTURE), *map(str, args)])
        return status, out.getvalue(), err.getvalue()

    def test_fixture_is_the_committed_calendar(self):
        entries = CHECK.parse_page(FIXTURE.read_text())
        self.assertEqual(entries, json.loads(CALENDAR.read_text())["entries"])
        self.assertGreaterEqual(len(entries), 200)
        self.assertGreaterEqual(len({(e["month"], e["day"]) for e in entries}), 180)
        self.assertEqual(list(dict.fromkeys(e["month"] for e in entries)), list(range(1, 13)))
        def day(month, number):
            return [e for e in entries if (e["month"], e["day"]) == (month, number)]
        self.assertEqual(day(2, 2), [dict(month=2, day=2, name="World Wetlands Day", resolutions="A/RES/75/317")])
        self.assertTrue(day(2, 1)[0]["name"].startswith("World Interfaith Harmony Week"))
        self.assertIn("February", day(2, 1)[0]["name"])
        self.assertGreaterEqual(len(day(3, 21)), 2)
        self.assertFalse(day(10, 26))
        self.assertFalse(day(11, 1))
        self.assertTrue(any(e["resolutions"] == "" for e in entries))
        self.assertTrue(any(e["resolutions"] == "A/RES/2142 (XXI)" for e in entries))
        self.assertTrue(any(e["resolutions"] == "30 C/Resolution 29" for e in entries))
        status, out, err = self.run_check()
        self.assertEqual(status, 0, err)
        self.assertEqual(out, f"no drift ({len(entries)} entries on {len({(e['month'], e['day']) for e in entries})} dates)\n")
        completed = subprocess.run([sys.executable, str(ROOT / "scripts/check-un-observances.py"), "--page", str(FIXTURE)], text=True, capture_output=True, check=False)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, out)

    def test_drift_and_order(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "calendar.json"
            data = json.loads(CALENDAR.read_text())
            data["entries"][0]["name"] = "Changed name"
            del data["entries"][1]
            path.write_text(json.dumps(data))
            status, out, err = self.run_check("--file", path)
            self.assertEqual(status, 1, err)
            self.assertEqual(sum(s.startswith("- ") for s in out.splitlines()), 1)
            self.assertEqual(sum(s.startswith("+ ") for s in out.splitlines()), 2)
            self.assertIn("- 1/4 Changed name (A/RES/73/161)", out)
            self.assertIn("drift: 1 removed, 2 added", out)
            data = json.loads(CALENDAR.read_text())
            data["entries"][0], data["entries"][1] = data["entries"][1], data["entries"][0]
            path.write_text(json.dumps(data))
            status, out, err = self.run_check("--file", path)
            self.assertEqual(status, 1, err)
            self.assertEqual(out, "order changed\ndrift: 0 removed, 0 added\n")
            # Repeated rows are retained and counted individually.
            data["entries"].append(data["entries"][0])
            path.write_text(json.dumps(data))
            status, out, err = self.run_check("--file", path)
            self.assertEqual(status, 1, err)
            self.assertIn("drift: 1 removed, 0 added", out)

    def test_write_is_exact(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "calendar.json"
            status, _, err = self.run_check("--write", "--file", path)
            self.assertEqual(status, 0, err)
            self.assertEqual(path.read_bytes(), CALENDAR.read_bytes())

    def test_parse_failure_never_writes(self):
        html = FIXTURE.read_text()
        cases = [
            html.replace("<h3>February</h3>", ""),
            html.replace('content="2021-01-04', 'content="2021-02-04', 1),
            html.replace("04 Jan", "05 Jan", 1),
            html.replace("<h3>January</h3>", ""),
            html.replace("<h3>February</h3>", "<h3>March</h3>"),
            html[:html.index("<h3>December</h3>")],
        ]
        with tempfile.TemporaryDirectory() as directory:
            page = Path(directory) / "bad.html"
            target = Path(directory) / "calendar.json"
            target.write_text("keep this")
            for bad in cases:
                page.write_text(bad)
                status, out, err = self.run_check("--page", page, "--file", target, "--write")
                self.assertEqual(status, 2, (out, err))
                self.assertTrue(err)
                self.assertEqual(target.read_text(), "keep this")

    def test_markup_is_data_and_chrome_is_ignored(self):
        html = FIXTURE.read_text()
        expected = CHECK.parse_page(html)
        changed = html.replace("World Braille Day", "World <b>Braille</b> &amp; Day", 1)
        result = CHECK.parse_page('<h3>Chrome</h3><div class="views-row">ignore</div>' + changed + '<h3>Main Bodies</h3><div class="views-row">ignore</div>')
        self.assertEqual(result[0]["name"], "World Braille & Day")
        self.assertEqual(result[1:], expected[1:])

    def test_workflows_are_read_only_and_run_offline_suite(self):
        workflow = (ROOT / ".github/workflows/un-observances-drift.yml").read_text()
        for value in ["schedule:", 'cron: "17 6 * * 1"', "workflow_dispatch:", "contents: read", "python3 scripts/check-un-observances.py", "actions/checkout@11d5960a"]:
            self.assertIn(value, workflow)
        self.assertNotIn("--write", workflow)
        self.assertNotIn("secrets.", workflow)
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("scripts/check-un-observances.py", ci)
        self.assertIn("python3 scripts/test-check-un-observances.py", ci)


if __name__ == "__main__":
    unittest.main()
