**English** | [简体中文](README.zh-CN.md) | [日本語](README.ja.md)

# final-fantasy-resonance-export

Asset exporter and local database builder for **FINAL FANTASY RESONANCE** (C# / .NET 10 / CUE4Parse + Rust / SQLite). It mounts the game's UE5 IoStore / Pak archives in read-only mode to export DataTables, localized strings, full asset inventories, non-image assets, and level/map graphs as JSON, then compiles them into a deterministic SQLite database (`ffr.sqlite`).

> This repository is dedicated strictly to **FINAL FANTASY RESONANCE** and is not a general-purpose UE5 extractor. No game files, `.usmap` dumps, or exported outputs are committed to the repository; local working data stays under `.deps/` (or `$FFR_DEPS`).

## Quick Start (Windows)

No development environment is required. Download the packaged Windows release and export with a few clicks in the GUI:

1. **Download the archive**:
   - **Official releases**: download the latest `ffr-export-<version>-win-x64.zip` from the repository's **Releases** page.
   - **Latest development builds**: open the latest passing **CI** run on `main` in **Actions** and download `ffr-export-<version>-<short-sha>-win-x64` from **Artifacts** (retained for 30 days).
2. **Extract to one folder**: extract the entire zip archive into a single folder so `ffr-export-gui.exe`, `ffr-export.exe`, `ffr-db.exe`, and `known-builds.json` sit side by side (do not move `ffr-export-gui.exe` out by itself; the archive also includes a quick-start guide in `README-Windows.txt`).
3. **Prepare a `.usmap` mapping file**: `.usmap` files cannot be bundled and must be dumped from the running game. If none is found in the game directory or local archive, click **Fetch .usmap (launches game once)** in the GUI (or run `ffr-export.exe fetch-usmap`), or dump it manually with UE4SS; see [Obtaining `.usmap`](#obtaining-usmap) and [docs/usmap.md](docs/usmap.md).
4. **Launch `ffr-export-gui.exe`**, verify the detected paths, and click **Start Export**:
   - **Language selector**: use the top-right dropdown (**English** / **简体中文** / **日本語**) to switch UI and child-CLI language immediately (defaults to English and saves your choice to `settings.json`).
   - **Auto-detect game**: automatically scans all Steam libraries on startup and lists the game title, Steam Build ID, and install path; click **Select Game Folder…** to pick manually.
   - **Auto-locate / one-click `.usmap`**: searches `%LOCALAPPDATA%\agp\ffr-export\usmap\*.usmap`, `FFRS\Binaries\Win64\ue4ss\*.usmap`, and `FFRS\Binaries\Win64\*.usmap` in order and picks the newest file; when missing, **Fetch .usmap (launches game once)** appears.
   - **Unified data directory**: default exports, archived `.usmap` files, download caches, logs, and GUI settings live under `%LOCALAPPDATA%\agp\ffr-export\` (see [Unified Data Directory and `AGP_HOME`](#unified-data-directory-and-agp_home)); your last output root and language preference are remembered automatically (no registry writes).
   - **Settings & UE4SS cleanup**: click **Settings…** to inspect the active data root, or **Clean up UE4SS…** on the main window to uninstall UE4SS installed by this tool (or back up and remove a user-installed UE4SS if explicitly chosen).
   - **Progress & verification**: displays live progress and logs with a **Cancel** button that terminates the child process tree cleanly. When finished, it shows the database path, file/content SHA-256 hashes, `✓` / `✗` comparison against `known-builds.json`, **Open Output Folder**, and **Copy Hashes**.

> `known-builds.json` only registers the **game files + `.usmap`** combination. Reflection data (`.jmap`) is outside the scope of the exporter; when needed for research, run `ffr-db build --jmap` directly (see [docs/database.md](docs/database.md)).

GUI overview:

```text
+------------------------------------------------------------------------------+
| FINAL FANTASY RESONANCE Data Exporter          Language: [ English       v ] |
+------------------------------------------------------------------------------+
| [1. Game (FINAL FANTASY RESONANCE)]                                          |
|   Game                      | Steam Build  | Install Folder     [Browse...]  |
| > FINAL FANTASY RESONANCE   | 24250053     | ...\common\...     [Rescan]     |
+------------------------------------------------------------------------------+
| [2. Mapping File (.usmap)]                                                   |
|   .usmap (required): Auto-found FFRS-5.6.1.usmap (2026-11-25)  [Select ...]  |
+------------------------------------------------------------------------------+
| [3. Output Location]                                                         |
|   Output folder: [ %LOCALAPPDATA%\agp\ffr-export\output\ffr-24250053 ] [...] |
+------------------------------------------------------------------------------+
| [                 S T A R T   E X P O R T                 ]     [ Cancel ]   |
| Status: Step 3/5 - Building SQLite database via ffr-db    [=========>     ]  |
| +--------------------------------------------------------------------------+ |
| | ==> Step 1/5: Detecting game version and input hashes                    | |
| | ==> Step 2/5: Exporting game data to JSON                                | |
| | ==> Step 3/5: Building SQLite database via ffr-db build                  | |
| +--------------------------------------------------------------------------+ |
+------------------------------------------------------------------------------+
| [4. Result]  ✓ Matches registered build (game files + .usmap)                |
|   Database:     [ ...\ffr-24250053\ffr.sqlite          ] [Open Output Folder]|
|   File SHA256:  [ b0a228269588...                      ] [   Copy Hashes    ]|
|   Content Hash: [ 28374329602c...                      ]                     |
+------------------------------------------------------------------------------+
```

### Language Selection

Both the GUI (`ffr-export-gui`) and CLI (`ffr-export`) support **English (`en`, default)**, **Simplified Chinese (`zh-Hans`)**, and **Japanese (`ja`)**. Language never auto-switches based on the OS locale; resolution follows strict priority order:

1. Command-line option: `--lang en|zh-Hans|ja` (must appear before `locres` if used with the `locres` subcommand, because `locres --lang` selects game localization languages)
2. Environment variable: `AGP_LANG`
3. Saved GUI preference: `"language"` in `<AGP_HOME>/ffr-export/settings.json`
4. Default: `en`

When the GUI spawns `ffr-export` child processes, it forwards `--lang <active-language>` and `--progress-json` so progress tracking uses the language-independent machine protocol (`@@progress {"step":n,"total":m,...}`).

### Unified Data Directory and `AGP_HOME`

All user-facing default files for the GUI, `build-db`, the interactive wizard, and `fetch-usmap` are stored in one directory tree:

```text
%LOCALAPPDATA%\agp\ffr-export\
  output\ffr-<Steam Build ID>\  Default export output (ffr.sqlite, ffr.sqlite.build.json, and json\ with --keep-json)
  usmap\                        Archived .usmap files from fetch-usmap or manual copies (searched first)
  downloads\                    Verified UE4SS official archive cache (SHA-256 verified before reuse)
  logs\                         GUI and CLI run logs (retains the latest 10 files)
  settings.json                 GUI settings (output root, language)
```

- **Root priority**: `AGP_HOME` environment variable (used directly as the `agp` root) -> `%LOCALAPPDATA%\agp` on Windows -> `$XDG_DATA_HOME/agp` (or `~/.local/share/agp`) on Linux/macOS. On restricted Windows environments (e.g. Low Integrity / unwritable `%LOCALAPPDATA%`), the tool automatically probes `%USERPROFILE%\AppData\LocalLow\agp`, `<exeDir>\agp-data`, `%USERPROFILE%\agp`, and saved `agp-location.txt` without ever writing to `%TEMP%`.
- **Explicit path priority**: `--out` on the CLI or **Browse…** in the GUI always takes precedence; **Open Output Folder** opens the actual directory used for that run.
- **Legacy settings migration**: if `%APPDATA%\ffr-export\settings.json` exists and the new `settings.json` does not, the GUI migrates it automatically on first start and renames the old file to `settings.json.migrated` without deleting it.

### Obtaining `.usmap`

See [docs/usmap.md](docs/usmap.md) for technical details, touched files, and troubleshooting.

#### Option 1: One-click fetch (recommended on Windows)

- **GUI**: launch `ffr-export-gui.exe` and click **Fetch .usmap (launches game once)** when no `.usmap` is found. Confirm the prompt and wait for completion; the archived `.usmap` is selected automatically (canceling at any point still cleans up the temporary mod and closes the game).
- **CLI**:

```powershell
ffr-export.exe fetch-usmap --game "<Steam Game Directory>" [--timeout 300] [--yes]
```

After your confirmation, `fetch-usmap` downloads the pinned official `UE4SS v4.0.0` release (verifying exact byte size and SHA-256; existing user-installed UE4SS configurations are left untouched), deploys a temporary read-only Lua dump mod via `enabled.txt` (never modifying `mods.txt`), launches the game via Steam to dump `.usmap` on the title screen, archives it into `%LOCALAPPDATA%\agp\ffr-export\usmap\`, compares it against `known-builds.json`, removes the temporary mod, and closes the game cleanly without touching saves. UE4SS remains installed for future updates and can be removed at any time:

```powershell
ffr-export.exe uninstall-ue4ss --game "<Steam Game Directory>"
```

Or click **Clean up UE4SS…** in the GUI (only removes files recorded in our install marker unless you explicitly opt to back up and remove a user-installed UE4SS).

- **Access denied during cleanup**: the app itself never requires administrator privileges, so if UE4SS files were previously created by an elevated process (for example under `Program Files`), cleanup may hit access denied. In the GUI, a dialog shows the denied path and offers a one-click **Clean up as Administrator** button that elevates only the cleanup CLI (`ffr-export.exe uninstall-ue4ss --yes --agp-home ... --result-file ...`) and displays the result in the log; cancelling UAC makes no changes. On the CLI, `uninstall-ue4ss` strips read-only attributes on targeted files before deleting, returns exit code `5` with an `@@ue4ss-cleanup` line on access denied (or exit code `6` when a file is locked by the running game—exit the game completely and retry), and safely resumes partial cleanups without duplicating foreign UE4SS backups.

#### Option 2: Manual dump

Download [RE-UE4SS v4.0.0](https://github.com/UE4SS-RE/RE-UE4SS/releases/tag/v4.0.0), extract it to `<GameDir>\FFRS\Binaries\Win64\`, run the game to dump `.usmap`, and leave it in `FFRS\Binaries\Win64\ue4ss\` or copy it into `%LOCALAPPDATA%\agp\ffr-export\usmap\`.

### Interactive Console Wizard & CLI (Windows Optional)

You can also use `ffr-export.exe` directly from the same folder:

- **Interactive wizard**: double-click `ffr-export.exe` with no arguments to follow step-by-step prompts to pick the game, `.usmap`, and output directory (defaults to `%LOCALAPPDATA%\agp\ffr-export\output\ffr-<Steam Build ID>\`).
- **One-command database build**: run `build-db` directly (the same subcommand used behind the scenes by the GUI):

```powershell
ffr-export.exe build-db --game "<Steam Game Directory>" --usmap "<Mappings.usmap>" [--out "<Output Directory>"]
```

## CLI & Development Usage

### Prerequisites

#### 1. Install .NET 10 SDK

By default, the scripts use `.deps/dotnet` in the repository root (with NuGet cache under `.deps/nuget`, never touching `~/.dotnet`):

```bash
mkdir -p .deps
curl -fsSL https://dot.net/v1/dotnet-install.sh -o .deps/dotnet-install.sh
bash .deps/dotnet-install.sh --channel 10.0 --install-dir .deps/dotnet --no-path
```

You can also set `DOTNET_ROOT` to an existing .NET 10 SDK installation; `scripts/run.sh` and `scripts/check.sh` honor it automatically.

#### 2. Game Files and `.usmap`

- **Game files (read-only)**: place or symlink the game installation at `.deps/ffr/files/`. The default mount path is `$FFR_DEPS/ffr/files/FFRS/Content/Paks` (or `.deps/ffr/files/FFRS/Content/Paks` when `FFR_DEPS` is unset). Game files are never modified.
- **`.usmap` mapping file**: `list`, `raw`, `locres`, and `inventory` do not require `.usmap`; `export`, `export-assets`, and `export-maps` require `--usmap <file>`.

## Path Conventions and `FFR_DEPS`

All developer subcommands read `FFR_DEPS` first (matching the downstream repository `Final-Fantasy-Resonance-Agent`) and fall back to `.deps/` in the repository root.

### Default Layout

Let `DEPS="${FFR_DEPS:-.deps}"`:

| Path | Subcommand | Description |
| --- | --- | --- |
| `$DEPS/ffr/files/FFRS/Content/Paks` | All (default input) | Read-only game IoStore / Pak directory (override with `--paks`) |
| `$DEPS/ffr/export` | `export` | DataTable / StringTable JSON exports and `manifest.json` |
| `$DEPS/ffr/export/Localization` | `locres` | Localized text `<lang>.json` files and `manifest.json` |
| `$DEPS/ffr/inventory` | `inventory` | Asset inventory `registry.jsonl`, `summary.json`, `manifest.json` |
| `$DEPS/ffr/export-full` | `export-assets` | Full non-image asset JSONs, verbatim `Config/*.ini`/`.uplugin`/`.uproject`, and `manifest.json` |
| `$DEPS/ffr/export-full/maps` | `export-maps` | Level `<Level>.json` files, map connectivity `_graph.json`, and `manifest.json` |
| `$DEPS/ffr/db/ffr.sqlite` | `ffr-db build` | Compiled local SQLite game database |

### Output Directory Guardrails

- `--out` for low-level subcommands must stay inside `$FFR_DEPS` (or `.deps/`) and cannot target protected directories (`ffr`, `ffr/files`, `r1`, `dotnet`, `nuget`, `dotnet-home`, `nuget-http`, or their subdirectories).
- Non-resumable subcommands (`export`, `locres`, `inventory`) refuse non-empty directories without `--force`; with `--force`, they require a matching `manifest.json` (with `cue4ParseVersion`) owned by the same subcommand before clearing.
- Resumable subcommands (`export-assets`, `export-maps`) reuse their own output directory and skip existing files without `--force`; `export-assets --force` preserves sibling subdirectories containing another subcommand's `manifest.json` (such as `maps/`).

## Subcommands & Options

Run any subcommand via `scripts/run.sh [--lang en|zh-Hans|ja] <subcommand> [options...]`:

### 1. `list` — List archive entries

```bash
scripts/run.sh list [--paks <dir>] [--prefix <prefix>]...
```

Prints each matching virtual path and byte size, followed by the total count. Defaults to `FFRS/Content/Datatable/` and `FFRS/Content/L10N/`.

### 2. `raw` — Extract raw package bytes

```bash
scripts/run.sh raw <package-path> --out <file> [--paks <dir>]
```

Writes the raw bytes of a virtual entry to `--out` (under `$FFR_DEPS` / `.deps/`) and prints byte length and SHA-256.

### 3. `export` — Export DataTables to JSON

```bash
scripts/run.sh export --usmap <Mappings.usmap> \
  [--out <dir>] [--prefix <prefix>]... [--paks <dir>] [--force]
```

Defaults to prefixes `FFRS/Content/Datatable/` and `FFRS/Content/L10N/` and outputs to `$DEPS/ffr/export`. Each `.uasset` is exported as 2-space indented UTF-8 (no BOM, LF) JSON. Per-file failures are recorded in `manifest.json` (`failures`) without aborting the batch; exit code is `1` if any package failed.

### 4. `locres` — Export localized text

```bash
scripts/run.sh locres [--lang <game-langs>]... [--out <dir>] [--paks <dir>] [--force]
```

Reads `FFRS/Content/Localization/Game/<lang>/Game.locres` via CUE4Parse `FTextLocalizationResource`, defaulting to `zh-Hans`, `en`, and `ja`, and writes `$DEPS/ffr/export/Localization/<lang>.json` (`{ namespace: { key: text } }`) plus `manifest.json`. `--lang` accepts repeated flags or comma-separated values (e.g., `--lang zh-Hans,en,ja`).

### 5. `inventory` — Generate full asset inventory

```bash
scripts/run.sh inventory [--paks <dir>] [--out <dir>] [--force]
```

Parses `FFRS/AssetRegistry.bin` (`FAssetRegistryState`) combined with the deduplicated VFS mount table (accounting for patch overrides and companion `.ubulk`/`.uptnl` sizes) and writes `registry.jsonl`, `summary.json`, and `manifest.json` to `$DEPS/ffr/inventory`. See [docs/data-inventory.md](docs/data-inventory.md).

### 6. `export-assets` — Export all non-image assets

```bash
scripts/run.sh export-assets --usmap <Mappings.usmap> \
  [--out <dir>] [--prefix <prefix>]... [--jobs <n>] [--paks <dir>] [--force]
```

Concurrently exports all non-image, non-level, non-large-binary assets under `FFRS/` (DataTables, StringTables, Curves, DataAssets, Enhanced Input, `LevelSequence`, user-defined enums/structs, Blueprint/WidgetBlueprint CDO/component defaults/`WidgetTree`) and copies `Config/*.ini`, `.uplugin`, and `.uproject` verbatim. Supports resume, fallback for patched IoStore containers lacking `ContainerHeader`, and per-file error isolation.

Classification lookup mode (offline, prints JSON directly):

```bash
scripts/run.sh export-assets --classify DataTable,Texture2D,BlueprintGeneratedClass
```

### 7. `export-maps` — Export levels and map connectivity graph

```bash
scripts/run.sh export-maps --usmap <Mappings.usmap> \
  [--out <dir>] [--prefix <prefix>]... [--map-data <CDT_MapData_Prod.json>] \
  [--jobs <n>] [--paks <dir>] [--force]
```

Scans all `.umap` levels under `FFRS/Content/`, separates game and engine actors, extracts summaries (NPCs, treasure boxes, shops, inns, save points, transitions, transition points, event triggers, gimmicks, encounter zones), and aggregates `_graph.json`. See [docs/maps.md](docs/maps.md).

Also supports `--from-json <fixture-dir>` offline mode for synthetic testing without game paks or `.usmap`.

## JSON to SQLite (`ffr-db`)

The repository includes the Rust data crate (`crates/ffr-data`) and SQLite builder CLI (`crates/ffr-db`) to compile exported JSONs plus optional `.jmap` reflection dumps and live snapshots into a single SQLite file. See [docs/database.md](docs/database.md).

### Usage

```bash
FFR_DEPS=.deps cargo run --release -p ffr-db -- build \
  [--export <dir>] [--full <dir>] [--inventory <dir>] [--maps <dir>] \
  [--jmap <file>]... [--snapshots <dir>] [--stages <list>] [--out <file>]
```

Research build with optional `.jmap` dumps (outside the exporter workflow; see [docs/database.md](docs/database.md)):

```bash
FFR_DEPS=.deps cargo run --release -p ffr-db -- build \
  --jmap .deps/r1/<title-jmap>/<Mappings>.jmap \
  --jmap .deps/r1/<battle-jmap>/<Mappings>.jmap
```

### Inputs and Output

- **Default inputs**: read from `$FFR_DEPS/ffr/` (`export`, `export-full`, `inventory`, `export-full/maps`), individually overridable via `--export`, `--full`, `--inventory`, and `--maps`.
- **Explicit inputs**: `--jmap` (repeatable, later entries win on conflict) and `--snapshots` must be passed explicitly; omitted stages are skipped.
- **Default output**: `$FFR_DEPS/ffr/db/ffr.sqlite` (overridable via `--out <file>`). Builds into a temporary file, creates indexes, runs `ANALYZE` and `VACUUM`, and atomically replaces the target on success.

`ffr-db` also provides read-only inspection subcommands: `stats`, `sql "<SELECT ...>"`, `explain "<SELECT ...>"`, `hash`, and `bench`.

## Integration with `Final-Fantasy-Resonance-Agent`

Downstream consumers live in the sibling repository `../Final-Fantasy-Resonance-Agent`. Point `FFR_DEPS` to its `.deps` directory to export directly into it:

```bash
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh export \
  --usmap ../Final-Fantasy-Resonance-Agent/.deps/r1/<dump-dir>/<Mappings>.usmap \
  --prefix FFRS/Content/Datatable/ --force

FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh locres --force
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh inventory --force
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh export-assets \
  --usmap ../Final-Fantasy-Resonance-Agent/.deps/r1/<dump-dir>/<Mappings>.usmap --force
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh export-maps \
  --usmap ../Final-Fantasy-Resonance-Agent/.deps/r1/<dump-dir>/<Mappings>.usmap --force
```

## Building and Testing

```bash
scripts/check.sh
```

The script runs:

1. `dotnet format FfrExport.sln --verify-no-changes` (Linux)
2. `dotnet build FfrExport.sln -c Release` (`Nullable` enabled, warnings treated as errors)
3. `dotnet test FfrExport.sln -c Release --no-build` (xUnit with synthetic fixtures only)
4. `cargo fmt --check` (Linux)
5. `cargo clippy --workspace --all-targets -- -D warnings`
6. `cargo test --workspace` (`ffr-data` and `ffr-db` tests; real-data tests are auto-skipped when `.deps` is absent)

## CI and Releases

GitHub Actions runs native toolchains on both `ubuntu-latest` and `windows-latest`:

### Continuous Integration (`.github/workflows/ci.yml`)

Triggered on every `push` and `pull_request` (with `contents: read`):

1. **`check`** (`ubuntu-latest` + `windows-latest`): runs `scripts/check.sh`, plus a Windows GUI `--self-test` under an `icacls` deny-write `TEMP` directory.
2. **`fixture-hash`** (`ubuntu-latest` + `windows-latest`): runs `scripts/fixture-db.sh` and `scripts/fixture-export.sh` on synthetic fixtures and uploads hash reports/manifests.
3. **`compare-hash`** (`ubuntu-latest`): verifies byte-identical `file_sha256`, `content_sha256`, per-table hashes, and per-exported-file SHA-256 across Linux and Windows.
4. **`package-windows`** (`push` to `main`, `windows-latest`): runs `scripts/package-windows.ps1`, verifies zip entries and single-file self-tests (including `en`, `zh-Hans`, and `ja` localization), and uploads the zip artifact for 30 days.

### Cross-Platform Hash Verification

Real game data is never uploaded to the public repo or CI. Running the identical synthetic fixtures natively on Linux and Windows enforces byte-identical SQLite and JSON outputs, catching OS-, path-separator-, threading-, or line-ending-dependent drift immediately.

### Release Workflow (`.github/workflows/release.yml`)

Triggered automatically by pushing a `v*` tag:

1. Update `VERSION` at the repository root (e.g., `0.2.0`) and commit.
2. Create and push the matching tag:

```bash
git tag v0.2.0
git push origin v0.2.0
```

`release.yml` verifies the tag matches `VERSION`, runs `scripts/check.sh`, builds the native Windows x64 archive with `scripts/package-windows.ps1`, self-tests the packaged single-file binaries, and creates a GitHub Release attaching `ffr-export-<version>-win-x64.zip`, `SHA256SUMS`, and `known-builds.json` (the local Linux cross-compilation script `scripts/package-windows.sh` remains available for local builds).

## Oodle and Network Policy

- **Oodle decompression**: uses CUE4Parse's built-in open-source pure-C# `OodleSharp` only. `OodleHelper.Initialize` and `DownloadOodleDll` are never called.
- **Network scope**:
  - **Completely offline export and database build**: `list`, `raw`, `export`, `locres`, `inventory`, `export-assets`, `export-maps`, `build-db`, and `ffr-db` never make any network requests.
  - **Only one-click `.usmap` fetch accesses the network**: only when you explicitly invoke `fetch-usmap` or click **Fetch .usmap** and confirm, and neither an existing UE4SS installation nor a verified cached archive exists, does the tool download the pinned release `https://github.com/UE4SS-RE/RE-UE4SS/releases/download/v4.0.0/UE4SS_v4.0.0.zip` over HTTPS with strict byte-size and SHA-256 verification.

## License

[MIT License](LICENSE)
