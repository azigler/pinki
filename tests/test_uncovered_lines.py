"""Regression tests for the coverage report parser and failure states."""

import contextlib
import importlib.util
import io
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "uncovered_lines", ROOT / "tools" / "uncovered-lines.py"
)
uncovered = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(uncovered)


class UncoveredLinesTests(unittest.TestCase):
    def test_report_path_under_src_checkout_is_repo_relative(self):
        report = (
            "TOTAL 1 0 0.00% 0 0 0.00% 0 1\n"
            "Uncovered Lines:\n"
            "/home/person/src/pinki/src/state.rs: 12\n"
        )
        _, missing = uncovered.parse_report(report, pathlib.Path("/home/person/src/pinki"))
        self.assertEqual(missing, {"src/state.rs": [12]})

    def test_same_line_number_in_different_files_can_be_claimed(self):
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            repo = pathlib.Path(temporary)
            (repo / "src").mkdir()
            (repo / "src" / "a.rs").write_text("first\n")
            (repo / "src" / "b.rs").write_text("second\n")
            known = [
                {"key": "a", "file": "src/a.rs", "lines": ["first"], "count": 1},
                {"key": "b", "file": "src/b.rs", "lines": ["second"], "count": 1},
            ]
            with mock.patch.object(uncovered, "KNOWN", known):
                found, extras, problems = uncovered.classify(
                    repo, {"src/a.rs": [1], "src/b.rs": [1]}
                )
            self.assertEqual(found, {"a": [(1,)], "b": [(1,)]})
            self.assertEqual((extras, problems), ([], []))

    def test_missing_cargo_is_could_not_evaluate(self):
        with mock.patch.object(sys, "argv", ["uncovered-lines.py"]), mock.patch.object(
            uncovered.subprocess, "run", side_effect=FileNotFoundError("cargo missing")
        ), contextlib.redirect_stderr(io.StringIO()) as error:
            result = uncovered.main()
        self.assertEqual(result, 3)
        self.assertIn("COULD NOT EVALUATE", error.getvalue())

    def test_missing_report_file_is_could_not_evaluate(self):
        with mock.patch.object(
            sys, "argv", ["uncovered-lines.py", "--report", str(ROOT / "target" / "absent-cov-report")]
        ), contextlib.redirect_stderr(io.StringIO()) as error:
            result = uncovered.main()
        self.assertEqual(result, 3)
        self.assertIn("COULD NOT EVALUATE", error.getvalue())

    def test_invalid_utf8_report_is_could_not_evaluate(self):
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            report = pathlib.Path(temporary) / "invalid-report"
            report.write_bytes(b"\xff")
            with mock.patch.object(
                sys, "argv", ["uncovered-lines.py", "--report", str(report)]
            ), contextlib.redirect_stderr(io.StringIO()) as error:
                result = uncovered.main()
        self.assertEqual(result, 3)
        self.assertEqual(
            error.getvalue().count("COULD NOT EVALUATE: unexpected UnicodeDecodeError:"),
            1,
        )

    def test_import_failure_is_could_not_evaluate(self):
        # A module that cannot be imported must exit 3, not the traceback's 1.
        code = (
            "import runpy, sys; sys.modules['argparse'] = None; "
            f"runpy.run_path({str(ROOT / 'tools' / 'uncovered-lines.py')!r}, run_name='x')"
        )
        result = subprocess.run(
            [sys.executable, "-B", "-c", code], capture_output=True, text=True
        )
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertIn("COULD NOT EVALUATE", result.stderr)

    def test_state_walk_shape_matches_one_line_of_state_rs(self):
        entry = next(e for e in uncovered.KNOWN if e["key"] == "state_walk_missing")
        src = (ROOT / entry["file"]).read_text().splitlines()
        every_line = {"src/state.rs": list(range(1, len(src) + 1))}
        with mock.patch.object(uncovered, "KNOWN", [entry]):
            found, _, _ = uncovered.classify(ROOT, every_line)
        hits = found["state_walk_missing"]
        self.assertEqual(len(hits), 1)
        # Pin WHICH line, not only how many: a `}` whose two lines above are a `}` and the
        # `stack.extend` of children. An off-by-one in the matcher fits another line.
        (line,) = hits[0]
        self.assertEqual(src[line - 1].strip(), "}")
        self.assertEqual(src[line - 2].strip(), "}")
        self.assertIn("stack.extend(children", src[line - 3])


if __name__ == "__main__":
    unittest.main()
