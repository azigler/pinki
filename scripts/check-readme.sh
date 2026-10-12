#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

if [[ -n ${PINKI_BIN:-} ]]; then
  binary=$PINKI_BIN
elif [[ -x target/release/pinki ]]; then
  binary=target/release/pinki
elif command -v pinki >/dev/null 2>&1; then
  binary=$(command -v pinki)
else
  cargo build --release --locked
  binary=target/release/pinki
fi

if [[ ! -x $binary ]]; then
  printf 'pinki binary is not executable: %s\n' "$binary" >&2
  exit 1
fi
binary=$(realpath "$binary")

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
mkdir "$scratch/bin" "$scratch/run"
ln -s "$binary" "$scratch/bin/pinki"

{
  printf 'set -euo pipefail\n'
  printf "trap 'status=\$?; printf \"README sh block %%s failed (exit %%s)\\n\" \"\$readme_block\" \"\$status\" >&2' ERR\n"
  awk '
    /^```sh[[:space:]]*$/ { in_block = 1; block++; print "readme_block=" block; next }
    in_block && /^```[[:space:]]*$/ {
      print "printf \047README sh block " block " passed\\n\047"
      in_block = 0
      next
    }
    in_block { print }
    END {
      if (in_block) { print "unclosed sh fence" > "/dev/stderr"; exit 2 }
      if (block == 0) { print "no sh fences found" > "/dev/stderr"; exit 2 }
    }
  ' README.md
} > "$scratch/readme-examples.sh"

(
  cd "$scratch/run"
  PATH="$scratch/bin:$PATH" bash "$scratch/readme-examples.sh"
)
