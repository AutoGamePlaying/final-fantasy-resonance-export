# 获取 `.usmap` 映射文件

本文说明《FINAL FANTASY RESONANCE》导出流程中 `.usmap` 的原理、一键获取流程、文件改动范围以及卸载方法。

## 为什么需要 `.usmap`？

《FINAL FANTASY RESONANCE》基于 Unreal Engine 5.6 构建，其打包后的 `.uasset` / `.umap` 资源采用了 UE5 的**无版本属性序列化（Unversioned Property Serialization）**格式：

- 为了减小包体与加快加载速度，打包后的资产文件里不再逐条保存属性名称与完整类型描述，只按结构体定义中的属性序号依次写入值；
- 离线解析工具（如本仓库使用的 CUE4Parse）在读取资产时，必须先知道每个 `UClass`、`UScriptStruct` 和 `UEnum` 的字段列表、序号和类型，才能把字节流还原为可读字段；
- 这份类型映射表就是 `.usmap` 文件。它**不存在于游戏安装包或可执行文件里**，只能在游戏运行期间，通过 UE4SS 从引擎内存的反射系统中转储（`DumpUSMAP()`）得到。

### `.usmap` 里包含什么？

- 仅包含引擎与游戏类的类型结构定义（类名、继承关系、属性名、属性类型与序号、枚举值名）；
- **不包含**游戏剧情文本、数值表内容、美术资源、音频、账号信息或游戏存档；
- 游戏版本更新导致数据结构变化时，需要重新转储一次对应的 `.usmap`。

## 一键获取流程（推荐，Windows）

可以通过窗口程序或命令行一键获取 `.usmap`（两者共用 `src/FfrExport/` 的同一套核心逻辑）：

- **窗口程序**：打开 `ffr-export-gui.exe`，当 `.usmap` 行未找到文件时，点击 **「一键获取 .usmap（会启动一次游戏）」**，阅读弹窗提示并确认；
- **命令行**：
  ```powershell
  ffr-export.exe fetch-usmap --game "<Steam 游戏目录>" [--timeout 300] [--yes]
  ```

### 具体步骤

1. **前置检查**：校验游戏目录有效、确认当前系统为 Windows，且 `FFRS-Win64-Shipping.exe` 未在运行（运行中会提示先关闭游戏）。
2. **检测与准备 UE4SS**：
   - 检查游戏目录的 `FFRS\Binaries\Win64\dwmapi.dll` 与 `FFRS\Binaries\Win64\ue4ss\UE4SS.dll`；
   - **若您原本已自行安装 UE4SS**（没有本工具的标记文件）：完全不动您的 `UE4SS-settings.ini`、`mods.txt` 和其他模组，只临时放入转储模组；
   - **若尚未安装 UE4SS**：优先检查本地下载缓存 `%LOCALAPPDATA%\agp\ffr-export\downloads\UE4SS_v4.0.0.zip`，未命中或校验失败时从官方 GitHub 发布页下载：
     - 版本：`RE-UE4SS v4.0.0`（MIT 许可，解压后保留 `ue4ss\LICENSE`）
     - URL：`https://github.com/UE4SS-RE/RE-UE4SS/releases/download/v4.0.0/UE4SS_v4.0.0.zip`
     - 固定字节数：`8,758,673` 字节
     - 固定 SHA-256：`251ad0d1381a241750e036f5918cbf2b5d7ec58ebc608cb2dd09d459d0c81e45`
     - 校验不一致立即拒绝；解压失败会完整回滚已写入文件；
     - 仅针对本工具安装的那份 UE4SS，自动修改 `ue4ss\UE4SS-settings.ini` 关闭外部控制台与调试 GUI 窗口（`ConsoleEnabled = 0`、`GuiConsoleEnabled = 0`、`GuiConsoleVisible = 0`）并写入稳定参数（`bUseUObjectArrayCache = false`、`MajorVersion = 5`、`MinorVersion = 6`），同时写出安装清单标记文件 `ue4ss\.installed-by-ffr-export.json`。
3. **部署临时转储模组**：
   - 从可执行文件嵌入资源写出 `FFRS\Binaries\Win64\ue4ss\Mods\FfrUsmapDump\Scripts\main.lua`；
   - 通过放置 `FfrUsmapDump\enabled.txt` 启用（UE4SS v4.0.0 原生支持 `enabled.txt`，**绝不修改 `mods.txt`**）。
4. **启动游戏并等待转储**：
   - 使用 `steam://rungameid/3259780` 通过 Steam 启动游戏；
   - 临时模组全程以 `pcall` 保护，等 UE5 引擎初始化完成并进入标题画面后，在游戏线程中执行一次只读的 `DumpUSMAP()`，随后原子写出完成标记 `ue4ss\ffr-export-usmap.done`；
   - 本工具持续轮询（每 5 秒打印进度），确认完成标记出现且 `.usmap` 的修改时间晚于启动时间。
5. **归档与校验**：
   - 计算 `.usmap` 的文件哈希（`file_sha256`）和顺序无关的规范内容哈希（`content_sha256`），复用 `build-db` 的同一套哈希代码；
   - 复制归档到 `%LOCALAPPDATA%\agp\ffr-export\usmap\FFRS-<Steam 构建号>-<哈希前8位>.usmap`；
   - 与 `known-builds.json` 中登记的 `.usmap` 哈希比对并输出结果。
6. **清理模组并安全关闭游戏**：
   - 无论成功、失败、超时还是用户取消，都会**立即移除**临时模组目录 `ue4ss\Mods\FfrUsmapDump\` 与完成标记 `ue4ss\ffr-export-usmap.done`，避免下次正常游玩时重复转储；
   - 先通过 `CloseMainWindow` 请求正常关闭游戏窗口并最多等待 15 秒，超时后结束进程；
   - **关于关闭游戏的安全性**：此时游戏停留在标题画面，模组全程只读、未进入存档也未写任何存档数据，关闭或结束游戏进程不会损坏游戏存档；
   - UE4SS 本体保留在游戏目录中供下次直接复用（下次更新游戏后无需再次下载）。

## 会改动哪些文件？

| 位置 | 改动说明 |
| --- | --- |
| `<游戏目录>\FFRS\Binaries\Win64\dwmapi.dll` | 仅在原本未安装 UE4SS 时写入（随官方 zip 解压） |
| `<游戏目录>\FFRS\Binaries\Win64\ue4ss\...` | 仅在原本未安装 UE4SS 时写入（含 `UE4SS.dll`、`UE4SS-settings.ini`、`LICENSE` 及安装清单 `.installed-by-ffr-export.json`） |
| `<游戏目录>\FFRS\Binaries\Win64\ue4ss\Mods\FfrUsmapDump\` | 临时转储模组，流程结束/失败/取消后自动删除 |
| `<游戏目录>\FFRS\Binaries\Win64\ue4ss\ffr-export-usmap.done` | 临时完成标记，流程结束后自动删除 |
| `<游戏目录>\FFRS\Binaries\Win64\ue4ss\*.usmap` | UE4SS `DumpUSMAP()` 写出的原始转储文件 |
| `%LOCALAPPDATA%\agp\ffr-export\downloads\UE4SS_v4.0.0.zip` | 官方安装包下载缓存（下次复用前会重新校验 SHA-256） |
| `%LOCALAPPDATA%\agp\ffr-export\usmap\FFRS-<构建号>-<哈希>.usmap` | 归档后的 `.usmap` 文件，供导出工具自动查找 |
| `%LOCALAPPDATA%\agp\ffr-export\logs\cli-*.log` | 命令行运行日志（最多保留最近 10 个） |

本工具**绝不修改**游戏原有的 `.pak` / `.utoc` / `.ucas` / `.exe` 文件、游戏存档以及用户已有的 `mods.txt`。

## 如何卸载 UE4SS

如果之后不再需要由本工具安装的 UE4SS，可以随时干净卸载：

- **窗口程序**：点击主窗口的 **「设置…」**，点击 **「卸载 UE4SS」**（仅当标记文件 `ue4ss\.installed-by-ffr-export.json` 存在时可用）；
- **命令行**：
  ```powershell
  ffr-export.exe uninstall-ue4ss --game "<Steam 游戏目录>"
  ```

卸载规则：
- 只删除安装标记文件里记录的那些文件及随后变空的目录；
- 已归档到 `%LOCALAPPDATA%\agp\ffr-export\usmap\` 的 `.usmap` 文件、以及非清单内的日志或用户自定义模组都不会被删除；
- 如果游戏正在运行，或游戏目录里没有本工具的标记文件（说明是您自行安装的 UE4SS），工具会拒绝卸载并说明原因。

## 手动获取 `.usmap`（备选方式）

如果不希望由本工具自动下载或启动游戏，也可以完全手动完成：

1. 从 [RE-UE4SS Releases](https://github.com/UE4SS-RE/RE-UE4SS/releases/tag/v4.0.0) 手动下载 `UE4SS_v4.0.0.zip` 并核对 SHA-256；
2. 将压缩包内的 `dwmapi.dll` 与 `ue4ss\` 解压到 `<游戏目录>\FFRS\Binaries\Win64\`；
3. 启动游戏，在 UE4SS 调试窗口的 **Dumpers** 标签页点击 **Generate .usmap file**（或通过自定义 Lua 模组调用 `DumpUSMAP()`）；
4. 转储完成后关闭游戏，`.usmap` 默认位于 `<游戏目录>\FFRS\Binaries\Win64\ue4ss\`（也可复制到 `%LOCALAPPDATA%\agp\ffr-export\usmap\`），窗口程序与控制台向导都会自动发现它。

## 常见问题排查

- **超时提示“未检测到游戏进程”**：请确认 Steam 已登录且能正常启动《FINAL FANTASY RESONANCE》，没有被 Steam 更新或首次运行弹窗卡住；
- **超时提示“游戏已启动但未收到完成标记”**：请查看 `<游戏目录>\FFRS\Binaries\Win64\ue4ss\UE4SS.log`，确认 UE4SS 是否成功加载以及首次着色器编译是否超过默认的 300 秒（可传 `--timeout 600` 重试）；
- **下载失败或校验失败**：本工具仅通过 HTTPS 访问官方 GitHub 发布页且严格校验 SHA-256；如网络受限，可手动下载 `UE4SS_v4.0.0.zip` 放到 `%LOCALAPPDATA%\agp\ffr-export\downloads\UE4SS_v4.0.0.zip`，本工具校验通过后将直接复用缓存而不再联网。
