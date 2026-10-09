#!/usr/bin/env bash
# Run every check CI runs. Exits non-zero if any fail.
#
#   ./check.sh          all checks
#   ./check.sh --fix    format Rust code first, then check
set -uo pipefail
cd "$(dirname "$0")"

if [[ "${1:-}" == "--fix" ]]; then
  cargo fmt --all --manifest-path src-tauri/Cargo.toml
fi

if [[ ! -d node_modules || pnpm-lock.yaml -nt node_modules/.modules.yaml ]]; then
  pnpm install --frozen-lockfile || exit 1
fi

failed=()
run() {
  local name="$1"
  shift
  printf '\033[1;34m==>\033[0m %s\n' "$name"
  if "$@"; then
    printf '\033[1;32m  ok\033[0m\n'
  else
    printf '\033[1;31m  FAILED\033[0m\n'
    failed+=("$name")
  fi
}

manifest=src-tauri/Cargo.toml
run "TypeScript typecheck"  pnpm -s typecheck
run "Frontend tests"        pnpm -s test
run "Frontend build"        pnpm -s exec vite build --logLevel warn
run "Rust format"           cargo fmt --all --manifest-path "$manifest" --check
run "Rust lint (clippy)"    cargo clippy --manifest-path "$manifest" --all-targets --quiet -- -D warnings
run "Rust tests"            cargo test --manifest-path "$manifest" --quiet

echo
if ((${#failed[@]})); then
  printf '\033[1;31m%d check(s) failed:\033[0m %s\n' "${#failed[@]}" "${failed[*]}"
  [[ " ${failed[*]} " == *"Rust format"* ]] && echo "Tip: ./check.sh --fix formats Rust code."
  exit 1
fi
printf '\033[1;32mAll checks passed.\033[0m\n'
