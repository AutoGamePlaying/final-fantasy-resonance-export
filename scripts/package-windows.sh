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
# Do not leave MSBuild / compiler server processes running after the script exits.
export MSBUILDDISABLENODEREUSE=1
export DOTNET_CLI_USE_MSBUILD_SERVER=0
export UseSharedCompilation=false

export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v cargo >/dev/null 2>&1; then
  echo "错误：找不到 cargo 可执行文件（请确认 \$HOME/.cargo/bin 已安装 Rust 工具链）" >&2
  exit 2
fi

if ! command -v cargo-xwin >/dev/null 2>&1; then
  echo "错误：找不到 cargo-xwin（请安装 cargo-xwin 以交叉编译 Windows 可执行文件）" >&2
  exit 2
fi

# Resolve package version: prefer exact git tag on HEAD, fallback to VERSION file.
RAW_TAG="$(git describe --tags --exact-match HEAD 2>/dev/null || true)"
if [[ -n "$RAW_TAG" ]]; then
  VERSION="${RAW_TAG#v}"
elif [[ -f "$ROOT/VERSION" ]]; then
  VERSION="$(tr -d '[:space:]' < "$ROOT/VERSION")"
else
  echo "错误：找不到 git tag 且仓库根目录缺少 VERSION 文件" >&2
  exit 2
fi

if [[ -z "$VERSION" ]]; then
  echo "错误：解析到的版本号为空" >&2
  exit 2
fi

GIT_COMMIT="$(git rev-parse --short=12 HEAD 2>/dev/null || echo unknown)"

STAGE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ffr-win-pkg.XXXXXX")"
DOTNET_OUT="$STAGE_DIR/dotnet-publish"
GUI_OUT="$STAGE_DIR/gui-publish"
PACKAGE_DIR="$STAGE_DIR/package"
trap 'rm -rf "$STAGE_DIR"' EXIT

mkdir -p "$DOTNET_OUT" "$GUI_OUT" "$PACKAGE_DIR" "$ROOT/dist"

echo "==> 打包版本：$VERSION（git: $GIT_COMMIT）"
echo "==> 发布 ffr-export.exe（win-x64 self-contained single-file，不向 TEMP 自解压原生库）"
"$DOTNET_BIN" publish "$ROOT/src/FfrExport/FfrExport.csproj" \
  -c Release \
  -r win-x64 \
  --self-contained true \
  -p:PublishSingleFile=true \
  -p:IncludeNativeLibrariesForSelfExtract=false \
  -p:EnableCompressionInSingleFile=true \
  -p:DebugType=None \
  -p:DebugSymbols=false \
  -p:AssemblyName=ffr-export \
  -p:Version="$VERSION" \
  -p:FfrGitCommit="$GIT_COMMIT" \
  -o "$DOTNET_OUT"

if [[ ! -f "$DOTNET_OUT/ffr-export.exe" ]]; then
  echo "错误：dotnet publish 未生成 $DOTNET_OUT/ffr-export.exe" >&2
  exit 2
fi

echo "==> 发布 ffr-export-gui.exe（win-x64 self-contained single-file WinForms，不向 TEMP 自解压原生库）"
"$DOTNET_BIN" publish "$ROOT/src/FfrExport.Gui/FfrExport.Gui.csproj" \
  -c Release \
  -r win-x64 \
  --self-contained true \
  -p:PublishSingleFile=true \
  -p:IncludeNativeLibrariesForSelfExtract=false \
  -p:EnableCompressionInSingleFile=true \
  -p:DebugType=None \
  -p:DebugSymbols=false \
  -p:AssemblyName=ffr-export-gui \
  -p:Version="$VERSION" \
  -p:FfrGitCommit="$GIT_COMMIT" \
  -o "$GUI_OUT"

if [[ ! -f "$GUI_OUT/ffr-export-gui.exe" ]]; then
  echo "错误：dotnet publish 未生成 $GUI_OUT/ffr-export-gui.exe" >&2
  exit 2
fi

echo "==> 交叉编译 ffr-db.exe（x86_64-pc-windows-msvc）"
cargo xwin build --release --target x86_64-pc-windows-msvc -p ffr-db

FFR_DB_EXE="$ROOT/target/x86_64-pc-windows-msvc/release/ffr-db.exe"
if [[ ! -f "$FFR_DB_EXE" ]]; then
  echo "错误：cargo xwin 未生成 $FFR_DB_EXE" >&2
  exit 2
fi

for required in known-builds.json README.md LICENSE; do
  if [[ ! -f "$ROOT/$required" ]]; then
    echo "错误：打包缺少必需文件：$ROOT/$required" >&2
    exit 2
  fi
done

cp "$DOTNET_OUT/ffr-export.exe" "$PACKAGE_DIR/ffr-export.exe"
cp "$GUI_OUT/ffr-export-gui.exe" "$PACKAGE_DIR/ffr-export-gui.exe"
cp "$FFR_DB_EXE" "$PACKAGE_DIR/ffr-db.exe"
cp "$ROOT/known-builds.json" "$PACKAGE_DIR/known-builds.json"
cp "$ROOT/README.md" "$PACKAGE_DIR/README.md"
cp "$ROOT/LICENSE" "$PACKAGE_DIR/LICENSE"

# Copy any native sidecar DLLs emitted alongside the single-file executables and verify duplicates match.
COPIED_DLLS=()
for publish_dir in "$DOTNET_OUT" "$GUI_OUT"; do
  shopt -s nullglob
  for dll in "$publish_dir"/*.dll; do
    base="$(basename "$dll")"
    dst="$PACKAGE_DIR/$base"
    if [[ -f "$dst" ]]; then
      existing_hash="$(sha256sum "$dst" | awk '{print $1}')"
      incoming_hash="$(sha256sum "$dll" | awk '{print $1}')"
      if [[ "$existing_hash" != "$incoming_hash" ]]; then
        echo "错误：CLI 与 GUI 产出的同名原生库内容不一致：$base" >&2
        exit 2
      fi
    else
      cp "$dll" "$dst"
      COPIED_DLLS+=("$base")
    fi
  done
  shopt -u nullglob
done

if (( ${#COPIED_DLLS[@]} > 0 )); then
  echo "==> 已随包复制原生库（${#COPIED_DLLS[@]} 个）：${COPIED_DLLS[*]}"
else
  echo "==> 本次 .NET 10 win-x64 单文件发布未产出额外外部原生 DLL（运行时原生代码已内置于 exe 中，不向 TEMP 解压）"
fi

# Write a concise Chinese quick-start guide (<= 10 lines, UTF-8 without BOM, CRLF for Windows Notepad).
printf '%s\r\n' \
  '《FINAL FANTASY RESONANCE》数据导出工具使用说明' \
  '' \
  '1. 将本压缩包完整解压到同一个文件夹（不要单独移动某个 exe 或原生库）。' \
  '2. 如双击时提示低完整性级别，可右键压缩包 -> 属性 -> 解除锁定，再用资源管理器重新解压。' \
  '3. 双击打开 ffr-export-gui.exe，可一键获取 .usmap 或手动选择，确认后点「开始导出」。' \
  '4. 导出完成后可点「打开输出文件夹」查看 ffr.sqlite，或点「复制哈希」核对版本。' \
  '5. 如需清理游戏目录中的 UE4SS，可点击主窗口「清理 UE4SS…」按钮；命令行用法见 README.md。' \
  > "$PACKAGE_DIR/使用说明.txt"

(
  cd "$PACKAGE_DIR"
  mapfile -t INNER_FILES < <(LC_ALL=C find . -maxdepth 1 -type f ! -name 'SHA256SUMS' -printf '%f\n' | LC_ALL=C sort)
  sha256sum "${INNER_FILES[@]}" > SHA256SUMS
)

ZIP_PATH="$ROOT/dist/ffr-export-${VERSION}-win-x64.zip"
rm -f "$ZIP_PATH"

echo "==> 生成压缩包：$ZIP_PATH"
# Prefer Python zipfile so UTF-8 entry names (使用说明.txt) always carry the ZIP UTF-8 flag for Windows Explorer.
if command -v python3 >/dev/null 2>&1; then
  python3 - "$PACKAGE_DIR" "$ZIP_PATH" <<'PY'
import os
import sys
import zipfile

pkg_dir, zip_path = sys.argv[1], sys.argv[2]
files = sorted(
    name for name in os.listdir(pkg_dir)
    if os.path.isfile(os.path.join(pkg_dir, name))
)
with zipfile.ZipFile(zip_path, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
    for name in files:
        zf.write(os.path.join(pkg_dir, name), arcname=name)
print("==> 压缩包内文件列表（zip 内容）：")
with zipfile.ZipFile(zip_path, "r") as zf:
    for info in zf.infolist():
        print(f"  {info.filename}\t{info.file_size} 字节\t(压缩后 {info.compress_size} 字节)")
PY
elif command -v zip >/dev/null 2>&1; then
  (
    cd "$PACKAGE_DIR"
    mapfile -t ZIP_FILES < <(LC_ALL=C find . -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort)
    zip -q -9 "$ZIP_PATH" "${ZIP_FILES[@]}"
  )
  unzip -l "$ZIP_PATH"
else
  echo "错误：缺少 python3 或 zip，无法生成 .zip 压缩包" >&2
  exit 2
fi

echo "==> 打包完成：$ZIP_PATH"
ls -lh "$ZIP_PATH"
echo "==> 压缩包内文件与校验："
ls -lh "$PACKAGE_DIR"
cat "$PACKAGE_DIR/SHA256SUMS"
