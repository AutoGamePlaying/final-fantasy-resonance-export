# 游戏资源总索引与非图像资源导出说明

本文记录 `FfrExport` 的 `inventory` 与 `export-assets` 两个子命令的盘点结果、分类规则、全量非图像资源导出统计，以及新发现的可供 AI 玩家使用的数据与下游 `ffr-data` 接口建议。所有导出产物均留在 `$FFR_DEPS`（默认 `.deps/`）下（`ffr/inventory/` 与 `ffr/export-full/`），不进入版本库；本文仅记录结构、路径、类名与统计，不包含游戏正文内容。

> 关卡资源（`.umap`，`World` 类，共 1,545 个游戏包 / 1,546 个全包）由 `export-maps` 单独处理并导出到 `$FFR_DEPS/ffr/export-full/maps/`（默认 `.deps/ffr/export-full/maps/`），详见 [maps.md](maps.md)；本文仅在总盘点与跳过统计中列出，不重复导出。

## 一、资源包总览与 `AssetRegistry.bin` 解析结果

通过 `scripts/run.sh inventory --force` 对安装目录的 IoStore / Pak（`FFRS-Windows.{pak,ucas,utoc}` 与补丁 `FFRS-Windows_0_P.{pak,ucas,utoc}`）完成全量索引：

- **VFS 去重总览**：挂载文件共 **60,876** 条；其中 **455** 条为补丁包（`*_P.pak/.utoc`）覆写或大小写变体条目，按有效挂载版本去重后为 **60,421** 个唯一文件（总计 **52,111,303,988 字节** ≈ **48.53 GiB**）。其中 `FFRS/` 下 **57,567** 个文件（**48.26 GiB**），`Engine/` 下 **2,854** 个文件（**282.94 MiB**）。
- **`FFRS/AssetRegistry.bin` 解析情况**：
  - 文件大小 **248,756,706 字节**（237.23 MiB），`FAssetRegistryHeader` 版本为 `ExternalActorToWorldIsEditorOnly`，`bFilterEditorOnlyData = true`；
  - `FAssetRegistryState` 完整读取 **248,756,706 / 248,756,706 字节**（`unparsedBytes = 0`）；
  - 共包含 **55,130** 条 `FAssetData`，归并到 **46,635** 个唯一包（其中 8,495 个包包含主/副双对象：6,021 个 `LevelSequence + BlueprintGeneratedClass`、1,736 个 `Blueprint + BlueprintGeneratedClass`、705 个 `WidgetBlueprint + WidgetBlueprintGeneratedClass`、23 个 `World + BlueprintGeneratedClass`、9 个 `AnimBlueprint + AnimBlueprintGeneratedClass`、1 个双 `Material`）；
  - 通过 `/Game → FFRS/Content`、`/Engine → Engine/Content` 及 `.uplugin` 插件挂载映射与大小写不敏感查表，**46,635 / 46,635（100.0%）** 全部命中 VFS 中的 `.uasset`/`.umap` 文件，未匹配包为 **0**，未入注册表的 VFS 包为 **0**。
- **无法从 `AssetRegistry.bin` 解析的部分（照实记录）**：
  1. `PreallocatedDependsNodeDataBuffers` 长度为 **0**：UE5 cooked 产物在 `bFilterEditorOnlyData = true` 时剥离了注册表依赖图，因此 `inventory/registry.jsonl` 中每行的 `dependencyCount` 均记为 `null`，`summary.json` 标记 `dependencyGraphAvailable: false`。
  2. `PreallocatedPackageDataBuffers` 长度为 **0**：cooked 构建同样剥离了编辑器侧包元数据，因此 `registry.jsonl` 中的 `packageSize` 与伴随 `.ubulk` 体积 `bulkSize` 改由 VFS 实测。
  3. 补丁容器 `FFRS-Windows_0_P.utoc` 含 448 个补丁 `LevelSequence`（ChunkType=1）与 2 个着色器块（ChunkType=8），但不含可选的 `ContainerHeader`（ChunkType=6）。CUE4Parse 的 `TryLoadPackage` 会因找不到 ChunkType=6 抛 `KeyNotFoundException`；`export-assets` 已增加回退逻辑，直接从补丁 `FIoStoreEntry` 构造 `IoPackage(uasset, containerHeader: null, ...)` 读取最新补丁字节，448 个文件全部成功解析。

### 主类名分布前 20（全量 46,635 个包，共 205 个主类）

| 排名 | 主类名（`primaryClass`） | 包数量 | 总大小（含 `.ubulk`，MiB） | 分类决策 |
| ---: | --- | ---: | ---: | --- |
| 1 | `Texture2D` | 11,710 | 5,991.56 | `skip_image` |
| 2 | `MaterialInstanceConstant` | 8,525 | 491.13 | `skip_binary` |
| 3 | `LevelSequence` | 6,026 | 387.27 | `export` |
| 4 | `StaticMesh` | 5,133 | 4,532.02 | `skip_binary` |
| 5 | `DataTable` | 1,828 | 8.93 | `export` |
| 6 | `SoundAtomCue` | 1,784 | 0.87 | `skip_binary` |
| 7 | `BlueprintGeneratedClass` | 1,736 | 10.93 | `export_blueprint_cdo` |
| 8 | `NiagaraSystem` | 1,727 | 169.91 | `skip_binary` |
| 9 | `World` | 1,546 | 290.16 | `skip_level`（`export-maps`） |
| 10 | `SoundAtomCueSheet` | 1,538 | 2,072.28 | `skip_binary` |
| 11 | `StringTable` | 870 | 2.10 | `export` |
| 12 | `WidgetBlueprintGeneratedClass` | 705 | 7.83 | `export_blueprint_cdo` |
| 13 | `Ss6Project` | 671 | 20.52 | `skip_image` |
| 14 | `Material` | 489 | 19.40 | `skip_binary` |
| 15 | `MaterialFunction` | 343 | 0.13 | `skip_binary` |
| 16 | `MapBuildDataRegistry` | 310 | 31,982.49 | `skip_binary` |
| 17 | `CurveLinearColor` | 251 | 0.17 | `export` |
| 18 | `FoliageType_InstancedStaticMesh` | 175 | 0.12 | `skip_binary` |
| 19 | `FileManaMovie` | 146 | 0.06 | `skip_binary` |
| 20 | `PlatformManaMovie` | 144 | 0.06 | `skip_binary` |

> 第 21–25 位依次为 `InputAction`（130）、`AnimSequence`（113）、`InputMappingContext`（57）、`PhysicsAsset`（52）、`SkeletalMesh`（52）。

### 顶层目录分布（前 15，全量 46,635 个包）

| 目录 | 包数量 | 总大小（含 `.ubulk`，MiB） | 主要内容 |
| --- | ---: | ---: | --- |
| `FFRS/Content/Env` | 14,949 | 8,351.01 | 场景模型、贴图、材质与植被 |
| `FFRS/Content/Effect` | 8,479 | 889.72 | Niagara 特效、粒子材质与贴图 |
| `FFRS/Content/Sequencer` | 6,181 | 411.03 | 主线/支线/战斗过场 `LevelSequence` |
| `FFRS/Content/UI` | 2,883 | 317.77 | UI 控件蓝图、UI DataAsset、UI 数据表与贴图 |
| `FFRS/Content/Chara` | 2,871 | 1,642.90 | 角色/NPC/敌人的 SpriteStudio 2D 动画与贴图 |
| `FFRS/Content/Datatable` | 2,618 | 13.60 | 主数据表、复合表、字符串表与 DataAsset |
| `FFRS/Content/Sound` | 2,436 | 1,002.40 | CriWare Atom Cue / CueSheet 音频 |
| `FFRS/Content/Map` | 1,839 | 32,267.50 | 关卡 `.umap` 与烘焙光照 `MapBuildDataRegistry`（31.2 GiB） |
| `FFRS/Content/L10N` | 1,345 | 1,253.90 | 多语言 UI 贴图与语音包 |
| `FFRS/Content/BP` | 991 | 6.44 | 玩法蓝图（玩家、NPC、机关、258 个战斗 AI、镜头） |
| `FFRS/Content/Debug` | 730 | 76.91 | 调试菜单数据表、蓝图与测试关卡 |
| `FFRS/Content/Movies` | 293 | 0.15 | CriWare Mana 影片引用资源 |
| `FFRS/Content/Input` | 168 | 0.14 | Enhanced Input 动作、映射上下文与输入 DataAsset |
| `FFRS/Plugins/DebugMenuWidget` | 165 | 0.98 | 调试菜单插件的数据表、输入映射与控件 |
| `Engine/*`（合计） | 603 | 228.59 | UE 引擎内置资源与插件默认资产 |

## 二、资源分类规则与跳过统计

分类逻辑实现在 `src/FfrExport/Inventory.cs`（`ClassifyAssetClass` / `ClassifyNonPackageFile`），并在 `src/FfrExport/AssetExport.cs` 执行导出。

### 1. 导出规则（`export` 与 `export_blueprint_cdo`）

| 决策 | 类别 | 匹配类名规则 | 导出内容与过滤策略 | 全注册表包数 | 原始大小（MiB） |
| --- | --- | --- | --- | ---: | ---: |
| `export` | `datatable` | `DataTable`、`CompositeDataTable`、`StringTable` | 全量导出（含 `Datatable/` 之外的 118 张 UI/输入/调试表） | 2,722 | 13.66 |
| `export` | `curve` | `CurveTable`、`CurveFloat`、`CurveLinearColor`、`CurveVector` 等 `Curve*`（不含 LUT 贴图 `CurveLinearColorAtlas`） | 全量导出 | 259 | 0.17 |
| `export` | `data_asset` | `DataAsset`、`PrimaryDataAsset`、`PrimaryAssetLabel`、`CPP_DA_*`、`CPP_DMI_DA_*`、`DA_*`、`*DataAsset`、`CPP_DM_EventShortcutData` | 全量导出（130 个子类，覆盖输入、地图、战斗、UI、字段、任务配置） | 171 | 0.50 |
| `export` | `input` | `InputAction`、`InputMappingContext`、`PlayerMappableInputConfig`、`InputModifier*`、`InputTrigger*` | 全量导出（供输入注入） | 187 | 0.14 |
| `export` | `sequencer` | `LevelSequence` | 导出轨道、对象绑定、事件触发、子序列与对话引用；剔除冗余编译缓存 `MovieSceneCompiledData`、`MovieSceneBuiltInEasingFunction` 及 `EdGraph*`/`K2Node*` | 6,026 | 387.27 |
| `export` | `gameplay_tags` | `GameplayTag*`、`RestrictedGameplayTag*` | 全量导出（本游戏主标签表位于 `Config/DefaultGameplayTags.ini`，由原文件照拷覆盖） | 0 | 0.00 |
| `export` | `schema` | `UserDefinedEnum`、`UserDefinedStruct` | 全量导出（蓝图自定义枚举与结构体定义） | 7 | 0.01 |
| `export_blueprint_cdo` | `blueprint` | `BlueprintGeneratedClass`、`Blueprint` | 仅导出类元数据（父类/成员/函数名表）、类默认对象（`Default__*` CDO）属性与组件默认值（`SimpleConstructionScript`/`SCS_Node`/默认子对象）；剔除 `Function` 与 `EdGraph*`/`K2Node*` | 1,736 | 10.93 |
| `export_blueprint_cdo` | `widget_blueprint` | `WidgetBlueprintGeneratedClass`、`WidgetBlueprint` | 导出类元数据、CDO 默认属性与 `WidgetTree` 控件树及插槽；剔除 `Function`、`EdGraph*`/`K2Node*` 与 UI 动效 `MovieScene*` | 705 | 7.83 |

### 2. 原文件照拷（`copy_raw`）

- 匹配规则：`*/Config/**/*.ini`、`*.uplugin`、`*.uproject`。
- `FFRS/` 前缀下共 **43** 个文件（**272,338 字节** ≈ 0.26 MiB）：
  - `.ini`：**27** 个（257,907 字节，含 `DefaultGameplayTags.ini` 180 KB、`DefaultEngine.ini` 35 KB、`DefaultGame.ini` 11.6 KB、`DefaultInput.ini` 10.4 KB、`DefaultDebugMenu.ini`、`DefaultSQEXSaveGameSystem.ini`、`DefaultTalkEventUtility.ini` 及插件配置）；
  - `.uplugin`：**15** 个（11,747 字节）；
  - `.uproject`：**1** 个（`FFRS/FFRS.uproject`，2,684 字节）。

### 3. 跳过规则与体积统计（供审阅与调整）

#### 3.1 包资源跳过汇总（全注册表 46,635 个包视角）

| 跳过决策 | 细分大类 | 典型类名 | 类数 | 包数量 | 总大小（含 `.ubulk`，MiB） | 跳过原因 |
| --- | --- | --- | ---: | ---: | ---: | --- |
| `skip_image` | `image_texture` | `Texture2D`（11,710）、`TextureCube`（28）、`CurveLinearColorAtlas`（21）、`TextureRenderTarget2D`（7）、`VolumeTexture`（6）、`Texture2DArray`（3）、`ManaTexture`（2）、`MediaTexture`（1） | 8 | 11,778 | 6,330.15 | 图像/贴图与 LUT 贴图图集，按要求跳过 |
| `skip_image` | `image_spritestudio` | `Ss6Project`（671） | 1 | 671 | 20.52 | SpriteStudio 6 2D 精灵贴图集与帧动画工程（单文件 JSON 可达 600 KB+） |
| `skip_image` | `image_paper2d` | `PaperSprite`（50）、`PaperFlipbook`（1）、`PaperTerrainMaterial`（1） | 3 | 52 | 0.06 | Paper2D 精灵与贴图材质，按要求跳过 |
| `skip_binary` | `level_build_data` | `MapBuildDataRegistry`（310） | 1 | 310 | 31,982.49 | 关卡烘焙光照贴图与可见性构建缓存（占全包体积 66%） |
| `skip_binary` | `mesh_geometry` | `StaticMesh`（5,133）、`FoliageType_InstancedStaticMesh`（175）、`SkeletalMesh`（52）、`LandscapeLayerInfoObject`（26）、`GeometryCollection`（2）、`ControlRigShapeLibrary`（1） | 6 | 5,389 | 4,545.50 | 三维网格、地形绘制层与植被分布，对 AI 决策无用且体积巨大 |
| `skip_binary` | `audio` | `SoundAtomCue`（1,784）、`SoundAtomCueSheet`（1,538）、`SoundWave`（14）、`SoundSubmix`（3）、`SoundClass`、`SoundAtomConfig`、`SubmixEffect*` | 8 | 3,343 | 2,073.36 | CriWare / UE 音频数据 |
| `skip_binary` | `material` | `MaterialInstanceConstant`（8,525）、`Material`（489）、`MaterialFunction`（343）、`MaterialParameterCollection`（6）、`MaterialFunctionMaterialLayer`（2） | 5 | 9,365 | 510.67 | 着色材质与参数实例 |
| `skip_binary` | `vfx` | `NiagaraSystem`（1,727）、`VectorFieldStatic`（3）、`WaterWavesAsset`（2） | 3 | 1,732 | 172.20 | 粒子/Niagara 特效与水面波形 |
| `skip_binary` | `render_denoiser` | `NNEModelData`（6）、`NNEDenoiserAsset`（6）、`NNEDenoiserTemporalAsset`（2） | 3 | 14 | 106.10 | Intel OIDN 神经网络降噪权重模型 |
| `skip_binary` | `animation_rigging` | `AnimSequence`（113）、`PhysicsAsset`（52）、`Skeleton`（45）、`AnimBlueprintGeneratedClass`（9）、`OptimusDeformer`（5）、`Anim*CompressionSettings`（5）、`VariableFrameStrippingSettings`（1） | 8 | 230 | 4.86 | 骨骼、动画序列/动画蓝图、布娃娃物理与蒙皮 |
| `skip_binary` | `font` | `FontFace`（46）、`Font`（13） | 2 | 59 | 0.17 | 字体资产 |
| `skip_binary` | `video` | `FileManaMovie`（146）、`PlatformManaMovie`（144）、`ManaPlayer`（2） | 3 | 292 | 0.12 | CriWare Mana 影片引用 |
| `skip_level` | `level` | `World`（1,546；`FFRS/` 下 1,545） | 1 | 1,546 | 290.16 | 关卡 `.umap`，由 `export-maps` 单独导出 |

#### 3.2 非包文件跳过汇总（`FFRS/` 下 11,492 个文件）

| 扩展名 | `FFRS/` 文件数 | `FFRS/` 大小（MiB） | 说明 |
| --- | ---: | ---: | --- |
| `.ubulk` | 11,261 | 6,558.44 | 贴图/网格/关卡的伴随二进制载荷（已在上面计入所属主类总大小） |
| `.usm` | 154 | 2,170.51 | CriWare USM 过场视频流 |
| `.awb` | 1 | 572.80 | `BGM_PP.awb` CriWare 音频流 |
| `.bin` | 1 | 237.23 | `FFRS/AssetRegistry.bin`（已由 `inventory` 完整解析） |
| `.ufont` | 38 | 162.99 | 字体二进制文件 |
| `.locres` / `.locmeta` | 26 | 23.97 | 多语言编译资源（已由 `locres` 子命令单独导出） |
| `.ushaderbytecode` / `.stinfo` / `.upipelinecache` | 10 | 11.18 | 着色器字节码与 PSO 管线缓存 |
| `.upluginmanifest` | 1 | 0.17 | `FFRS/Plugins/FFRS.upluginmanifest` 插件清单 |

> 注：`Engine/` 下另有 `.res`/`.png`/`.svg`/`.ttf`/`.brk`/`.icu`/`.bin`（`GlobalShaderCache*.bin`）等引擎运行时二进制文件，不随 `FFRS/` 默认前缀导出。

### 4. 待定类清单（`undecided`）

以下 11 个小众类（全包共 **41** 个文件，合计 **67,825 字节** ≈ 66 KiB；其中 `FFRS/` 下仅 **28** 个文件，**9,881 字节**）当前归为“待定”并默认跳过，如后续需要可加入白名单：

| 类名 | 数量 | 总大小（字节） | 所在位置 | 待定理由 |
| --- | ---: | ---: | --- | --- |
| `CPP_MapPhysicalMaterial` | 27 | 9,519 | `FFRS/Content/Env/Com/PhysicsMaterial/` | 地图地表物理材质（如地毯、水晶、森林）；主要控制脚步音效/特效，待确认是否影响移动速度或触发逻辑 |
| `CPP_MovieSceneCustomClockSource_Tick` | 1 | 362 | `FFRS/Content/Sequencer/System/ClockSource/` | 过场自定义时钟对象，仅 1 个内部配置 |
| `PhysicalMaterial` | 2 | 722 | `Engine/Content/EngineMaterials/` | 引擎默认物理材质 |
| `TouchInterface` | 1 | 859 | `Engine/Content/MobileResources/HUD/` | UE 默认移动端虚拟摇杆配置（PC 版未启用） |
| `Interchange*Pipeline`（7 类） | 10 | 56,363 | `Engine/Plugins/Interchange/` | UE 编辑器资源导入管线预设（GLTF/MaterialX/Assets 等），与运行时玩法无关 |

## 三、全量导出实测结果（不含关卡）

执行命令：

```bash
scripts/run.sh export-assets \
  --usmap <Mappings.usmap> \
  --out .deps/ffr/export-full --force
```

（默认前缀 `FFRS/`；并行度默认 8；输出目录内的 `maps/` 子目录带独立 `manifest.json`，`--force` 时会自动保留不被误删。）

- **处理范围（`FFRS/` 下 46,032 个包 + 11,535 个非包文件）**：
  - **包导出成功**：**11,789 / 11,789**（`export` 9,357 + `export_blueprint_cdo` 2,432），**失败 0**；
  - **原文件照拷**：**43 / 43**（27 `.ini` + 15 `.uplugin` + 1 `.uproject`），**失败 0**；
  - **跳过包**：**34,215**（`skip_binary` 20,294 / 39,271.96 MiB，`skip_image` 12,376 / 6,252.82 MiB，`skip_level` 1,545 / 290.16 MiB）；
  - **待定跳过**：**28**（`CPP_MapPhysicalMaterial` 27 + `CPP_MovieSceneCustomClockSource_Tick` 1，共 9.65 KiB）；
- **输出体积与耗时**：
  - 总输出：**1,368,604,975 字节**（**1,305.20 MiB** ≈ **1.27 GiB**，共 11,833 个文件含 `manifest.json`）；
    - 其中 `LevelSequence` 占 1,161.58 MiB（6,026 个），`DataTable` + `CompositeDataTable` + `StringTable` 占 93.58 MiB（2,712 个），`BlueprintGeneratedClass` 占 30.28 MiB（1,733 个），`WidgetBlueprintGeneratedClass` 占 13.93 MiB（699 个），`Curve*` 占 2.51 MiB，`DataAsset`/输入/原文件合计约 3.3 MiB；
  - 全量耗时：约 **21 秒**（续跑实测跳过已有 11,341 包 + 43 原文件、补导 448 补丁包仅耗时 **2.56 秒**）。

## 四、对下游 AI 玩家有用的数据源与 `ffr-data` 接口建议

相比仅导出的 `FFRS/Content/Datatable/`（2,618 个表），全量非图像导出新增了五类对输入、执行原语、离线环境与规划有直接价值的数据源：

### 1. 输入映射体系

- **位置**：
  - `FFRS/Content/Input/`（168 个文件：`Action/` 12、`Battle/` 6、`DataAsset/` 3、`Mapping/` 4、`MetaData/` 2、`Race/` 5、`RaceUnit/` 6、`Title/` 4、`UI/` 126）；
  - `FFRS/Plugins/DebugMenuWidget/Content/Input/`（47 个文件）；
  - `FFRS/Config/DefaultInput.ini`（10.4 KB）及 `Content/UI/Option/DataAsset/DA_Option_KeySetting_InvalidKeys.json`。
- **结构与用途**：
  - `CPP_DA_MUCHA_INPUT`（27 个，如 `DA_Input_Battle_Default`）将逻辑动作名（`InputActionList`）绑定到 `InputAction` 路径，并按 `eMuchaInputDeviceType::Gamepad` / `KeyboardAndMouse` 指向对应的 `InputMappingContext`；
  - `InputMappingContext`（57 个，如 `IMC_Battle_Default_KBM`）记录每个 `InputAction` 对应的物理键位（`Key.KeyName`）、`Triggers` 与 `Modifiers`；
  - `InputAction`（130 个）记录值类型（Boolean / Axis1D / Axis2D）及 `PlayerMappableKeySettings`；
  - `Content/Input/DataAsset/DT_InputData.json` 汇总各场景输入配置。

### 2. 蓝图类默认常数与玩法配置

- **位置**：
  - 玩家与探索常数：`FFRS/Content/BP/Map/Unit/BP_MapUnit_Player.json`、`BP_MapUnit_Player_Field.json`、`FFRS/Content/Field/`（9 个文件：`DA_Airship`、`DA_Landmark*`、`DA_LandingPointMap`、`DA_FieldObject*`、`BP_DestAreaBox`）；
  - 机关与交互蓝图：`FFRS/Content/BP/Map/Gimmick/`（218 个，含 `BP_MapGimmickJumpPoint_Walk`、`BP_MapGimmickLadder`、`BP_ChatTrigger` 等）与 `BP/Event/GimmickEvent/`；
  - 战斗单位 AI 蓝图：`FFRS/Content/BP/Battle/AI/`（258 个 `BP_BtlUnitAI_*`，含各 Boss 与十二武具 AI）；
  - 全局配置 DataAsset：`CPP_DA_*`（123 个子类，覆盖 `DA_EncountSensor`、`DA_MapArea`、`DA_MapManager`、`DA_MapLandmark`、`DA_MapUnderworld`、`DA_CollectedFieldObjectList`、`DA_Nawatobi` 等）。
- **结构与用途**：
  - `Default__BP_MapUnit_Player_C` 直接给出室内/迷宫移动常数：`m_MaxWalkingSpeed = 600.0`、`m_MaxRunningSpeed = 800.0`、`m_EncountThresholdDistance = 4.0`、`CharMoveComp.MaxAcceleration = 15000.0`；
  - `Default__BP_MapUnit_Player_Field_C` 给出大世界移动常数：`m_MaxWalkingSpeed = 180.0`、`m_MaxRunningSpeed = 300.0`、`m_EncountThresholdDistance = 1.666667`；
  - 机关蓝图组件模板（`BoxComponent` / `SphereComponent` / `CapsuleComponent`）记录默认触发盒与交互点；
  - 258 个 `BP_BtlUnitAI_*` 的 CDO 与类 `FuncMap` 列表揭示各 Boss 阶段切换函数名、反击/破防回调钩子与默认参数。

### 3. 过场与事件演出结构

- **位置**：`FFRS/Content/Sequencer/`（6,026 个 `LevelSequence`：主线 `Event/Main_Event` 1,228、支线 `SubEvent` 344、传说 `LegendEvent` 347、角色 `CharaEvent` 200、自由对话 `FreeTalk` 215、召唤兽 `SummonEvent` 67、机关 `Gimmick`/`GimmickEvent` 114、十二武具 `12ArmsEvent` 62、强敌 `StrongEnemy` 16，及战斗技能/魔法/敌技/召唤演出 3,391）。
- **结构与用途**：
  - `LevelSequence.Properties.BindingReferences` 列出演出绑定的关卡路径与 Actor 名；
  - `MovieScene` 列出 `Possessables`/`Spawnables`/`PlaybackRange`/`TickResolution`，可离线计算演出帧数与时长上限；
  - `MovieSceneSubSection` / `MovieSceneCinematicShotSection` 给出主序列与分镜子序列树；
  - `MovieSceneEventTrack` / `MovieSceneEventTriggerSection` 配合目录命名（`*_Choice`、`*_Yes`、`*_No`、`*_yet`）可识别含分支选项或战斗切入的演出序列。

### 4. UI 控件结构、额外 UI 数据表与 GameplayTags

- **位置**：
  - `FFRS/Content/UI/` 下 **699** 个 `WidgetBlueprintGeneratedClass`（`Common` 206、`Battle` 168、`MainMenu` 123、`Field` 45、`Vision` 43、`Event` 34、`Option` 33、`Shop` 28、`System` 25、`Map` 16、`Quest` 16、`SaveData` 7、`INN` 5、`Talk` 5）与约 90 个 `CPP_DA_UI_*` / `CPP_DA_Btl*` / `CPP_DA_MainMenu*` DataAsset；
  - `FFRS/Content/Datatable/` 之外的 **26 张 UI 运行时数据表**，包括：
    - `UI/MainMenu/Data/DT_MainMenuScene.json`（主菜单页面路由）
    - `UI/Shop/DataList/DT_SHOP_Command.json`、`DT_SHOP_VariousShopsCommand.json`（商店菜单指令）
    - `UI/INN/Datatable/DT_INN_Command.json`（旅店菜单指令）
    - `UI/Battle/Command/Data/Behavior/CDT_BtlCmdBehavior.json`、`DT_BtlCmdUniqueActionBehavior.json`、`UI/Battle/Gameover/DT_Btl_Gameover_Command.json`（战斗指令层级与战败菜单）
    - `UI/Field/Object/Interact/Data/DT_ObjectInteractData.json`（场景可交互物体类型与提示）
    - `UI/System/ActionGuide/Data/CDT_ActionGuide.json`、`UI/System/KeyGuide/Data/DT_KeyGuideList.json`（各界面按键指南）
  - `FFRS/Config/DefaultGameplayTags.ini`：定义 **2,212** 个 GameplayTag（`UI.*` 1,133、`DebugMenu.*` 432、`Delegate.*` 418、`Sound.*` 54、`Color.*` 41、`Asset.*` 33、`Field.*` 32、`Event.*` 26、`Artifact.*` 20、`Map.*` 9、`System.*` 9）。

### 5. 下游 `ffr-data` 读取接口参考

下游共鸣仓库（`../Final-Fantasy-Resonance-Agent` 的 `crates/ffr-data/`）可按以下模块读取 `$FFR_DEPS/ffr/export-full/`：

| 模块 | 建议函数签名（示意） | 数据来源 | 用途 |
| --- | --- | --- | --- |
| `input.rs` | `load_input_catalog(export_full_root)`、`find_input_action(name)`、`list_context_mappings(context_name, device)`、`resolve_action_keys(action_name, device)` | `Content/Input/{Action,Mapping,DataAsset}/`、`Config/DefaultInput.ini` | 按场景与设备（Gamepad/KBM）查询 `InputAction` 路径、按键绑定与修饰符 |
| `blueprint.rs` | `load_blueprint_cdo(export_full_root, asset_path)`、`load_player_movement_constants(export_full_root)`、`load_gimmick_defaults(export_full_root)`、`list_battle_ai_blueprints(export_full_root)` | `Content/BP/Map/Unit/BP_MapUnit_Player*.json`、`BP/Map/Gimmick/`、`BP/Battle/AI/` | 读取步行/奔跑速度、遇敌阈值、机关触发盒默认值及 Boss AI 蓝图元数据 |
| `sequencer.rs` | `load_sequence_summary(export_full_root, seq_path)`、`list_event_sequences(export_full_root, event_id_prefix)` | `Content/Sequencer/Event/` | 解析过场时长上限、子序列链、绑定关卡/Actor 与分支事件触发点 |
| `ui.rs` | `load_gameplay_tags(export_full_root)`、`load_main_menu_scenes(export_full_root)`、`load_ui_command_tables(export_full_root)`、`load_object_interact_data(export_full_root)`、`load_widget_tree(export_full_root, widget_path)` | `Config/DefaultGameplayTags.ini`、`Content/UI/**/DT_*.json`、`Content/UI/**/WBP_*.json` | 把运行时 UI Tag/Widget 类名映射到菜单结构、指令顺序、交互提示与按键指南 |
| `field_config.rs` | `load_field_landmarks_extra(export_full_root)`、`load_airship_landing_points(export_full_root)`、`load_encount_sensor_config(export_full_root)` | `Content/Field/**/DA_*.json`、`Content/Datatable/Map/DA_*.json` | 补全飞空艇停靠映射、地标分类与大世界遇敌传感器配置 |
