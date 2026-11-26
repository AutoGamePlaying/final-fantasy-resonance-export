#!/usr/bin/env bash
set -euo pipefail

# Assemble synthetic Rust test fixtures across all database areas, run a full `ffr-db build`,
# and emit the `ffr-db hash --json` report for cross-platform reproducibility verification.

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

HASH_OUT=""
DB_OUT=""
FFR_DB_BIN="${FFR_DB_BIN:-}"

usage() {
  cat >&2 <<'EOF'
Usage: scripts/fixture-db.sh [--hash-out <json-file>] [--db-out <sqlite-file>] [--ffr-db <executable>]

Run a full `ffr-db build` across all synthetic fixtures in tests/fixtures/rust/db/ and emit `ffr-db hash --json`.
  --hash-out <file>   Write the hash JSON report to <file> (defaults to stdout)
  --db-out <file>     Keep the built SQLite database at <file> (defaults to a temporary file)
  --ffr-db <file>     Use an existing ffr-db executable (defaults to building target/release/ffr-db)
  -h, --help          Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --hash-out)
      if [[ $# -lt 2 ]]; then
        echo "Error: --hash-out requires a file path argument" >&2
        exit 2
      fi
      HASH_OUT="$2"
      shift 2
      ;;
    --db-out)
      if [[ $# -lt 2 ]]; then
        echo "Error: --db-out requires a file path argument" >&2
        exit 2
      fi
      DB_OUT="$2"
      shift 2
      ;;
    --ffr-db)
      if [[ $# -lt 2 ]]; then
        echo "Error: --ffr-db requires an executable path argument" >&2
        exit 2
      fi
      FFR_DB_BIN="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Error: unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

to_native_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -m "$1"
  else
    printf '%s' "$1"
  fi
}

FIXTURE_ROOT="$ROOT/tests/fixtures/rust/db"
if [[ ! -d "$FIXTURE_ROOT" ]]; then
  echo "Error: fixture directory not found: $FIXTURE_ROOT" >&2
  exit 2
fi

if [[ -z "$FFR_DB_BIN" ]]; then
  # Prefer cargo already on PATH; fall back to CARGO_HOME or $HOME/.cargo/bin.
  if ! command -v cargo >/dev/null 2>&1; then
    if [[ -n "${CARGO_HOME:-}" && -d "$CARGO_HOME/bin" ]]; then
      export PATH="$PATH:$CARGO_HOME/bin"
    elif [[ -d "$HOME/.cargo/bin" ]]; then
      export PATH="$PATH:$HOME/.cargo/bin"
    fi
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo executable not found and no binary was specified via --ffr-db" >&2
    exit 2
  fi

  echo "==> Building ffr-db (release)" >&2
  cargo build --release -p ffr-db >&2

  if [[ -x "$ROOT/target/release/ffr-db.exe" ]]; then
    FFR_DB_BIN="$ROOT/target/release/ffr-db.exe"
  elif [[ -x "$ROOT/target/release/ffr-db" ]]; then
    FFR_DB_BIN="$ROOT/target/release/ffr-db"
  else
    echo "Error: cargo build did not produce target/release/ffr-db" >&2
    exit 2
  fi
elif [[ ! -x "$FFR_DB_BIN" ]]; then
  echo "Error: specified ffr-db executable does not exist or is not executable: $FFR_DB_BIN" >&2
  exit 2
fi

STAGE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ffr-fixture-db.XXXXXX")"
trap 'rm -rf "$STAGE_DIR"' EXIT

IN_DIR="$STAGE_DIR/inputs"
mkdir -p \
  "$IN_DIR/export" \
  "$IN_DIR/export-full" \
  "$IN_DIR/inventory" \
  "$IN_DIR/maps" \
  "$IN_DIR/snapshots" \
  "$IN_DIR/jmaps"

# Stage `export`: load supplementary domain fixtures first, then `links/export` on top so
# cross-area references in the `links` fixture remain internally consistent.
for area in text datatable battle world links; do
  if [[ ! -d "$FIXTURE_ROOT/$area/export" ]]; then
    echo "Error: missing export fixture directory: $FIXTURE_ROOT/$area/export" >&2
    exit 2
  fi
  cp -R "$FIXTURE_ROOT/$area/export/." "$IN_DIR/export/"
done

# Stage `export-full`: all four fixture areas occupy disjoint paths.
for area in assets input_ui sequences links; do
  if [[ ! -d "$FIXTURE_ROOT/$area/export-full" ]]; then
    echo "Error: missing full-asset fixture directory: $FIXTURE_ROOT/$area/export-full" >&2
    exit 2
  fi
  cp -R "$FIXTURE_ROOT/$area/export-full/." "$IN_DIR/export-full/"
done

# Stage `inventory`.
cp -R "$FIXTURE_ROOT/assets/inventory/." "$IN_DIR/inventory/"

# Stage `maps`: load `levels/maps` first, then `links/maps` on top so `_graph.json` matches links.
for area in levels links; do
  if [[ ! -d "$FIXTURE_ROOT/$area/maps" ]]; then
    echo "Error: missing map fixture directory: $FIXTURE_ROOT/$area/maps" >&2
    exit 2
  fi
  cp -R "$FIXTURE_ROOT/$area/maps/." "$IN_DIR/maps/"
done

# Stage `snapshots`: merge both standalone snapshots and `links/snapshots`.
cp -R "$FIXTURE_ROOT/snapshots/." "$IN_DIR/snapshots/"
cp -R "$FIXTURE_ROOT/links/snapshots/." "$IN_DIR/snapshots/"

# Stage `.jmap` reflection fixtures in fixed order.
JMAP_FILES=(
  "$FIXTURE_ROOT/links/01-mock.jmap"
  "$FIXTURE_ROOT/reflection/01-base.jmap"
  "$FIXTURE_ROOT/reflection/02-override.jmap"
)
STAGED_JMAPS=()
for jmap in "${JMAP_FILES[@]}"; do
  if [[ ! -f "$jmap" ]]; then
    echo "Error: missing reflection fixture file: $jmap" >&2
    exit 2
  fi
  base="$(basename -- "$jmap")"
  cp "$jmap" "$IN_DIR/jmaps/$base"
  STAGED_JMAPS+=("$IN_DIR/jmaps/$base")
done

# Remove placeholder files so directory scanners see only fixture data.
find "$IN_DIR" -name '.gitkeep' -delete

if [[ -n "$DB_OUT" ]]; then
  mkdir -p "$(dirname -- "$DB_OUT")"
  DB_PATH="$DB_OUT"
else
  DB_PATH="$STAGE_DIR/fixture.sqlite"
fi

echo "==> Building synthetic database from all fixtures" >&2
BUILD_ARGS=(
  build
  --export "$(to_native_path "$IN_DIR/export")"
  --full "$(to_native_path "$IN_DIR/export-full")"
  --inventory "$(to_native_path "$IN_DIR/inventory")"
  --maps "$(to_native_path "$IN_DIR/maps")"
  --snapshots "$(to_native_path "$IN_DIR/snapshots")"
  --out "$(to_native_path "$DB_PATH")"
)
for jmap in "${STAGED_JMAPS[@]}"; do
  BUILD_ARGS+=(--jmap "$(to_native_path "$jmap")")
done

env -u FFR_DEPS "$FFR_DB_BIN" "${BUILD_ARGS[@]}" >&2

echo "==> Computing synthetic database hashes (ffr-db hash --json)" >&2
HASH_JSON="$(env -u FFR_DEPS "$FFR_DB_BIN" hash "$(to_native_path "$DB_PATH")" --json)"

if [[ -n "$HASH_OUT" ]]; then
  mkdir -p "$(dirname -- "$HASH_OUT")"
  printf '%s\n' "$HASH_JSON" > "$HASH_OUT"
  echo "==> Wrote hash report: $HASH_OUT" >&2
else
  printf '%s\n' "$HASH_JSON"
fi
