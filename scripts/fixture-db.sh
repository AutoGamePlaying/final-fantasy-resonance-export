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
用法：scripts/fixture-db.sh [--hash-out <JSON 文件>] [--db-out <SQLite 文件>] [--ffr-db <可执行文件>]

使用 tests/fixtures/rust/db/ 下的全部区域合成夹具执行完整 ffr-db build，并输出 ffr-db hash --json。
  --hash-out <文件>   将哈希 JSON 写入指定文件（不传时输出到标准输出）
  --db-out <文件>     保留构建出的 SQLite 数据库到指定路径（不传时使用临时文件并自动清理）
  --ffr-db <文件>     使用指定的 ffr-db 可执行文件（不传时自动编译 target/release/ffr-db）
  -h, --help          显示此帮助信息
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --hash-out)
      if [[ $# -lt 2 ]]; then
        echo "错误：--hash-out 缺少文件路径参数" >&2
        exit 2
      fi
      HASH_OUT="$2"
      shift 2
      ;;
    --db-out)
      if [[ $# -lt 2 ]]; then
        echo "错误：--db-out 缺少文件路径参数" >&2
        exit 2
      fi
      DB_OUT="$2"
      shift 2
      ;;
    --ffr-db)
      if [[ $# -lt 2 ]]; then
        echo "错误：--ffr-db 缺少可执行文件路径参数" >&2
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
      echo "错误：未知参数：$1" >&2
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
  echo "错误：找不到假数据目录：$FIXTURE_ROOT" >&2
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
    echo "错误：找不到 cargo 可执行文件，且未通过 --ffr-db 指定二进制路径" >&2
    exit 2
  fi

  echo "==> 编译 ffr-db（release）" >&2
  cargo build --release -p ffr-db >&2

  if [[ -x "$ROOT/target/release/ffr-db.exe" ]]; then
    FFR_DB_BIN="$ROOT/target/release/ffr-db.exe"
  elif [[ -x "$ROOT/target/release/ffr-db" ]]; then
    FFR_DB_BIN="$ROOT/target/release/ffr-db"
  else
    echo "错误：cargo build 未生成 target/release/ffr-db 可执行文件" >&2
    exit 2
  fi
elif [[ ! -x "$FFR_DB_BIN" ]]; then
  echo "错误：指定的 ffr-db 可执行文件不存在或不可执行：$FFR_DB_BIN" >&2
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
    echo "错误：缺少导出夹具目录：$FIXTURE_ROOT/$area/export" >&2
    exit 2
  fi
  cp -R "$FIXTURE_ROOT/$area/export/." "$IN_DIR/export/"
done

# Stage `export-full`: all four fixture areas occupy disjoint paths.
for area in assets input_ui sequences links; do
  if [[ ! -d "$FIXTURE_ROOT/$area/export-full" ]]; then
    echo "错误：缺少全量资源夹具目录：$FIXTURE_ROOT/$area/export-full" >&2
    exit 2
  fi
  cp -R "$FIXTURE_ROOT/$area/export-full/." "$IN_DIR/export-full/"
done

# Stage `inventory`.
cp -R "$FIXTURE_ROOT/assets/inventory/." "$IN_DIR/inventory/"

# Stage `maps`: load `levels/maps` first, then `links/maps` on top so `_graph.json` matches links.
for area in levels links; do
  if [[ ! -d "$FIXTURE_ROOT/$area/maps" ]]; then
    echo "错误：缺少关卡夹具目录：$FIXTURE_ROOT/$area/maps" >&2
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
    echo "错误：缺少反射夹具文件：$jmap" >&2
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

echo "==> 使用全部合成夹具构建假数据库" >&2
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

echo "==> 计算假数据库哈希（ffr-db hash --json）" >&2
HASH_JSON="$(env -u FFR_DEPS "$FFR_DB_BIN" hash "$(to_native_path "$DB_PATH")" --json)"

if [[ -n "$HASH_OUT" ]]; then
  mkdir -p "$(dirname -- "$HASH_OUT")"
  printf '%s\n' "$HASH_JSON" > "$HASH_OUT"
  echo "==> 已写入哈希报告：$HASH_OUT" >&2
else
  printf '%s\n' "$HASH_JSON"
fi
