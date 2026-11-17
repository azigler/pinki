#!/usr/bin/env bash
# Prepare a pinki release: everything mechanical, nothing irreversible.
#
#   tools/release-prep.sh [--dry-run] <version> <one-line release summary>
#       e.g. tools/release-prep.sh 0.7.0 'Add record_ref to the CLI'
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
# --dry-run runs every check and every gate against the tree as it is, leaves tracked
# files alone (the gates still write target/ and git fetch updates .git/FETCH_HEAD
# and refs), keeps
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
release_line=""
for arg in "$@"; do
  case $arg in
    --dry-run) dry_run=1 ;;
    -h | --help) usage ;;
    -*) printf 'unknown option: %s\n' "$arg" >&2; usage ;;
    *)
      if [[ -z $version ]]; then
        version=$arg
      elif [[ -z $release_line ]]; then
        release_line=$arg
      else
        printf 'more than two arguments given\n' >&2
        usage
      fi
      ;;
  esac
done
[[ -n $version && -n $release_line ]] || usage
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  printf 'not a semantic version: %s\n' "$version" >&2
  exit 2
fi
if [[ $release_line == *$'\n'* ]]; then
  printf 'release summary must be one line\n' >&2
  exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"
date_stamp=${RELEASE_DATE:-$(date +%F)}
tag="v$version"

# The word-splitting of the prefix is the point: it is a command and its arguments.
read -r -a run_prefix <<<"${PINKI_RUN_PREFIX:-}"

failed=()
bump_started=0

restore_hint() {
  local path
  local touched=()
  for path in Cargo.toml Cargo.lock CHANGELOG.md tests/cli.rs; do
    if ! git diff --quiet -- "$path"; then
      touched+=("$path")
    fi
  done
  if ((${#touched[@]})); then
    printf 'Restore release edits: git checkout --' >&2
    printf ' %s' "${touched[@]}" >&2
    printf '\n' >&2
  fi
}

# fail MESSAGE: stop at once, or (dry run) record it and carry on.
fail() {
  if ((dry_run)); then
    failed+=("$1")
    printf 'FAIL (dry run, continuing): %s\n' "$1" >&2
  else
    printf 'FAIL: %s\n' "$1" >&2
    if ((bump_started)); then
      restore_hint
    fi
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
  branch=$(git rev-parse --abbrev-ref HEAD) || return 1
  [[ $branch == main ]] || { printf 'on %s, not main\n' "$branch" >&2; return 1; }
}

check_clean() {
  local dirty
  dirty=$(git status --porcelain) || return 1
  [[ -z $dirty ]] || { printf 'working tree is not clean:\n%s\n' "$dirty" >&2; return 1; }
}

sync_main() {
  git fetch origin main || return 1
  local ancestor_status
  if git merge-base --is-ancestor origin/main HEAD; then
    local ahead
    ahead=$(git rev-list --count origin/main..HEAD) || return 1
    ((ahead == 0)) || printf 'note: main is %s commit(s) ahead of origin/main\n' "$ahead"
    return 0
  else
    ancestor_status=$?
    ((ancestor_status == 1)) || return "$ancestor_status"
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
  else
    local local_status=$?
    ((local_status == 1)) || return "$local_status"
  fi
  local remote
  remote=$(git ls-remote --tags origin "refs/tags/$tag") || return 1
  [[ -z $remote ]] || { printf 'tag %s already exists on origin\n' "$tag" >&2; return 1; }
}

warn_recent_release() {
  local newest now age
  newest=$(git for-each-ref --sort=-creatordate --count=1 \
    --format='%(refname:short) %(creatordate:unix)' refs/tags) || return 1
  [[ -n $newest ]] || return 0
  now=$(date +%s) || return 1
  age=$((now - ${newest##* }))
  if ((age < 7 * 24 * 60 * 60)); then
    printf 'warning: newest tag %s is under 7 days old; the steward owns the one-release-per-week rule\n' \
      "${newest% *}" >&2
  fi
}

step 'on main' check_branch
step 'working tree clean' check_clean
step 'sync with origin/main' sync_main
step "tag $tag is free" check_tag_free
step 'release spacing' warn_recent_release

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
  bump_started=1
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
step 'README examples' env PINKI_BIN=target/release/pinki bash scripts/check-readme.sh
step 'cargo llvm-cov --summary-only' heavy cargo llvm-cov --summary-only
step 'coverage gate (>= 97% lines)' heavy cargo llvm-cov report --fail-under-lines 97
if ((dry_run)); then
  check_uncovered_dry_run() {
    local status
    if python3 tools/uncovered-lines.py --check; then
      return 0
    else
      status=$?
      if ((status == 1)); then
        printf 'would rewrite the uncovered-lines comment block in tests/cli.rs\n'
        return 0
      fi
      return "$status"
    fi
  }
  step 'tools/uncovered-lines.py --check' check_uncovered_dry_run
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
  printf '\nDRY RUN: every check and gate passed for %s. Tracked files were not changed.\n' "$tag"
  exit 0
fi

release_message=":bookmark: v$version — $release_line"
cat <<EOF

Release prep for $tag is done. Nothing is committed, tagged or pushed. Left for you:

  1. Edit CHANGELOG.md: write the intro paragraph under "## [$version] - $date_stamp" and
     the released-tree "Testing" line (tests, coverage; see earlier releases).
  2. Review:      git diff
  3. Commit:      git add Cargo.toml Cargo.lock CHANGELOG.md tests/cli.rs
                  git commit -m $(printf '%q' "$release_message")
  4. Tag:         git tag -a $tag -m $(printf '%q' "$release_message")
  5. Push:        git push origin main $tag
     The tag push runs release.yml, which builds and publishes the tarball.
EOF
