# final-fantasy-resonance-export

《FINAL FANTASY RESONANCE》的资源导出工具（C# / .NET 10 / CUE4Parse）。工具以只读方式挂载游戏安装目录中的 UE5 IoStore / Pak 资源包，导出数据表、多语言文本、资源总索引、全量非图像资源以及关卡与地图图谱为 JSON。

> 本仓库仅面向《FINAL FANTASY RESONANCE》服务，不做通用化；所有游戏文件、`.usmap` 映射文件与导出产物均留在 `.deps/`（或 `$FFR_DEPS`）下，不进入版本库。

## 快速开始（Windows）

无需安装开发环境，直接下载打包好的 Windows 版本并用图形界面点几下即可导出：

1. **下载压缩包**：
   - **正式版本**：前往仓库的 **Releases** 页面下载最新 `ffr-export-<版本>-win-x64.zip`；
   - **最新开发构建**：前往仓库的 **Actions** 页面，打开最新一次 `main` 分支通过的 **CI** 记录，在底部的 **Artifacts** 中下载 `ffr-export-<版本>-<提交短号>-win-x64`（保留 30 天）。
2. **解压到同一目录**：将压缩包完整解压到一个文件夹，确保 `ffr-export-gui.exe`、`ffr-export.exe`、`ffr-db.exe` 与 `known-builds.json` 位于同一目录（不要只单独移动 `ffr-export-gui.exe`；压缩包内另附简短中文 `使用说明.txt`）。
3. **准备 `.usmap` 映射文件**：`.usmap` 无法随工具分发，需要先用 UE4SS 在《FINAL FANTASY RESONANCE》运行时转储（默认保存在游戏目录的 `FFRS\Binaries\Win64\ue4ss\` 下）。
4. **双击打开 `ffr-export-gui.exe`**，按界面顺序确认并点击 **开始导出**：
   - **自动扫描游戏**：启动时自动扫描 Steam 全部游戏库并列出名称、Steam 构建号和安装目录；若未找到可点 **手动选择游戏目录…**；
   - **自动查找 `.usmap` / 可选 `.jmap`**：依次查找 `FFRS\Binaries\Win64\ue4ss\*.usmap` 与 `FFRS\Binaries\Win64\*.usmap` 并选中最新一份；勾选 **包含反射数据（.jmap，可选）** 后会在同样位置自动列出 `.jmap`（默认不勾选）；
   - **输出位置**：默认保存到 `%USERPROFILE%\Documents\FF-Export\ffr-<Steam 构建号>\`，下次启动会自动记住上次的输出根目录与勾选状态（保存在 `%APPDATA%\ffr-export\settings.json`，不写注册表）；
   - **完成与核对**：导出时界面实时显示进度条与日志，可随时点 **取消** 结束后台进程树；完成后直接显示数据库路径、文件哈希、内容哈希与 `✓` / `✗` 比对结论，并提供 **打开输出文件夹** 与 **复制哈希** 按钮。

> 已知版本表（`known-builds.json`）同时登记了两种输入组合：“只有 `.usmap`”和“`.usmap` 加两份 `.jmap`”，因此勾选或不勾选 `.jmap` 都可能与登记版本一致。

窗口界面示意：

```text
+------------------------------------------------------------------------------+
| 《FINAL FANTASY RESONANCE》数据导出工具                                      |
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
|   输出目录：[ %USERPROFILE%\Documents\FF-Export\ffr-24250053    ] [ 浏览… ]  |
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
|   数据库路径：[ ...\FF-Export\ffr-24250053\ffr.sqlite       ] [打开输出文件夹]|
|   文件哈希：  [ b0a228269588...                             ] [  复制哈希   ]|
|   内容哈希：  [ 28374329602c...                             ]                |
+------------------------------------------------------------------------------+
```

### 控制台向导与命令行（Windows 可选）

如果偏好终端操作或在脚本中自动化运行，也可以直接使用同目录下的 `ffr-export.exe`：

- **交互式控制台向导**：双击运行不带参数的 `ffr-export.exe`，按中文提示选择游戏、粘贴 `.usmap`/`.jmap` 与输出目录（默认 `output/ffr-<Steam 构建号>/`）即可完成导出与建库。
- **命令行一键建库**：直接带参数调用 `build-db` 子命令（窗口程序后台调用的也是同一条命令）：

```powershell
ffr-export.exe build-db --game "<Steam 游戏目录>" --usmap "<Mappings.usmap>" --out "<输出目录>"
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
- **`.usmap` 映射文件**：`list`、`raw`、`locres`、`inventory` 不需要映射文件；`export`、`export-assets`、`export-maps` 需要从游戏进程中用 UE4SS 导出的 `.usmap`，运行时通过 `--usmap <文件>` 传入。

## 路径约定与 `FFR_DEPS`

工具所有默认输入与输出目录均优先读取环境变量 `FFR_DEPS`（与下游共鸣仓库 `Final-Fantasy-Resonance-Agent` 的 Rust 数据层 `ffr-data` 同名、同义）；未设置 `FFR_DEPS` 时默认使用本仓库的 `.deps/`。

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

- `--out` 必须位于 `$FFR_DEPS`（或未设置时的 `.deps/`）内部，且禁止指向受保护目录（`ffr` 本身、`ffr/files`、`r1`、`dotnet`、`nuget`、`dotnet-home`、`nuget-http` 及其子目录）。
- 非续跑命令（`export`、`locres`、`inventory`）遇到非空目录时拒绝执行；加 `--force` 时必须校验目标目录存在属于同一子命令的 `manifest.json`（带 `cue4ParseVersion`）后才清空。
- 续跑命令（`export-assets`、`export-maps`）在不加 `--force` 时自动复用同名 `manifest.json` 目录并跳过已有文件；`export-assets --force` 清空时会自动保留含独立 `manifest.json` 的其他子命令子目录（例如 `maps/`）。

## 子命令与参数

所有子命令通过 `scripts/run.sh <子命令> [参数...]` 运行：

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

默认前缀为 `FFRS/Content/Datatable/`、`FFRS/Content/L10N/`，默认输出到 `$DEPS/ffr/export`。每个 `.uasset` 导出为 `<out>/<包路径去扩展名>.json`（两空格缩进、UTF-8 无 BOM）。单个包失败记录进 `manifest.json` 的 `failures` 不中断其余包；存在失败时退出码为 1。

### 4. `locres` — 导出多语言文本

```bash
scripts/run.sh locres [--lang <语言>]... [--out <目录>] [--paks <目录>] [--force]
```

读取 `FFRS/Content/Localization/Game/<语言>/Game.locres`（使用 CUE4Parse `FTextLocalizationResource`），默认导出 `zh-Hans`、`en`、`ja` 到 `$DEPS/ffr/export/Localization/<语言>.json`（格式 `{ namespace: { key: text } }`）并写出 `manifest.json`。`--lang` 支持重复传入或逗号分隔（如 `--lang zh-Hans,en,ja`）。

### 5. `inventory` — 生成全量资源索引

```bash
scripts/run.sh inventory [--paks <目录>] [--out <目录>] [--force]
```

解析 `FFRS/AssetRegistry.bin`（`FAssetRegistryState`）并结合去重后的 VFS 挂载表（按补丁包生效版本去重并计入伴随 `.ubulk`/`.uptnl` 体积），输出 `registry.jsonl`、`summary.json`、`manifest.json` 到 `$DEPS/ffr/inventory`。详细规则与统计见 [docs/data-inventory.md](docs/data-inventory.md)。

### 6. `export-assets` — 全量非图像资源导出

```bash
scripts/run.sh export-assets --usmap <Mappings.usmap> \
  [--out <目录>] [--prefix <前缀>]... [--jobs <并行数>] [--paks <目录>] [--force]
```

按 [docs/data-inventory.md](docs/data-inventory.md) 的分类规则并行导出 `FFRS/` 下全部非图像、非关卡、非大体积二进制资源（数据表、字符串表、曲线、DataAsset、Enhanced Input、过场 `LevelSequence`、用户自定义枚举/结构体、蓝图与控件蓝图 CDO/组件默认值/`WidgetTree`），并原样拷贝 `Config/*.ini`、`.uplugin`、`.uproject`。支持断点续跑、对缺失 `ContainerHeader` 的补丁 IoStore 自动回退以及单文件容错。

另提供分类查询模式（不挂载游戏包，直接打印 JSON）：

```bash
scripts/run.sh export-assets --classify DataTable,Texture2D,BlueprintGeneratedClass
```

### 7. `export-maps` — 全量关卡导出与地图图谱

```bash
scripts/run.sh export-maps --usmap <Mappings.usmap> \
  [--out <目录>] [--prefix <前缀>]... [--map-data <CDT_MapData_Prod.json>] \
  [--jobs <并行数>] [--paks <目录>] [--force]
```

扫描 `FFRS/Content/` 下全部 `.umap` 关卡，分层导出游戏类与引擎类 Actor，提取 NPC、宝箱、商店、旅店、存档点、出入口触发器、传送落点、事件触发器、机关与遇敌区域摘要，并聚合生成 `_graph.json` 地图连通图谱。详细格式与统计见 [docs/maps.md](docs/maps.md)。

也支持 `--from-json <夹具目录>` 离线模式（供测试与合成夹具验证摘要提取和图谱聚合，不需要游戏包与 `.usmap`）。

## JSON → SQLite

本仓库内置 Rust 数据层（`crates/ffr-data`）与本地数据库工具（`crates/ffr-db`），将前面导出的 JSON（数据表、多语言文本、资源索引、全量非图像资源、关卡图谱）以及可选的 `.jmap` 反射转储、真机快照统一构造成单个 SQLite 文件。表结构、索引与查询 API 设计见 [docs/database.md](docs/database.md)。

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
- **显式输入**：`--jmap`（可多次传入合并，冲突以后者为准）与 `--snapshots` 必须显式指定；未传时跳过对应阶段。部署到游戏机的库不带 `--snapshots`。
- **默认输出**：`$FFR_DEPS/ffr/db/ffr.sqlite`（例如 `FFR_DEPS=.deps` 时为 `.deps/ffr/db/ffr.sqlite`），可用 `--out <文件>` 覆盖。构建过程先写入临时文件并完成建索引、`ANALYZE`、`VACUUM`，全部成功后再原子替换目标文件。

`ffr-db` 同时提供只读查询与检查子命令：`stats`（表行数与文件大小）、`sql "<SELECT ...>"`、`explain "<SELECT ...>"`、`bench`。

## 与共鸣仓库的关系

本仓库的下游是同级目录的 `../Final-Fantasy-Resonance-Agent`（其中的 `ffr-module` 直接依赖本仓库的 `ffr-data` / `ffr-db` 读取 API 以及构建出的 SQLite 数据库）。

只需设置 `FFR_DEPS` 指向共鸣仓库的 `.deps`，即可直接向共鸣仓库输出全套数据：

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

1. `dotnet format FfrExport.sln --verify-no-changes`
2. `dotnet build FfrExport.sln -c Release`（启用 `Nullable` 与警告当错误）
3. `dotnet test FfrExport.sln -c Release --no-build`（xUnit，全部使用合成测试数据，不依赖真实游戏文件）
4. `cargo fmt --check`
5. `cargo clippy --workspace --all-targets -- -D warnings`
6. `cargo test --workspace`（`ffr-data` 与 `ffr-db` 单元/集成测试；真实数据测试通过 `FFR_DEPS` 或 `.deps` 发现，无数据时自动跳过）

## CI 和发布

仓库通过 GitHub Actions 在 `ubuntu-latest` 与 `windows-latest` 上使用各自原生工具链执行持续集成与自动发版。

### 持续集成（`.github/workflows/ci.yml`）

每次向任意分支 `push` 或提交 `pull_request` 时触发（默认 `contents: read` 权限），包含四个 job：

1. **`check`**（`ubuntu-latest` + `windows-latest`）：安装 .NET 10 SDK 与 Rust stable，缓存 NuGet 与 Cargo 后执行 `scripts/check.sh`（Linux 上额外跑 `dotnet format --verify-no-changes` 与 `cargo fmt --check`，两个平台均执行 `dotnet build -c Release`、`dotnet test`、`cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo test --workspace`；CI 上无 `.deps/`，真实数据测试自动跳过）。
2. **`fixture-hash`**（`ubuntu-latest` + `windows-latest`）：执行 `scripts/fixture-db.sh`，汇总 `tests/fixtures/rust/db/` 下全部区域的合成假数据运行一次完整 `ffr-db build`，再运行 `ffr-db hash --json` 并将结果上传为 artifact。
3. **`compare-hash`**（依赖两个 `fixture-hash`）：下载 Linux 与 Windows 两份哈希 JSON，比对 `file_sha256`、`content_sha256` 以及每张普通表的行数与哈希；任意一项不一致即失败并打印存在差异的表。
4. **`package-windows`**（仅在 `push` 到 `main` 且 `check` 通过后运行，`windows-latest`）：执行 `scripts/package-windows.ps1` 原生打包 Windows x64 压缩包，并上传为名为 `ffr-export-<版本>-<提交短号>-win-x64` 的 artifact（保留 30 天）。

### 跨平台哈希核对的意义

真实游戏文件与导出数据不会上传到公开仓库或 CI。通过同一套合成假数据在 Linux 与 Windows 上各自原生编译 `ffr-db` 并建库，强制要求 `file_sha256`（整个 SQLite 文件字节级一致）、`content_sha256` 以及全部表哈希完全相同，可以在每次改动时持续验证建库流程没有引入与操作系统、路径分隔符、线程调度或换行符相关的非确定性差异。

### 发版流程（`.github/workflows/release.yml`）

发版由 `v*` tag 自动触发，流程如下：

1. 更新仓库根目录的 `VERSION` 文件（例如 `0.1.0`）并提交到目标分支；
2. 创建与 `VERSION` 一致的 tag 并推送：

```bash
git tag v0.1.0
git push origin v0.1.0
```

推送 tag 后，`release.yml` 会在 `windows-latest` 上校验 tag 去掉 `v` 后与 `VERSION` 一致，先完整运行 `scripts/check.sh`，再运行 `scripts/package-windows.ps1` 原生打包，最后使用 `GITHUB_TOKEN` 创建标题为 tag 名的 GitHub Release，并上传 `ffr-export-<版本>-win-x64.zip`、`SHA256SUMS` 与 `known-builds.json`（本地 Linux 交叉编译打包脚本 `scripts/package-windows.sh` 保留供本机使用）。

## Oodle 与联网说明

Oodle 解压完全使用 CUE4Parse 自带的开源纯 C# 实现 `OodleSharp`。工具绝不调用 `OodleHelper.Initialize` 或 `DownloadOodleDll` 下载专有 Oodle 动态库，在资源读取、解码和导出过程中不发起任何网络请求。

## 许可

[MIT License](LICENSE)
