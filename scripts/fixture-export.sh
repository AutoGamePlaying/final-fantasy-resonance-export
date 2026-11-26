#!/usr/bin/env bash
set -euo pipefail

# Build FfrExport and run all export subcommands on synthetic fixtures (matching the smoke tests),
# emitting a per-file SHA-256 manifest for cross-platform Linux vs Windows verification.

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
HOST_ROOT="$(pwd -W 2>/dev/null || pwd)"

MANIFEST_OUT=""

usage() {
  cat >&2 <<'EOF'
Usage: scripts/fixture-export.sh [--manifest-out <json-file>]

Run all export subcommands (export, locres, inventory, export-assets, export-maps) on synthetic fixtures
and emit a per-file SHA-256 manifest for cross-platform CI verification.
  --manifest-out <file>  Write the manifest JSON to <file> (defaults to stdout)
  -h, --help             Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --manifest-out)
      if [[ $# -lt 2 ]]; then
        echo "Error: --manifest-out requires a file path argument" >&2
        exit 2
      fi
      MANIFEST_OUT="$2"
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
  echo "Error: dotnet executable not found: $DOTNET_BIN (install .NET 10 SDK into .deps/dotnet or set DOTNET_ROOT)" >&2
  exit 2
fi

export DOTNET_CLI_HOME="${DOTNET_CLI_HOME:-$HOST_ROOT/.deps/dotnet-home}"
export NUGET_PACKAGES="${NUGET_PACKAGES:-$HOST_ROOT/.deps/nuget}"
export NUGET_HTTP_CACHE_PATH="${NUGET_HTTP_CACHE_PATH:-$HOST_ROOT/.deps/nuget-http}"
export DOTNET_CLI_TELEMETRY_OPTOUT=1
export DOTNET_NOLOGO=1
export MSBUILDDISABLENODEREUSE=1
export DOTNET_CLI_USE_MSBUILD_SERVER=0
export UseSharedCompilation=false

echo "==> Building FfrExport and synthetic export runner (Release)" >&2
"$DOTNET_BIN" build tests/FfrExport.TestCliHost/FfrExport.TestCliHost.csproj -c Release >&2

HOST_DLL="tests/FfrExport.TestCliHost/bin/Release/net10.0/FfrExport.TestCliHost.dll"
if [[ ! -f "$HOST_DLL" ]]; then
  echo "Error: runner assembly not found after build: $HOST_DLL" >&2
  exit 2
fi

RUN_ARGS=(exec "$(to_native_path "$HOST_DLL")" --fixture-export)
if [[ -n "$MANIFEST_OUT" ]]; then
  RUN_ARGS+=(--manifest-out "$(to_native_path "$MANIFEST_OUT")")
fi

echo "==> Running synthetic export and collecting per-file SHA-256 manifest" >&2
"$DOTNET_BIN" "${RUN_ARGS[@]}"
