#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
mkdir -p "$root/target"
scratch=$(mktemp -d "$root/target/release-prechecks.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
fixture="$scratch/fixture"
mkdir -p "$fixture/tools" "$fixture/scripts" "$scratch/bin"
cp "$root/tools/release-prep.sh" "$fixture/tools/"
cp "$root/Cargo.toml" "$root/CHANGELOG.md" "$fixture/"
printf '#!/usr/bin/env bash\nexit 0\n' > "$fixture/scripts/check-readme.sh"
cat > "$scratch/bin/cargo" <<'EOF'
#!/usr/bin/env bash
if [[ ${FAIL_BUILD:-0} == 1 && ${1:-} == build ]]; then
  exit 7
fi
exit 0
EOF
cat > "$scratch/bin/python3" <<'EOF'
#!/usr/bin/env bash
if [[ ${1:-} == tools/uncovered-lines.py ]]; then
  if [[ ${COVERAGE_RESULT:-0} == 1 ]]; then
    printf 'comment block is STALE\n'
    exit 1
  fi
  if [[ ${COVERAGE_RESULT:-0} == 2 ]]; then
    printf 'uncovered set changed\n' >&2
    exit 2
  fi
fi
if [[ ${MAKE_BUMP:-0} == 1 && ${1:-} == - ]]; then
  printf '\n# fixture bump\n' >> Cargo.toml
  printf '\nfixture bump\n' >> CHANGELOG.md
fi
exit 0
EOF
cat > "$scratch/bin/git" <<'EOF'
#!/usr/bin/env bash
if [[ ${FAIL_STATUS:-0} == 1 && $1 == status ]]; then
  printf 'simulated git status failure\n' >&2
  exit 42
fi
exec "$REAL_GIT" "$@"
EOF
chmod +x "$scratch/bin/"*

git -C "$fixture" init -q -b main
git -C "$fixture" config user.name Test
git -C "$fixture" config user.email test@example.invalid
git -C "$fixture" add .
git -C "$fixture" commit -qm fixture
git -C "$fixture" update-ref refs/remotes/origin/main "$(git -C "$fixture" rev-parse HEAD)"
git -C "$fixture" remote add origin "$scratch/nonexistent-origin"

set +e
output=$(PATH="$scratch/bin:$PATH" REAL_GIT="$(command -v git)" FAIL_STATUS=1 \
  bash "$fixture/tools/release-prep.sh" --dry-run 9.9.9 'Fixture release' 2>&1)
status=$?
set -e
[[ $status == 1 ]] || { printf 'expected exit 1, got %s\n%s\n' "$status" "$output" >&2; exit 1; }
for check in 'working tree clean' 'sync with origin/main' 'tag v9.9.9 is free'; do
  [[ $output == *"FAIL (dry run, continuing): $check"* ]] || {
    printf 'expected %s to fail\n%s\n' "$check" "$output" >&2
    exit 1
  }
done
printf 'release prechecks fail closed when git status, fetch, and ls-remote fail\n'

git -C "$fixture" remote set-url origin "$fixture"
set +e
output=$(PATH="$scratch/bin:$PATH" REAL_GIT="$(command -v git)" COVERAGE_RESULT=1 \
  bash "$fixture/tools/release-prep.sh" --dry-run 9.9.9 'Fixture release' 2>&1)
status=$?
set -e
[[ $status == 0 ]] || { printf 'expected stale block to pass, got %s\n%s\n' "$status" "$output" >&2; exit 1; }
[[ $output == *'would rewrite the uncovered-lines comment block in tests/cli.rs'* ]] || {
  printf 'missing named rewrite line\n%s\n' "$output" >&2
  exit 1
}
printf 'dry run accepts a stale comment block and names the rewrite\n'

set +e
output=$(PATH="$scratch/bin:$PATH" REAL_GIT="$(command -v git)" COVERAGE_RESULT=2 \
  bash "$fixture/tools/release-prep.sh" --dry-run 9.9.9 'Fixture release' 2>&1)
status=$?
set -e
[[ $status == 1 && $output == *'FAIL (dry run, continuing): tools/uncovered-lines.py --check'* ]] || {
  printf 'changed uncovered set must fail the dry run\n%s\n' "$output" >&2
  exit 1
}
printf 'dry run rejects a changed uncovered set\n'

git -C "$fixture" tag -a v9.9.8 -m 'fixture tag'
set +e
output=$(PATH="$scratch/bin:$PATH" REAL_GIT="$(command -v git)" \
  bash "$fixture/tools/release-prep.sh" --dry-run 9.9.9 'Fixture release' 2>&1)
status=$?
set -e
[[ $status == 0 && $output == *'warning: newest tag v9.9.8 is under 7 days old'* ]] || {
  printf 'recent release must warn without refusal\n%s\n' "$output" >&2
  exit 1
}
printf 'recent release warns without refusal\n'

set +e
output=$(PATH="$scratch/bin:$PATH" REAL_GIT="$(command -v git)" MAKE_BUMP=1 FAIL_BUILD=1 \
  bash "$fixture/tools/release-prep.sh" 9.9.9 'Fixture release' 2>&1)
status=$?
set -e
[[ $status == 1 && $output == *'Restore release edits: git checkout -- Cargo.toml CHANGELOG.md'* ]] || {
  printf 'failed build must name the exact restore command\n%s\n' "$output" >&2
  exit 1
}
printf 'failed real run names the exact restore command\n'
