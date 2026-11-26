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
用法：scripts/fixture-export.sh [--manifest-out <JSON 文件>]

使用合成数据跑一遍全部导出子命令（export / locres / inventory / export-assets / export-maps），
并收集每个输出文件的 SHA-256 清单，用于 CI 跨平台核对。
  --manifest-out <文件>  将清单 JSON 写入指定文件（不传时输出到标准输出）
  -h, --help             显示此帮助信息
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --manifest-out)
      if [[ $# -lt 2 ]]; then
        echo "错误：--manifest-out 缺少文件路径参数" >&2
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
export MSBUILDDISABLENODEREUSE=1
export DOTNET_CLI_USE_MSBUILD_SERVER=0
export UseSharedCompilation=false

echo "==> 编译 FfrExport 与合成导出运行器（Release）" >&2
"$DOTNET_BIN" build tests/FfrExport.TestCliHost/FfrExport.TestCliHost.csproj -c Release >&2

HOST_DLL="tests/FfrExport.TestCliHost/bin/Release/net10.0/FfrExport.TestCliHost.dll"
if [[ ! -f "$HOST_DLL" ]]; then
  echo "错误：编译后找不到运行器产物：$HOST_DLL" >&2
  exit 2
fi

RUN_ARGS=(exec "$(to_native_path "$HOST_DLL")" --fixture-export)
if [[ -n "$MANIFEST_OUT" ]]; then
  RUN_ARGS+=(--manifest-out "$(to_native_path "$MANIFEST_OUT")")
fi

echo "==> 运行合成导出并收集逐文件 SHA-256 清单" >&2
"$DOTNET_BIN" "${RUN_ARGS[@]}"
