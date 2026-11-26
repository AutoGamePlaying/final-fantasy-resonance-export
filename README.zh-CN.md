[English](README.md) | **简体中文** | [日本語](README.ja.md)

# final-fantasy-resonance-export

《FINAL FANTASY RESONANCE》的资源导出与本地建库工具（C# / .NET 10 / CUE4Parse + Rust / SQLite）。工具以只读方式挂载游戏安装目录中的 UE5 IoStore / Pak 资源包，导出数据表、多语言文本、资源总索引、全量非图像资源以及关卡与地图图谱为 JSON，并构建为确定性的本地 SQLite 数据库（`ffr.sqlite`）。

> 本仓库仅面向《FINAL FANTASY RESONANCE》服务，不做通用化；所有游戏文件、`.usmap` 映射文件与导出产物均留在 `.deps/`（或 `$FFR_DEPS`）下，不进入版本库。

## 快速开始（Windows）

无需安装开发环境，直接下载打包好的 Windows 版本并用图形界面点几下即可导出：

1. **下载压缩包**：
   - **正式版本**：前往仓库的 **Releases** 页面下载最新 `ffr-export-<版本>-win-x64.zip`；
   - **最新开发构建**：前往仓库的 **Actions** 页面，打开最新一次 `main` 分支通过的 **CI** 记录，在底部的 **Artifacts** 中下载 `ffr-export-<版本>-<提交短号>-win-x64`（保留 30 天）。
2. **解压到同一目录**：将压缩包完整解压到一个文件夹，确保 `ffr-export-gui.exe`、`ffr-export.exe`、`ffr-db.exe` 与 `known-builds.json` 位于同一目录（不要只单独移动 `ffr-export-gui.exe`；压缩包内另附 `README-Windows.txt` 使用说明，含英/中/日三语）。
3. **准备 `.usmap` 映射文件**：`.usmap` 无法随工具分发，需要从运行中的游戏内存转储。若游戏目录和本地缓存里还没有 `.usmap`，可以直接在窗口程序里点 **一键获取 .usmap（会启动一次游戏）**（或运行 `ffr-export.exe fetch-usmap`），也可以手动用 UE4SS 转储；详见下文 [获取 `.usmap`](#获取-usmap) 与 [docs/usmap.md](docs/usmap.md)。
4. **双击打开 `ffr-export-gui.exe`**，按界面顺序确认并点击 **开始导出**：
   - **切换界面语言**：默认使用英文，可在窗口右上角下拉框切换 **English** / **简体中文** / **日本語**，切换后立刻生效并自动写入 `settings.json`（启动后台命令行时也会透传 `--lang`）；
   - **自动扫描游戏**：启动时自动扫描 Steam 全部游戏库并列出名称、Steam 构建号和安装目录；若未找到可点 **手动选择游戏目录…**；
   - **自动查找 / 一键获取 `.usmap`（及可选 `.jmap`）**：依次查找 `%LOCALAPPDATA%\agp\ffr-export\usmap\*.usmap`、`FFRS\Binaries\Win64\ue4ss\*.usmap` 与 `FFRS\Binaries\Win64\*.usmap` 并选中最新一份；未找到时会显示 **一键获取 .usmap（会启动一次游戏）** 按钮；勾选 **包含反射数据（.jmap，可选）** 后会在 `ue4ss\` / `Win64\` 下自动列出 `.jmap`（默认不勾选）；
   - **输出位置与统一数据目录**：默认导出、`.usmap` 归档、下载缓存、日志与设置统一保存在 `%LOCALAPPDATA%\agp\ffr-export\`（详见下方 [统一数据目录与 `AGP_HOME`](#统一数据目录与-agp_home)），下次启动会自动记住上次的输出根目录、勾选状态与语言选择（不写注册表）；
   - **设置与清理 UE4SS**：点击 **设置…** 可查看数据目录；点击主窗口 **清理 UE4SS…** 可卸载本工具安装的 UE4SS（或按需备份并移走用户原有 UE4SS）；
   - **完成与核对**：导出时界面实时显示进度条与日志，可随时点 **取消** 结束后台进程树；完成后直接显示数据库路径、文件哈希、内容哈希与 `✓` / `✗` 比对结论，并提供 **打开输出文件夹** 与 **复制哈希** 按钮。

> 已知版本表（`known-builds.json`）同时登记了两种输入组合：“只有 `.usmap`”和“`.usmap` 加两份 `.jmap`”，因此勾选或不勾选 `.jmap` 都可能与登记版本一致。

窗口界面示意：

```text
+------------------------------------------------------------------------------+
| 《FINAL FANTASY RESONANCE》数据导出工具            语言: [ 简体中文      v ] |
+------------------------------------------------------------------------------+
| [1. 游戏（FINAL FANTASY RESONANCE）]                                         |
|   游戏名称                  | Steam 构建号 | 安装目录           [手动选择…]  |
| > FINAL FANTASY RESONANCE   | 24250053     | ...\common\...     [重新扫描]   |
+------------------------------------------------------------------------------+
| [2. 映射与反射文件（.usmap / .jmap）]                                        |
|   .usmap（必填）：已自动找到 FFRS-5.6.1.usmap（2026-11-25）     [选择 .usmap…]|
|   [ ] 包含反射数据（.jmap，可选）                                            |
|       (勾选后自动扫描 ue4ss / Win64 下的 *.jmap，支持添加/移除)              |
+------------------------------------------------------------------------------+
| [3. 输出位置]                                                                |
|   输出目录：[ %LOCALAPPDATA%\agp\ffr-export\output\ffr-24250053   ] [ 浏览… ]  |
+------------------------------------------------------------------------------+
| [                  开 始 导 出                  ]                [ 取 消 ]   |
| 状态：步骤 3/5 — 调用 ffr-db build 构建 SQLite 数据库   [=========>      ]   |
| +--------------------------------------------------------------------------+ |
| | ==> 步骤 1/5：识别游戏版本与输入哈希                                     | |
| | ==> 步骤 2/5：导出游戏数据为 JSON                                        | |
| | ==> 步骤 3/5：调用 ffr-db build 构建 SQLite 数据库                       | |
| +--------------------------------------------------------------------------+ |
+------------------------------------------------------------------------------+
| [4. 完成结果]  ✓ 与登记的版本一致（仅 .usmap / .usmap + 两份 .jmap 均可比对）|
|   数据库路径：[ ...\ffr-24250053\ffr.sqlite                  ] [打开输出文件夹]|
|   文件哈希：  [ b0a228269588...                              ] [  复制哈希   ]|
|   内容哈希：  [ 28374329602c...                              ]                |
+------------------------------------------------------------------------------+
```

### 语言选择

窗口程序（`ffr-export-gui`）与命令行（`ffr-export`）共用一套本地化资源，支持 **英文（`en`，默认）**、**简体中文（`zh-Hans`）** 与 **日文（`ja`）**。程序**不会**跟随系统语言自动切换，按以下优先顺序决定当前语言：

1. 命令行参数：`--lang en|zh-Hans|ja`（与 `locres` 子命令混用时请放在 `locres` 之前，因为 `locres --lang` 用于指定要导出的游戏语言）
2. 环境变量：`AGP_LANG`
3. 窗口程序保存的设置：`<AGP_HOME>/ffr-export/settings.json` 中的 `"language"`
4. 默认值：`en`

窗口程序调用命令行子进程时会自动传入 `--lang <当前语言>` 与 `--progress-json`，并通过与语言无关的 `@@progress {"step":n,"total":m,...}` 协议驱动进度条。

### 统一数据目录与 `AGP_HOME`

窗口程序与 `build-db` / 交互式向导 / `fetch-usmap` 的默认文件全部集中在统一目录下：

```text
%LOCALAPPDATA%\agp\ffr-export\
  output\ffr-<Steam 构建号>\   默认导出位置（ffr.sqlite、ffr.sqlite.build.json、--keep-json 时的 json\）
  usmap\                       一键获取或手动复制来的 .usmap（自动查找优先覆盖此目录）
  downloads\                   UE4SS 官方安装包下载缓存（复用前会校验 SHA-256）
  logs\                        窗口程序与命令行的运行日志（仅保留最近 10 个）
  settings.json                窗口程序的设置（输出根目录、.jmap 开关、语言选择）
```

- **根目录优先级**：环境变量 `AGP_HOME`（直接作为 `agp` 这一层目录）→ Windows 的 `%LOCALAPPDATA%\agp` → Linux/macOS 的 `$XDG_DATA_HOME/agp`（未设置时为 `~/.local/share/agp`）。在低完整性级别或 `%LOCALAPPDATA%` 不可写的 Windows 环境下，会自动依次回退检测 `%USERPROFILE%\AppData\LocalLow\agp`、`<程序目录>\agp-data`、`%USERPROFILE%\agp` 及已保存的 `agp-location.txt`，绝不回退写入 `%TEMP%`；
- **显式路径优先**：命令行传入的 `--out` 或窗口程序里通过 **浏览…** 选定的目录照旧优先使用；**打开输出文件夹** 打开的是本次实际导出的目录；
- **旧版设置迁移**：若旧版 `%APPDATA%\ffr-export\settings.json` 存在而新位置不存在，首次启动会自动迁移并将旧文件改名为 `settings.json.migrated`（不删除）。

### 获取 `.usmap`

详细原理、改动文件列表与排查建议见 [docs/usmap.md](docs/usmap.md)。

#### 方式一：一键获取（推荐，Windows）

- **图形界面**：打开 `ffr-export-gui.exe`，当 `.usmap` 缺失时点击 **一键获取 .usmap（会启动一次游戏）**，确认弹窗说明后等待自动完成并自动选中新的 `.usmap`（随时可取消，取消时同样停用模组并关闭游戏）；
- **命令行**：

```powershell
ffr-export.exe fetch-usmap --game "<Steam 游戏目录>" [--timeout 300] [--yes]
```

一键获取仅在您确认后按固定版本下载官方 `UE4SS v4.0.0`（校验大小与 SHA-256，若游戏目录已自行安装 UE4SS 则完全不动原有配置），部署临时只读模组（使用 `enabled.txt`，不改 `mods.txt`），经 Steam 启动游戏并在标题画面转储 `.usmap`，归档到 `%LOCALAPPDATA%\agp\ffr-export\usmap\` 并与 `known-builds.json` 核对，最后移除临时模组并安全关闭游戏（停在标题画面，不触碰存档）。UE4SS 本体保留供下次复用，随时可卸载：

```powershell
ffr-export.exe uninstall-ue4ss --game "<Steam 游戏目录>"
```

或在窗口程序点击 **清理 UE4SS…**（默认只卸载本工具安装的 UE4SS，不碰用户自行安装的 UE4SS）。

#### 方式二：手动获取

自行从 [RE-UE4SS v4.0.0](https://github.com/UE4SS-RE/RE-UE4SS/releases/tag/v4.0.0) 下载解压到 `<游戏目录>\FFRS\Binaries\Win64\`，运行游戏通过 UE4SS 转储 `.usmap` 后，留在 `FFRS\Binaries\Win64\ue4ss\` 或复制到 `%LOCALAPPDATA%\agp\ffr-export\usmap\` 即可被工具自动发现。

### 控制台向导与命令行（Windows 可选）

如果偏好终端操作或在脚本中自动化运行，也可以直接使用同目录下的 `ffr-export.exe`：

- **交互式控制台向导**：双击运行不带参数的 `ffr-export.exe`（可配合 `AGP_LANG=zh-Hans` 或 `--lang zh-Hans`），按提示选择游戏、确认 `.usmap`/`.jmap` 与输出目录（默认 `%LOCALAPPDATA%\agp\ffr-export\output\ffr-<Steam 构建号>\`）即可完成导出与建库。
- **命令行一键建库**：直接带参数调用 `build-db` 子命令（窗口程序后台调用的也是同一条命令）：

```powershell
ffr-export.exe --lang zh-Hans build-db --game "<Steam 游戏目录>" --usmap "<Mappings.usmap>" [--out "<输出目录>"]
```

## 命令行与开发用法

### 环境准备

#### 1. 安装 .NET 10 SDK

默认使用仓库根目录下的 `.deps/dotnet`（NuGet 缓存位于 `.deps/nuget`，不改动 `~/.dotnet`）：

```bash
mkdir -p .deps
curl -fsSL https://dot.net/v1/dotnet-install.sh -o .deps/dotnet-install.sh
bash .deps/dotnet-install.sh --channel 10.0 --install-dir .deps/dotnet --no-path
```

也可以通过环境变量 `DOTNET_ROOT` 指定已安装好的 .NET 10 SDK 目录，`scripts/run.sh` 与 `scripts/check.sh` 会自动识别。

#### 2. 准备游戏文件与 `.usmap`

- **游戏文件（只读）**：将游戏安装目录放到 `.deps/ffr/files/`（或指向安装目录的软链接）。默认挂载目录为 `$FFR_DEPS/ffr/files/FFRS/Content/Paks`（未设 `FFR_DEPS` 时为 `.deps/ffr/files/FFRS/Content/Paks`）。工具绝不修改游戏文件。
- **`.usmap` 映射文件**：`list`、`raw`、`locres`、`inventory` 不需要映射文件；`export`、`export-assets`、`export-maps` 需要通过 `--usmap <文件>` 传入。

## 路径约定与 `FFR_DEPS`

工具所有默认输入与输出目录均优先读取环境变量 `FFR_DEPS`（与下游共鸣仓库 `Final-Fantasy-Resonance-Agent` 同名、同义）；未设置 `FFR_DEPS` 时默认使用本仓库的 `.deps/`。

### 默认输出布局

设 `DEPS="${FFR_DEPS:-.deps}"`：

| 路径 | 对应子命令 | 说明 |
| --- | --- | --- |
| `$DEPS/ffr/files/FFRS/Content/Paks` | 全部（默认输入） | 游戏 IoStore / Pak 目录（只读，可用 `--paks` 覆盖） |
| `$DEPS/ffr/export` | `export` | 数据表 / 字符串表导出 JSON 与 `manifest.json` |
| `$DEPS/ffr/export/Localization` | `locres` | 多语言文本 `<lang>.json` 与 `manifest.json` |
| `$DEPS/ffr/inventory` | `inventory` | 资源索引 `registry.jsonl`、`summary.json`、`manifest.json` |
| `$DEPS/ffr/export-full` | `export-assets` | 全量非图像资源 JSON、原样拷贝的 `Config/*.ini`/`.uplugin`/`.uproject` 与 `manifest.json` |
| `$DEPS/ffr/export-full/maps` | `export-maps` | 全量关卡 `<关卡名>.json`、地图图谱 `_graph.json` 与 `manifest.json` |
| `$DEPS/ffr/db/ffr.sqlite` | `ffr-db build` | 汇总导出 JSON 与可选 `.jmap`/快照的本地 SQLite 游戏数据库 |

### 输出目录护栏

- `--out` 必须位于 `$FFR_DEPS`（或未设置时的 `.deps/`）内部，且禁止指向受保护目录（`ffr`、`ffr/files`、`r1`、`dotnet`、`nuget`、`dotnet-home`、`nuget-http` 及其子目录）。
- 非续跑命令（`export`、`locres`、`inventory`）遇到非空目录时拒绝执行；加 `--force` 时必须校验目标目录存在属于同一子命令的 `manifest.json`（带 `cue4ParseVersion`）后才清空。
- 续跑命令（`export-assets`、`export-maps`）在不加 `--force` 时自动复用同名 `manifest.json` 目录并跳过已有文件；`export-assets --force` 清空时会自动保留含独立 `manifest.json` 的其他子命令子目录（例如 `maps/`）。

## 子命令与参数

所有子命令通过 `scripts/run.sh [--lang en|zh-Hans|ja] <子命令> [参数...]` 运行：

### 1. `list` — 列出包内文件

```bash
scripts/run.sh list [--paks <目录>] [--prefix <前缀>]...
```

逐行打印匹配的虚拟路径和字节数，并在末尾打印总数。不传 `--prefix` 时默认匹配 `FFRS/Content/Datatable/` 与 `FFRS/Content/L10N/`。

### 2. `raw` — 导出单个包原始字节

```bash
scripts/run.sh raw <包路径> --out <输出文件> [--paks <目录>]
```

将指定虚拟路径的原始字节写入 `--out`（必须在 `$FFR_DEPS` / `.deps/` 下），并打印长度与 SHA-256。

### 3. `export` — 导出数据表为 JSON

```bash
scripts/run.sh export --usmap <Mappings.usmap> \
  [--out <目录>] [--prefix <前缀>]... [--paks <目录>] [--force]
```

默认前缀为 `FFRS/Content/Datatable/`、`FFRS/Content/L10N/`，默认输出到 `$DEPS/ffr/export`。每个 `.uasset` 导出为 `<out>/<包路径去扩展名>.json`（两空格缩进、UTF-8 无 BOM、LF 换行）。单个包失败记录进 `manifest.json` 的 `failures` 不中断其余包；存在失败时退出码为 1。

### 4. `locres` — 导出多语言文本

```bash
scripts/run.sh locres [--lang <语言>]... [--out <目录>] [--paks <目录>] [--force]
```

读取 `FFRS/Content/Localization/Game/<语言>/Game.locres`（使用 CUE4Parse `FTextLocalizationResource`），默认导出 `zh-Hans`、`en`、`ja` 到 `$DEPS/ffr/export/Localization/<语言>.json`（格式 `{ namespace: { key: text } }`）并写出 `manifest.json`。`--lang` 支持重复传入或逗号分隔（如 `--lang zh-Hans,en,ja`）。

### 5. `inventory` — 生成全量资源索引

```bash
scripts/run.sh inventory [--paks <目录>] [--out <目录>] [--force]
```

解析 `FFRS/AssetRegistry.bin`（`FAssetRegistryState`）并结合去重后的 VFS 挂载表，输出 `registry.jsonl`、`summary.json`、`manifest.json` 到 `$DEPS/ffr/inventory`。详见 [docs/data-inventory.md](docs/data-inventory.md)。

### 6. `export-assets` — 全量非图像资源导出

```bash
scripts/run.sh export-assets --usmap <Mappings.usmap> \
  [--out <目录>] [--prefix <前缀>]... [--jobs <并行数>] [--paks <目录>] [--force]
```

按 [docs/data-inventory.md](docs/data-inventory.md) 的分类规则并行导出 `FFRS/` 下全部非图像、非关卡、非大体积二进制资源，并原样拷贝 `Config/*.ini`、`.uplugin`、`.uproject`。支持断点续跑、对缺失 `ContainerHeader` 的补丁 IoStore 自动回退以及单文件容错。

分类查询模式（不挂载游戏包，直接打印 JSON）：

```bash
scripts/run.sh export-assets --classify DataTable,Texture2D,BlueprintGeneratedClass
```

### 7. `export-maps` — 全量关卡导出与地图图谱

```bash
scripts/run.sh export-maps --usmap <Mappings.usmap> \
  [--out <目录>] [--prefix <前缀>]... [--map-data <CDT_MapData_Prod.json>] \
  [--jobs <并行数>] [--paks <目录>] [--force]
```

扫描 `FFRS/Content/` 下全部 `.umap` 关卡，分层导出游戏类与引擎类 Actor，提取 NPC、宝箱、商店、旅店、存档点、出入口触发器、传送落点、事件触发器、机关与遇敌区域摘要，并聚合生成 `_graph.json` 地图连通图谱。详见 [docs/maps.md](docs/maps.md)。也支持 `--from-json <夹具目录>` 离线模式。

## JSON → SQLite

本仓库内置 Rust 数据层（`crates/ffr-data`）与本地数据库工具（`crates/ffr-db`），将导出的 JSON 以及可选的 `.jmap` 反射转储、真机快照统一构造成单个 SQLite 文件。设计说明见 [docs/database.md](docs/database.md)。

### 用法

```bash
FFR_DEPS=.deps cargo run --release -p ffr-db -- build \
  [--export <目录>] [--full <目录>] [--inventory <目录>] [--maps <目录>] \
  [--jmap <文件>]... [--snapshots <目录>] [--stages <阶段列表>] [--out <文件>]
```

典型完整构建（合并多份 `.jmap`，不含真机快照）：

```bash
FFR_DEPS=.deps cargo run --release -p ffr-db -- build \
  --jmap .deps/r1/<title-jmap>/<Mappings>.jmap \
  --jmap .deps/r1/<battle-jmap>/<Mappings>.jmap
```

### 输入与输出位置

- **默认输入**：全部从 `$FFR_DEPS/ffr/` 下读取（`export`、`export-full`、`inventory`、`export-full/maps`），可通过 `--export`、`--full`、`--inventory`、`--maps` 单独覆盖。
- **显式输入**：`--jmap`（可多次传入合并，冲突以后者为准）与 `--snapshots` 必须显式指定；未传时跳过对应阶段。
- **默认输出**：`$FFR_DEPS/ffr/db/ffr.sqlite`，可用 `--out <文件>` 覆盖。构建过程先写入临时文件并完成建索引、`ANALYZE`、`VACUUM`，全部成功后再原子替换目标文件。

`ffr-db` 同时提供只读查询与检查子命令：`stats`、`sql "<SELECT ...>"`、`explain "<SELECT ...>"`、`hash`、`bench`。

## 与共鸣仓库的关系

本仓库的下游是同级目录的 `../Final-Fantasy-Resonance-Agent`。只需设置 `FFR_DEPS` 指向共鸣仓库的 `.deps`，即可直接输出全套数据：

```bash
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh export \
  --usmap ../Final-Fantasy-Resonance-Agent/.deps/r1/<转储目录>/<Mappings>.usmap \
  --prefix FFRS/Content/Datatable/ --force

FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh locres --force
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh inventory --force
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh export-assets \
  --usmap ../Final-Fantasy-Resonance-Agent/.deps/r1/<转储目录>/<Mappings>.usmap --force
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh export-maps \
  --usmap ../Final-Fantasy-Resonance-Agent/.deps/r1/<转储目录>/<Mappings>.usmap --force
```

## 构建与测试

```bash
scripts/check.sh
```

脚本依次执行：

1. `dotnet format FfrExport.sln --verify-no-changes`（Linux）
2. `dotnet build FfrExport.sln -c Release`（启用 `Nullable` 与警告当错误）
3. `dotnet test FfrExport.sln -c Release --no-build`（xUnit，全部使用合成测试数据，不依赖真实游戏文件）
4. `cargo fmt --check`（Linux）
5. `cargo clippy --workspace --all-targets -- -D warnings`
6. `cargo test --workspace`（`ffr-data` 与 `ffr-db` 测试；无真实数据时自动跳过真实数据用例）

## CI 和发布

仓库通过 GitHub Actions 在 `ubuntu-latest` 与 `windows-latest` 上使用各自原生工具链执行持续集成与自动发版：

- **持续集成（`.github/workflows/ci.yml`）**：包含 `check`（含 Windows 不可写 `TEMP` 下的 GUI 自检）、`fixture-hash`（合成数据库与合成导出清单）、`compare-hash`（Linux 与 Windows 字节级一致性核对）与 `package-windows`（`main` 分支原生打包 Windows x64 压缩包并自检单文件多语言运行，保留 30 天）。
- **发版流程（`.github/workflows/release.yml`）**：更新根目录 `VERSION` 并推送匹配的 `v*` tag 后，自动在 `windows-latest` 上执行校验、打包、单文件自检并创建 GitHub Release，附带 `ffr-export-<版本>-win-x64.zip`、`SHA256SUMS` 与 `known-builds.json`（本地 Linux 交叉编译打包脚本 `scripts/package-windows.sh` 保留供本机使用）。

## Oodle 与联网说明

- **Oodle 解压**：完全使用 CUE4Parse 自带的开源纯 C# 实现 `OodleSharp`，绝不调用 `OodleHelper.Initialize` 或 `DownloadOodleDll`。
- **联网范围**：
  - **资源导出与建库完全离线**：`list`、`raw`、`export`、`locres`、`inventory`、`export-assets`、`export-maps`、`build-db` 以及 `ffr-db` 建库过程中绝不发起任何网络请求；
  - **仅一键获取 `.usmap` 会联网**：只有用户明确触发 `fetch-usmap` 或点击窗口程序 **一键获取 .usmap** 并确认，且未安装 UE4SS 且本地无有效缓存时，才会通过 HTTPS 下载固定版本的 `https://github.com/UE4SS-RE/RE-UE4SS/releases/download/v4.0.0/UE4SS_v4.0.0.zip` 并校验大小与 SHA-256。

## 许可

[MIT License](LICENSE)
