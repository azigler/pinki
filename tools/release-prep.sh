#!/usr/bin/env bash
# Prepare a pinki release: everything mechanical, nothing irreversible.
#
#   tools/release-prep.sh [--dry-run] <version>        e.g. tools/release-prep.sh 0.7.0
#
# What it does, in order:
#   1. asserts you are on main with a clean tree, fast-forwards to origin/main, and that
#      the tag v<version> does not exist yet
#   2. bumps Cargo.toml (Cargo.lock follows the build) and restructures CHANGELOG.md:
#      [Unreleased] becomes [<version>] - <date>, a new empty [Unreleased] goes above it,
#      and the footer compare links move
#   3. runs the gates: build --release, fmt --check, clippy -D warnings, test,
#      README examples, llvm-cov (summary + the 97% line gate), then
#      tools/uncovered-lines.py (re-derives the block at the end of tests/cli.rs),
#      then fmt --check again
#   4. STOPS, and prints the steps only a person does: write the intro paragraph and the
#      released-tree test line in the new CHANGELOG section, commit, tag, push.
#
# It never commits, tags, or pushes. release.yml builds the tarball when the tag lands.
#
# --dry-run runs every check and every gate against the tree as it is, writes no file
# in the working tree (the gates still write target/ and git fetch updates refs), keeps
# going past a failure, and exits 1 with the list of what failed.
#
# Environment:
#   PINKI_RUN_PREFIX   a command to put in front of each heavy cargo invocation, for
#                      machines that meter builds (e.g. "nice -n 19"). Default: none.
#   RELEASE_DATE       the date for the CHANGELOG heading (default: today, YYYY-MM-DD).
set -euo pipefail

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//' >&2
  exit 2
}

dry_run=0
version=""
for arg in "$@"; do
  case $arg in
    --dry-run) dry_run=1 ;;
    -h | --help) usage ;;
    -*) printf 'unknown option: %s\n' "$arg" >&2; usage ;;
    *)
      [[ -z $version ]] || { printf 'more than one version given\n' >&2; usage; }
      version=$arg
      ;;
  esac
done
[[ -n $version ]] || usage
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  printf 'not a semantic version: %s\n' "$version" >&2
  exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"
date_stamp=${RELEASE_DATE:-$(date +%F)}
tag="v$version"

# The word-splitting of the prefix is the point: it is a command and its arguments.
read -r -a run_prefix <<<"${PINKI_RUN_PREFIX:-}"

failed=()

# fail MESSAGE: stop at once, or (dry run) record it and carry on.
fail() {
  if ((dry_run)); then
    failed+=("$1")
    printf 'FAIL (dry run, continuing): %s\n' "$1" >&2
  else
    printf 'FAIL: %s\n' "$1" >&2
    exit 1
  fi
}

# step NAME CMD...: announce, run, and route a nonzero exit through fail.
step() {
  local name=$1
  shift
  printf '\n== %s\n' "$name"
  if ! "$@"; then
    fail "$name"
  fi
}

heavy() { "${run_prefix[@]+"${run_prefix[@]}"}" "$@"; }

# --- 1. preconditions -------------------------------------------------------------

check_branch() {
  local branch
  branch=$(git rev-parse --abbrev-ref HEAD)
  [[ $branch == main ]] || { printf 'on %s, not main\n' "$branch" >&2; return 1; }
}

check_clean() {
  local dirty
  dirty=$(git status --porcelain)
  [[ -z $dirty ]] || { printf 'working tree is not clean:\n%s\n' "$dirty" >&2; return 1; }
}

sync_main() {
  git fetch origin main
  if git merge-base --is-ancestor origin/main HEAD; then
    local ahead
    ahead=$(git rev-list --count origin/main..HEAD)
    ((ahead == 0)) || printf 'note: main is %s commit(s) ahead of origin/main\n' "$ahead"
    return 0
  fi
  if ((dry_run)); then
    printf 'main is behind origin/main; a real run would fast-forward it\n' >&2
    return 1
  fi
  git merge --ff-only origin/main
}

check_tag_free() {
  if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    printf 'tag %s already exists locally\n' "$tag" >&2
    return 1
  fi
  local remote
  remote=$(git ls-remote --tags origin "refs/tags/$tag")
  [[ -z $remote ]] || { printf 'tag %s already exists on origin\n' "$tag" >&2; return 1; }
}

step 'on main' check_branch
step 'working tree clean' check_clean
step 'sync with origin/main' sync_main
step "tag $tag is free" check_tag_free

# --- 2. version bump and CHANGELOG ------------------------------------------------

# edit_release_files [--write]: bump Cargo.toml and restructure CHANGELOG.md. Without
# --write, check that every anchor is present and change nothing.
edit_release_files() {
  python3 - "$version" "$date_stamp" "$@" <<'PY'
import pathlib
import re
import sys

version, date = sys.argv[1], sys.argv[2]
write = "--write" in sys.argv[3:]

cargo = pathlib.Path("Cargo.toml")
text = cargo.read_text()
text, n = re.subn(r'(?m)^version = "[^"]*"$', f'version = "{version}"', text, count=1)
if n != 1:
    sys.exit('Cargo.toml: no `version = "..."` line to bump')
cargo_new = text

log = pathlib.Path("CHANGELOG.md")
text = log.read_text()
if f"## [{version}]" in text:
    sys.exit(f"CHANGELOG.md already has a [{version}] section")
if text.count("\n## [Unreleased]\n") != 1:
    sys.exit("CHANGELOG.md: expected exactly one `## [Unreleased]` heading")
text = text.replace(
    "\n## [Unreleased]\n", f"\n## [Unreleased]\n\n## [{version}] - {date}\n", 1
)
link = re.search(
    r"(?m)^\[Unreleased\]: (https://\S+)/compare/v(\S+?)\.\.\.HEAD$", text
)
if not link:
    sys.exit("CHANGELOG.md: no `[Unreleased]: .../compare/vX...HEAD` footer link")
base, prev = link.group(1), link.group(2)
if prev == version:
    sys.exit(f"CHANGELOG.md: the footer already compares from v{version}")
text = text.replace(
    link.group(0),
    f"[Unreleased]: {base}/compare/v{version}...HEAD\n"
    f"[{version}]: {base}/compare/v{prev}...v{version}",
    1,
)

if write:
    cargo.write_text(cargo_new)
    log.write_text(text)
    print(f"Cargo.toml -> {version}; CHANGELOG [Unreleased] -> [{version}] - {date}")
else:
    print(f"would bump Cargo.toml to {version} and cut [{version}] - {date} from v{prev}")
PY
}

if ((dry_run)); then
  step 'version bump + CHANGELOG (checked, not written)' edit_release_files
else
  step 'version bump + CHANGELOG' edit_release_files --write
fi

# --- 3. gates ---------------------------------------------------------------------

# In a real run the build is not --locked: it is what carries the bump into Cargo.lock.
if ((dry_run)); then
  step 'cargo build --release --locked' heavy cargo build --release --locked
else
  step 'cargo build --release' heavy cargo build --release
  check_lock() { grep -A1 '^name = "pinki"$' Cargo.lock | grep -qx "version = \"$version\""; }
  step 'Cargo.lock carries the new version' check_lock
fi
step 'cargo fmt --check' cargo fmt --check
step 'cargo clippy' heavy cargo clippy --all-targets -- -D warnings
step 'cargo test' heavy cargo test
step 'README examples' bash scripts/check-readme.sh
step 'cargo llvm-cov --summary-only' heavy cargo llvm-cov --summary-only
step 'coverage gate (>= 97% lines)' heavy cargo llvm-cov report --fail-under-lines 97
if ((dry_run)); then
  step 'tools/uncovered-lines.py --check' python3 tools/uncovered-lines.py --check
else
  step 'tools/uncovered-lines.py' python3 tools/uncovered-lines.py
fi
step 'cargo fmt --check (again)' cargo fmt --check

# --- 4. stop ----------------------------------------------------------------------

if ((${#failed[@]})); then
  printf '\nDRY RUN: %d check(s) failed:\n' "${#failed[@]}" >&2
  printf '  - %s\n' "${failed[@]}" >&2
  exit 1
fi

if ((dry_run)); then
  printf '\nDRY RUN: every check and gate passed for %s. Nothing was written.\n' "$tag"
  exit 0
fi

cat <<EOF

Release prep for $tag is done. Nothing is committed, tagged or pushed. Left for you:

  1. Edit CHANGELOG.md: write the intro paragraph under "## [$version] - $date_stamp" and
     the released-tree "Testing" line (tests, coverage; see earlier releases).
  2. Review:      git diff
  3. Commit:      git add Cargo.toml Cargo.lock CHANGELOG.md tests/cli.rs
                  git commit -m ':bookmark: v$version'
  4. Tag:         git tag -a $tag -m 'pinki $version'
  5. Push:        git push origin main $tag
     The tag push runs release.yml, which builds and publishes the tarball.
EOF
