#!/usr/bin/env python3
"""Re-derive the "what stays uncovered" comment block at the end of tests/cli.rs.

The block lists the source lines that no test executes, with the reason each one is
deliberate, and says how many "Missed Lines" the coverage summary reports against that
list ("N against K"). Both go stale whenever code moves. This script re-reads them from
`cargo llvm-cov report --show-missing-lines` instead of asserting remembered figures.

Usage:
    tools/uncovered-lines.py [--check] [--report FILE] [REPO]

    REPO            the pinki checkout (default: the parent of this script's directory)
    --check         write nothing; report whether the block is current
    --report FILE   read a saved `cargo llvm-cov report --show-missing-lines` output
                    instead of running cargo (for tests and saved reports from this checkout)

The report needs coverage data, so run `cargo llvm-cov --summary-only` first.

The uncovered SET must be exactly the known lines, identified by file and by the
shape of the source text (not by line number, which moves). Any other uncovered line, or
any known line that is now covered, means the set changed: the script stops, names the
lines, and leaves the file alone. A human then decides whether the new gap is deliberate
(add it to KNOWN here and to the block) or a missing test.

Exit status:
    0  the set is known, and the block is current (or was just rewritten)
    1  --check only: the set is known, but the block is stale
    2  the uncovered set changed
    3  could not evaluate (no coverage data, unparseable report, comment shape changed)
"""

import argparse
import pathlib
import re
import subprocess
import sys

# One entry per documented gap. `lines` are regexes on the STRIPPED source text of
# consecutive lines; `prev` optionally constrains the line before the first. `count` is
# how many separate occurrences the set must contain, in file order.
KNOWN = [
    {
        "key": "state_walk_missing",
        "file": "src/state.rs",
        "prev": r"\}",
        "lines": [r"\}"],
        "count": 1,
        "why": "Every graph walk id comes from a fold view.",
    },
    {
        "key": "state_conditional",
        "file": "src/state.rs",
        "lines": [r"_ => State::Conditional,"],
        "count": 1,
    },
    {
        "key": "state_dangling",
        "file": "src/state.rs",
        "lines": [
            r"self\.memo\.remove\(current\);",
            r"stack\.pop\(\);",
            r"continue;",
        ],
        "count": 1,
    },
    {
        "key": "state_detached",
        "file": "src/state.rs",
        "lines": [r"None => State::Detached,"],
        "count": 1,
    },
    {
        "key": "verbs_none",
        "file": "src/verbs.rs",
        "prev": r"EventBody::Assess \{ observer, \.\. \} => Some\(observer\.clone\(\)\),",
        "lines": [r"_ => None,"],
        "count": 1,
    },
    {
        "key": "verbs_terminal",
        "file": "src/verbs.rs",
        "prev": r"if io::stdin\(\)\.is_terminal\(\) \{",
        "lines": [r"return Err\(Fail::Usage\("],
        "count": 1,
    },
    {
        "key": "verbs_correct_clap",
        "file": "src/verbs.rs",
        "lines": [r'_ => unreachable!\("clap validates --as"\),'],
        "count": 1,
        "why": "Clap accepts only the five correction outcomes before correct() runs.",
    },
    {
        "key": "verbs_typed_fallbacks",
        "file": "src/verbs.rs",
        "lines": [r"_ => None,"],
        "count": 4,
        "why": "Fold event fields contain only their matching body variants.",
    },
    {
        "key": "export_typed_fallbacks",
        "file": "src/export.rs",
        "lines": [r"_ => None,"],
        "count": 2,
        "why": "Fold collections contain only Assess and Correct event bodies.",
    },
    {
        "key": "event_panic",
        "file": "src/event.rs",
        "lines": [r'panic!\("failed to read back \{text\}: \{e\}"\);'],
        "count": 2,
    },
    {
        "key": "ledger_panic",
        "file": "src/ledger.rs",
        "lines": [r'other => panic!\("expected Malformed'],
        "count": 2,
    },
]

# (pattern, key, builder). Group 1 is kept, the number(s) are replaced, group 2 is kept.
# `found[key]` is the list of start-line tuples (first..last line of each occurrence).
COMMENT_EDITS = [
    (r"(src/state\.rs )\d+(  graph walk)", "state_walk_missing", 0, "single"),
    (
        r"(src/state\.rs )\d+(\s+`_ => State::Conditional`)",
        "state_conditional",
        0,
        "single",
    ),
    (
        r"(src/state\.rs )\d+-\d+(\s+the dangling arm)",
        "state_dangling",
        0,
        "range",
    ),
    (
        r"(src/state\.rs )\d+(\s+`None => State::Detached`)",
        "state_detached",
        0,
        "single",
    ),
    (r"(src/verbs\.rs )\d+(\s+`_ => None` over)", "verbs_none", 0, "single"),
    (
        r"(src/verbs\.rs )\d+(\s+the `io::stdin\(\)\.is_terminal\(\)` refusal)",
        "verbs_terminal",
        0,
        "single",
    ),
    (
        r"(src/event\.rs )\d+(\s+`panic!` in `every_event_type_round_trips`)",
        "event_panic",
        0,
        "single",
    ),
    (
        r"(src/event\.rs )\d+(\s+`panic!` in `meta_survives)",
        "event_panic",
        1,
        "single",
    ),
    (
        r"(src/ledger\.rs )\d+,\d+( `other => panic!)",
        "ledger_panic",
        None,
        "pair",
    ),
    (r"(src/verbs\.rs )\d+(  correction `--as`)", "verbs_correct_clap", 0, "single"),
    (
        r"(src/verbs\.rs )\d+(?:,\d+){3}(  typed event fallbacks)",
        "verbs_typed_fallbacks",
        None,
        "list",
    ),
    (
        r"(src/export\.rs )\d+,\d+(  typed event fallbacks)",
        "export_typed_fallbacks",
        None,
        "list",
    ),
]


class CouldNotEvaluate(Exception):
    pass


def run_report(repo: pathlib.Path) -> str:
    proc = subprocess.run(
        ["cargo", "llvm-cov", "report", "--show-missing-lines"],
        cwd=repo,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise CouldNotEvaluate(
            "cargo llvm-cov report failed (run `cargo llvm-cov --summary-only` first):\n"
            + proc.stderr.strip()
        )
    return proc.stdout


def parse_report(text: str, repo: pathlib.Path):
    """Return (missed_lines_total, {relative file: sorted uncovered line numbers})."""
    total = re.search(
        r"^TOTAL\s+\d+\s+\d+\s+[\d.]+%\s+\d+\s+\d+\s+[\d.]+%\s+\d+\s+(\d+)",
        text,
        re.M,
    )
    if not total:
        raise CouldNotEvaluate(
            "no TOTAL row in the report: the summary table is missing"
        )
    if "Uncovered Lines:" not in text:
        raise CouldNotEvaluate("no 'Uncovered Lines:' section in the report")
    miss = {}
    for line in text.split("Uncovered Lines:", 1)[1].splitlines():
        m = re.match(r"^\s*(\S+\.rs):\s*([\d,\s]+)$", line)
        if not m:
            continue
        path = pathlib.Path(m.group(1))
        if path.is_absolute():
            try:
                path = path.relative_to(repo)
            except ValueError:
                continue
        if not path.parts or path.parts[0] != "src":
            continue
        miss[path.as_posix()] = sorted(
            int(x) for x in re.findall(r"\d+", m.group(2))
        )
    if not miss:
        raise CouldNotEvaluate(
            "the 'Uncovered Lines:' section parsed to zero files"
        )
    return int(total.group(1)), miss


def classify(repo: pathlib.Path, miss):
    """Match uncovered lines to KNOWN shapes. Return (found, extras, problems)."""
    found = {}
    claimed = set()
    problems = []
    for entry in KNOWN:
        path = repo / entry["file"]
        if not path.is_file():
            raise CouldNotEvaluate(f"cannot read {path}")
        src = path.read_text().splitlines()
        uncovered = set(miss.get(entry["file"], []))
        hits = []
        for start in sorted(uncovered):
            span = range(start, start + len(entry["lines"]))
            if any(n not in uncovered or (entry["file"], n) in claimed for n in span):
                continue
            if any(
                n > len(src)
                or not re.match(pat, src[n - 1].strip())
                for n, pat in zip(span, entry["lines"], strict=True)
            ):
                continue
            prev = entry.get("prev")
            if prev and not (
                start >= 2 and re.search(prev, src[start - 2].strip())
            ):
                continue
            hits.append(tuple(span))
            claimed.update((entry["file"], n) for n in span)
        found[entry["key"]] = hits
        if len(hits) != entry["count"]:
            problems.append(
                f"{entry['file']}: expected {entry['count']} x {entry['key']}, "
                f"found {len(hits)} (shape {entry['lines']})"
            )
    extras = []
    for f, lines in sorted(miss.items()):
        src = (
            (repo / f).read_text().splitlines() if (repo / f).is_file() else []
        )
        for n in lines:
            if (f, n) not in {
                (e["file"], c)
                for e in KNOWN
                for h in found[e["key"]]
                for c in h
            }:
                text = src[n - 1].strip() if 0 < n <= len(src) else "?"
                extras.append((f, n, text))
    return found, extras, problems


def known_line_count() -> int:
    return sum(e["count"] * len(e["lines"]) for e in KNOWN)


def rewrite(text: str, found, total: int) -> str:
    for pattern, key, idx, kind in COMMENT_EDITS:
        hits = found[key]
        if kind in ("pair", "list"):
            repl = rf"\g<1>{','.join(str(hit[0]) for hit in hits)}\g<2>"
        else:
            span = hits[idx]
            num = f"{span[0]}-{span[-1]}" if kind == "range" else f"{span[0]}"
            repl = rf"\g<1>{num}\g<2>"
        text, n = re.subn(pattern, repl, text)
        if n != 1:
            raise CouldNotEvaluate(
                f"comment shape changed: {pattern!r} matched {n} times in tests/cli.rs, "
                "expected once"
            )
    known_total = known_line_count()
    text, n = re.subn(
        r"\(\d+ against \d+\)", f"({total} against {known_total})", text
    )
    if n != 1:
        raise CouldNotEvaluate(
            f"comment shape changed: the 'N against {known_total}' count matched {n} times"
        )
    return text


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawTextHelpFormatter
    )
    ap.add_argument(
        "repo", nargs="?", help="pinki checkout (default: parent of tools/)"
    )
    ap.add_argument(
        "--check",
        action="store_true",
        help="write nothing; report whether current",
    )
    ap.add_argument(
        "--report",
        help="saved `cargo llvm-cov report --show-missing-lines` output",
    )
    args = ap.parse_args()

    repo = pathlib.Path(
        args.repo or pathlib.Path(__file__).resolve().parent.parent
    )
    repo = repo.resolve()
    try:
        report = (
            pathlib.Path(args.report).read_text()
            if args.report
            else run_report(repo)
        )
        total, miss = parse_report(report, repo)
        found, extras, problems = classify(repo, miss)

        derived = sorted(
            (e["file"], span[0], span[-1])
            for e in KNOWN
            for span in found[e["key"]]
        )
        print(f"Missed Lines in the summary: {total}")
        print(
            f"Known lines matched ({sum(len(sp) for v in found.values() for sp in v)} of "
            f"{known_line_count()} expected):"
        )
        for f, a, b in derived:
            print(f"  {f} {a}" if a == b else f"  {f} {a}-{b}")

        if extras or problems:
            print(
                f"\nFAIL: the uncovered set is not the {known_line_count()} known lines.",
                file=sys.stderr,
            )
            for p in problems:
                print(f"  {p}", file=sys.stderr)
            if extras:
                print(
                    f"  {len(extras)} uncovered line(s) outside the known set:",
                    file=sys.stderr,
                )
                for f, n, text in extras:
                    print(f"    {f}:{n}  {text}", file=sys.stderr)
            print(
                "Decide per line: a deliberate gap goes in KNOWN (tools/uncovered-lines.py)\n"
                "and in the block at the end of tests/cli.rs; anything else needs a test.\n"
                "The file was not changed.",
                file=sys.stderr,
            )
            return 2

        cli = repo / "tests" / "cli.rs"
        old = cli.read_text()
        new = rewrite(old, found, total)
        if new == old:
            print("comment block is current")
            return 0
        if args.check:
            print(
                f"comment block is STALE (set is the {known_line_count()} known lines; numbers moved)"
            )
            return 1
        cli.write_text(new)
        print("comment block re-derived")
        return 0
    except (CouldNotEvaluate, OSError) as e:
        print(f"COULD NOT EVALUATE: {e}", file=sys.stderr)
        return 3
    except Exception as e:
        print(f"COULD NOT EVALUATE: unexpected {type(e).__name__}: {e}", file=sys.stderr)
        return 3


if __name__ == "__main__":
    sys.exit(main())
