#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ -n "${DOTNET_ROOT:-}" ]]; then
  DOTNET_BIN="$DOTNET_ROOT/dotnet"
else
  export DOTNET_ROOT="$ROOT/.deps/dotnet"
  DOTNET_BIN="$DOTNET_ROOT/dotnet"
fi

if [[ ! -x "$DOTNET_BIN" ]]; then
  echo "错误：找不到 dotnet 可执行文件：$DOTNET_BIN（请安装 .NET 10 SDK 到 .deps/dotnet 或设置 DOTNET_ROOT）" >&2
  exit 2
fi

export DOTNET_CLI_HOME="${DOTNET_CLI_HOME:-$ROOT/.deps/dotnet-home}"
export NUGET_PACKAGES="${NUGET_PACKAGES:-$ROOT/.deps/nuget}"
export NUGET_HTTP_CACHE_PATH="${NUGET_HTTP_CACHE_PATH:-$ROOT/.deps/nuget-http}"
export DOTNET_CLI_TELEMETRY_OPTOUT=1
export DOTNET_NOLOGO=1
# Do not leave MSBuild / compiler server processes running after the script exits: they would
# inherit file descriptors such as the shared `flock /tmp/ffpr-heavy.lock` and hold it forever.
export MSBUILDDISABLENODEREUSE=1
export DOTNET_CLI_USE_MSBUILD_SERVER=0
export UseSharedCompilation=false

echo "==> dotnet format --verify-no-changes"
"$DOTNET_BIN" format FfrExport.sln --verify-no-changes

echo "==> dotnet build -c Release"
"$DOTNET_BIN" build FfrExport.sln -c Release

echo "==> dotnet test"
"$DOTNET_BIN" test FfrExport.sln -c Release --no-build
