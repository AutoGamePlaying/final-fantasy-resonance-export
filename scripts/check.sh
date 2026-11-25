#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
# Resolve native Windows path in Git Bash / MSYS when available so .NET env defaults stay valid.
HOST_ROOT="$(pwd -W 2>/dev/null || pwd)"

if [[ -n "${DOTNET_ROOT:-}" ]]; then
  DOTNET_DIR="${DOTNET_ROOT%/}"
  DOTNET_DIR="${DOTNET_DIR%\\}"
  if [[ -x "$DOTNET_DIR/dotnet.exe" ]]; then
    DOTNET_BIN="$DOTNET_DIR/dotnet.exe"
  else
    DOTNET_BIN="$DOTNET_DIR/dotnet"
  fi
elif [[ -x "$ROOT/.deps/dotnet/dotnet" ]]; then
  export DOTNET_ROOT="$HOST_ROOT/.deps/dotnet"
  DOTNET_BIN="$ROOT/.deps/dotnet/dotnet"
elif [[ -x "$ROOT/.deps/dotnet/dotnet.exe" ]]; then
  export DOTNET_ROOT="$HOST_ROOT/.deps/dotnet"
  DOTNET_BIN="$ROOT/.deps/dotnet/dotnet.exe"
elif command -v dotnet >/dev/null 2>&1; then
  DOTNET_BIN="$(command -v dotnet)"
else
  export DOTNET_ROOT="$HOST_ROOT/.deps/dotnet"
  DOTNET_BIN="$ROOT/.deps/dotnet/dotnet"
fi

if [[ ! -x "$DOTNET_BIN" ]]; then
  echo "错误：找不到 dotnet 可执行文件：$DOTNET_BIN（请安装 .NET 10 SDK 到 .deps/dotnet 或设置 DOTNET_ROOT）" >&2
  exit 2
fi

export DOTNET_CLI_HOME="${DOTNET_CLI_HOME:-$HOST_ROOT/.deps/dotnet-home}"
export NUGET_PACKAGES="${NUGET_PACKAGES:-$HOST_ROOT/.deps/nuget}"
export NUGET_HTTP_CACHE_PATH="${NUGET_HTTP_CACHE_PATH:-$HOST_ROOT/.deps/nuget-http}"
export DOTNET_CLI_TELEMETRY_OPTOUT=1
export DOTNET_NOLOGO=1
# Do not leave MSBuild / compiler server processes running after the script exits: they would
# inherit file descriptors such as the shared `flock /tmp/ffpr-heavy.lock` and hold it forever.
export MSBUILDDISABLENODEREUSE=1
export DOTNET_CLI_USE_MSBUILD_SERVER=0
export UseSharedCompilation=false

# Format checks run on Linux only to avoid Windows line-ending noise in CI.
SKIP_FMT="${FFR_CHECK_SKIP_FMT:-0}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    SKIP_FMT=1
    ;;
esac

if [[ "$SKIP_FMT" == "0" ]]; then
  echo "==> dotnet format --verify-no-changes"
  "$DOTNET_BIN" format FfrExport.sln --verify-no-changes
else
  echo "==> 跳过 dotnet format（非 Linux 环境或已设置 FFR_CHECK_SKIP_FMT）"
fi

echo "==> dotnet build -c Release"
"$DOTNET_BIN" build FfrExport.sln -c Release

echo "==> dotnet test"
"$DOTNET_BIN" test FfrExport.sln -c Release --no-build

# Prefer cargo already on PATH; fall back to CARGO_HOME or $HOME/.cargo/bin.
if ! command -v cargo >/dev/null 2>&1; then
  if [[ -n "${CARGO_HOME:-}" && -d "$CARGO_HOME/bin" ]]; then
    export PATH="$PATH:$CARGO_HOME/bin"
  elif [[ -d "$HOME/.cargo/bin" ]]; then
    export PATH="$PATH:$HOME/.cargo/bin"
  fi
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "错误：找不到 cargo 可执行文件（请确认 PATH 或 \$HOME/.cargo/bin 已安装 Rust 工具链）" >&2
  exit 2
fi

if [[ "$SKIP_FMT" == "0" ]]; then
  echo "==> cargo fmt --check"
  cargo fmt --check
else
  echo "==> 跳过 cargo fmt（非 Linux 环境或已设置 FFR_CHECK_SKIP_FMT）"
fi

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test --workspace"
cargo test --workspace
