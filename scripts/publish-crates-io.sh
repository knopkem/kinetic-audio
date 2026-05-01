#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/publish-crates-io.sh [--execute] [--allow-dirty]

Default behavior:
  - run the crate validation steps
  - run `cargo publish --dry-run`

Options:
  --execute      Perform a real `cargo publish` after validation succeeds.
  --allow-dirty  Pass `--allow-dirty` to cargo publish / dry-run.
  -h, --help     Show this help text.
EOF
}

execute=0
allow_dirty=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --execute)
      execute=1
      ;;
    --allow-dirty)
      allow_dirty=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

publish_args=()
if [[ $allow_dirty -eq 1 ]]; then
  publish_args+=(--allow-dirty)
fi

echo "==> cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> cargo test"
cargo test

echo "==> cargo clippy --all-targets --all-features"
cargo clippy --all-targets --all-features

echo "==> cargo build --features symphonia"
cargo build --features symphonia

echo "==> cargo check --target wasm32-unknown-unknown --features 'web-backend symphonia'"
cargo check --target wasm32-unknown-unknown --features "web-backend symphonia"

if [[ $execute -eq 1 ]]; then
  echo "==> cargo publish ${publish_args[*]:-}"
  if [[ ${#publish_args[@]} -gt 0 ]]; then
    cargo publish "${publish_args[@]}"
  else
    cargo publish
  fi
else
  echo "==> cargo publish --dry-run ${publish_args[*]:-}"
  if [[ ${#publish_args[@]} -gt 0 ]]; then
    cargo publish --dry-run "${publish_args[@]}"
  else
    cargo publish --dry-run
  fi
fi
