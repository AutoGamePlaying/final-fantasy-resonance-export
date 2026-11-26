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
  echo "Error: dotnet executable not found: $DOTNET_BIN (install .NET 10 SDK into .deps/dotnet or set DOTNET_ROOT)" >&2
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
  echo "Error: cargo executable not found (ensure the Rust toolchain is installed in \$HOME/.cargo/bin)" >&2
  exit 2
fi

if ! command -v cargo-xwin >/dev/null 2>&1; then
  echo "Error: cargo-xwin not found (install cargo-xwin to cross-compile Windows executables)" >&2
  exit 2
fi

# Resolve package version: prefer exact git tag on HEAD, fallback to VERSION file.
RAW_TAG="$(git describe --tags --exact-match HEAD 2>/dev/null || true)"
if [[ -n "$RAW_TAG" ]]; then
  VERSION="${RAW_TAG#v}"
elif [[ -f "$ROOT/VERSION" ]]; then
  VERSION="$(tr -d '[:space:]' < "$ROOT/VERSION")"
else
  echo "Error: no git tag on HEAD and VERSION file is missing at repository root" >&2
  exit 2
fi

if [[ -z "$VERSION" ]]; then
  echo "Error: resolved package version is empty" >&2
  exit 2
fi

GIT_COMMIT="$(git rev-parse --short=12 HEAD 2>/dev/null || echo unknown)"

STAGE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ffr-win-pkg.XXXXXX")"
DOTNET_OUT="$STAGE_DIR/dotnet-publish"
GUI_OUT="$STAGE_DIR/gui-publish"
PACKAGE_DIR="$STAGE_DIR/package"
trap 'rm -rf "$STAGE_DIR"' EXIT

mkdir -p "$DOTNET_OUT" "$GUI_OUT" "$PACKAGE_DIR" "$ROOT/dist"

echo "==> Packaging version: $VERSION (git: $GIT_COMMIT)"
echo "==> Publishing ffr-export.exe (win-x64 self-contained single-file, no TEMP native self-extract)"
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
  echo "Error: dotnet publish did not produce $DOTNET_OUT/ffr-export.exe" >&2
  exit 2
fi

echo "==> Publishing ffr-export-gui.exe (win-x64 self-contained single-file WinForms, no TEMP native self-extract)"
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
  echo "Error: dotnet publish did not produce $GUI_OUT/ffr-export-gui.exe" >&2
  exit 2
fi

echo "==> Cross-compiling ffr-db.exe (x86_64-pc-windows-msvc)"
cargo xwin build --release --target x86_64-pc-windows-msvc -p ffr-db

FFR_DB_EXE="$ROOT/target/x86_64-pc-windows-msvc/release/ffr-db.exe"
if [[ ! -f "$FFR_DB_EXE" ]]; then
  echo "Error: cargo xwin did not produce $FFR_DB_EXE" >&2
  exit 2
fi

for required in known-builds.json README.md LICENSE; do
  if [[ ! -f "$ROOT/$required" ]]; then
    echo "Error: required packaging file not found: $ROOT/$required" >&2
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
        echo "Error: mismatched sidecar native DLL between CLI and GUI publish outputs: $base" >&2
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
  echo "==> Copied ${#COPIED_DLLS[@]} sidecar native DLL(s): ${COPIED_DLLS[*]}"
else
  echo "==> No external native sidecar DLLs emitted by .NET 10 win-x64 single-file publish"
fi

# Write Windows quick-start guide (English primary with concise Chinese & Japanese sections, UTF-8 without BOM, CRLF for Notepad).
printf '%s\r\n' \
  'FINAL FANTASY RESONANCE Data Exporter (Windows Quick Start)' \
  '' \
  '1. Extract this entire zip archive into one folder (keep all .exe files and known-builds.json together).' \
  '2. If Windows warns about Low Integrity Level when launching, right-click the .zip -> Properties -> Unblock, then re-extract with File Explorer.' \
  '3. Double-click ffr-export-gui.exe. Switch language anytime in the top-right dropdown (English / 简体中文 / 日本語).' \
  '4. Click "Fetch .usmap (launches game once)" or select an existing .usmap file, then click "Start Export".' \
  '5. When finished, click "Open Output Folder" to view ffr.sqlite or "Copy Hashes" to compare against known-builds.json.' \
  '6. To remove UE4SS from the game folder, click "Clean up UE4SS..." in the main window. For CLI usage, see README.md.' \
  '' \
  '---' \
  '[简体中文简版]' \
  '1. 将整个压缩包解压到同一个文件夹（保留所有 exe 与 known-builds.json 在一起）。' \
  '2. 若启动时提示低完整性级别，右键压缩包 -> 属性 -> 解除锁定，再用资源管理器重新解压。' \
  '3. 双击 ffr-export-gui.exe，可在右上角切换语言（English / 简体中文 / 日本語）；获取或选择 .usmap 后点「开始导出」。' \
  '4. 完成后可点「打开输出文件夹」查看 ffr.sqlite，或点「清理 UE4SS…」卸载 UE4SS；命令行用法见 README.md。' \
  '' \
  '---' \
  '[日本語クイックガイド]' \
  '1. zip 全体を 1 つのフォルダに展開してください（すべての exe と known-builds.json を同じ場所に保ちます）。' \
  '2. 起動時に低整合性レベルの警告が出た場合は、zip を右クリック -> プロパティ -> 許可する（ブロック解除）後にエクスプローラーで再展開してください。' \
  '3. ffr-export-gui.exe を開き、右上で表示言語（English / 简体中文 / 日本語）を切り替えられます。.usmap を取得または選択して「エクスポート開始」をクリックします。' \
  '4. 完了後は「出力フォルダを開く」で ffr.sqlite を確認できます。UE4SS の削除は「UE4SS をクリーンアップ…」から行えます。CLI の使い方は README.md を参照してください。' \
  > "$PACKAGE_DIR/README-Windows.txt"

(
  cd "$PACKAGE_DIR"
  mapfile -t INNER_FILES < <(LC_ALL=C find . -maxdepth 1 -type f ! -name 'SHA256SUMS' -printf '%f\n' | LC_ALL=C sort)
  sha256sum "${INNER_FILES[@]}" > SHA256SUMS
)

ZIP_PATH="$ROOT/dist/ffr-export-${VERSION}-win-x64.zip"
rm -f "$ZIP_PATH"

echo "==> Creating archive: $ZIP_PATH"
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
print("==> Zip archive entries:")
with zipfile.ZipFile(zip_path, "r") as zf:
    for info in zf.infolist():
        print(f"  {info.filename}\t{info.file_size} bytes\t(compressed {info.compress_size} bytes)")
PY
elif command -v zip >/dev/null 2>&1; then
  (
    cd "$PACKAGE_DIR"
    mapfile -t ZIP_FILES < <(LC_ALL=C find . -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort)
    zip -q -9 "$ZIP_PATH" "${ZIP_FILES[@]}"
  )
  unzip -l "$ZIP_PATH"
else
  echo "Error: neither python3 nor zip is available to build the .zip archive" >&2
  exit 2
fi

echo "==> Packaging complete: $ZIP_PATH"
ls -lh "$ZIP_PATH"
echo "==> Packaged files and checksums:"
ls -lh "$PACKAGE_DIR"
cat "$PACKAGE_DIR/SHA256SUMS"
