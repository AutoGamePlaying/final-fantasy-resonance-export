# 关卡数据（`export-maps`）

本文记录通过 `scripts/run.sh export-maps` 从游戏包内全部 `.umap` 关卡提取出的 JSON 结构、各子关卡类型的作用与地图图谱格式。导出文件全部留在 `$FFR_DEPS/ffr/export-full/maps/`（未设置 `FFR_DEPS` 时为 `.deps/ffr/export-full/maps/`），不进版本库；本文只记录结构与统计，不含游戏正文内容。

## 一、导出命令、目录布局与过滤策略

```bash
scripts/run.sh export-maps --usmap <Mappings.usmap> \
  [--out .deps/ffr/export-full/maps] [--prefix FFRS/Content/] [--paks <目录>] [--jobs <并发数>] [--force]
```

- **覆盖范围与产物**：默认扫描 `FFRS/Content/` 下全部 **1,545** 个 `.umap`（含 `FFRS/Content/Map/` 1,534 个与 `FFRS/Content/Debug/` 11 个），写出：
  - `<关卡名>.json`（1,545 个，合计约 124 MB；若出现极少数跨目录重名子关卡，如复制至其他目录的 `*_KD`/`*_SD`，自动以 `<目录段>__<关卡名>.json` 消歧避免覆盖）；
  - `_graph.json`（约 1.1 MB）：汇总全部地图连通边、落点、商店映射与对象计数；
  - `manifest.json`：记录 `cue4ParseVersion`、`command: "export-maps"`、`usmapSha256`、起止时间、耗时、总计/新导出/跳过/失败数及失败列表。
- **并发与续跑**：默认并行处理；已存在且可解析的 `<关卡名>.json` 会直接跳过并复用于重建 `_graph.json`（原子写临时文件再替换，避免中断留下半成品）；加 `--force` 仅在校验目录内 `manifest.json` 确属 `export-maps` 后清空重跑。
- **主关卡归属与 `mapId` 反查**：先扫描全部 `*_PL.umap` 的 `UWorld.StreamingLevels`（`LevelStreamingAlwaysLoaded` / `LevelStreamingDynamic`）与 `CDT_MapData_Prod.mainProgressLoadLevelList`，再按目录层级兜底推出每个子关卡所属主关卡 `persistentLevel`，并用主关卡短名精确匹配 `CDT_MapData_Prod.Level` 取得 `mapId`（186 / 186 张正式地图 100% 命中）。
- **游戏类与引擎类分层导出**：
  - **游戏类**（`/Script/mucha.*` 与 `/Game/*` 蓝图）：完整保留 Actor 的非默认 `properties`，并挂出归属该 Actor 的 `subobjects`（游戏子对象/组件保留全部非默认属性；引擎形状/网格组件仅保留变换、碰撞盒尺寸 `boxExtent`/`sphereRadius`/`capsule*` 与外部资源引用，不展开 `Model`/`BodySetup` 几何）；
  - **引擎类**（灯光、静态网格、贴花、粒子、声音、导航体等）：只记录 `name`、`label`、`class`、世界坐标变换（`location`/`rotation`/`scale`，支持 `AttachParent` 组合）与去重后的外部资源路径 `assetRefs`，不展开顶点、碰撞体、植被实例或贴图流数据。

## 二、子关卡类型与统计（共 1,545 个 `.umap`）

| 后缀类型 | 文件数 | 已映射 `mapId` | Actor 总数（游戏 / 引擎） | JSON 体积 | 作用与主要对象 |
| --- | ---: | ---: | ---: | ---: | --- |
| `LT` | 305 | 297 | 2,868（347 / 2,521） | 1.67 MB | 光照与后期调色（`SkyLight`、`PointLight`、`SphereReflectionCapture`、`PostProcessVolume`）；常按剧情进度配多套 `01_LT`/`02_LT` |
| `BG` | 287 | 279 | 130,790（6,592 / 124,198） | 88.97 MB | 地形、建筑与场景美术（含战斗舞台 `*_BG0`）；主要为 `StaticMeshActor`、灯光与 `NiagaraActor` |
| `PL` | 189 | 186 | 630（239 / 391） | 0.82 MB | 持久主关卡；持有 `CPP_MuchaWorldSetting`、`RecastNavMesh` 与全部子关卡的 `UWorld.StreamingLevels` 清单 |
| `GD` | 176 | 176 | 4,375（4,314 / 61） | 11.95 MB | 玩法与关卡设计核心：出入口触发器（`BP_MapTransitionTrigger`）、传送落点（`BP_MapTransitionPoint`）、宝箱、存档点、阻挡/机关、遇敌区域与大地图地标/降落点 |
| `EV` | 166 | 166 | 2,632（2,418 / 214） | 15.66 MB | 事件与角色摆放：NPC（`BP_MapUnit_NPC`，含 `ShopEntry.ShopID`、旅店标记与对话事件）、装饰单位（`BP_MapUnit_Object*`）、事件/闲聊触发器、寻路点 |
| `NAV` | 156 | 156 | 2,771（851 / 1,920） | 2.48 MB | 导航网格包围体与分区定义（`NavMeshBoundsVolume`、`CPP_Map_NavMeshBoundsVolume`、`NavModifierVolume`） |
| `KD` | 143 | 143 | 535（510 / 25） | 1.48 MB | 镜头控制与地图规则区域（`BP_MapCamera_KeepBox`、`BP_MapCameraStopTrackingAtEdge`、`BP_Map_CannotSaveArea`、`BP_MapClimate`） |
| `SD` | 112 | 112 | 399（221 / 178） | 0.55 MB | 环境音效与室内混响（`AtomSound`、`BP_MapSound_InDoorReverberation`、`BP_MapAmbientSound`） |
| `OTHER` | 6 | 0 | 10（10 / 0） | 0.01 MB | 启动、标题与调试入口图（`MUCHA_Boot`/`Notify`/`Activity`/`Title` 及调试场景） |
| `MOCK` | 2 | 2 | 6（2 / 4） | <0.01 MB | 少量原型/占位光照子关卡 |
| `FG` | 2 | 2 | 2（2 / 0） | <0.01 MB | 世界地图区域雾效/前景占位子关卡（当前仅含 `CPP_MuchaWorldSetting`） |
| `COL` | 1 | 1 | 2（1 / 1） | <0.01 MB | 世界地图碰撞占位子关卡（`Wld_COL`） |

> 未映射到 `mapId` 的 25 个关卡均为非正式探索地图：`Debug/`（11 个）、未挂到主关卡的 `Btl/`（8 个）、`Boot/` 与 `Title/`（4 个）以及 2 个无对应 `_PL` 的历史遗留子关卡。

## 三、单关卡 JSON 结构（`<关卡名>.json`）

每个关卡文件为顶层 JSON 对象（`schemaVersion: 1`），含三段：

- **`level`**：`name`、`outputFile`、`path`、`packagePath`、`sublevelType`、`persistentLevel`、`persistentLevels[]`、`mapId`、`mapIds[]`、`mapRowKey`、`mapType`、`streamingLevels[{name, worldAsset, streamingClass}]`、`actorCount`、`gameActorCount`、`engineActorCount`。
- **`actors[]`**：逐个记录 `ULevel.Actors` 中的实例（`name`、`label`、`class`、`isGameClass`、`location`、`rotation`、`scale`；游戏类附 `properties` 与 `subobjects[]`，引擎类仅附 `assetRefs[]`）。
- **`summary`**：从游戏类 Actor 提取的 10 类重点对象清单，便于离线检索与图谱构建：
  1. `mapUnits[]`：全部 `CPP_MapUnit*`/`BP_MapUnit*` 或带 `m_MapUnitId` 的单位，按 `kind` 区分 `npc` / `object` / `vehicle` / `player` / `unit`，记录 `mapUnitId`、`uniqueId`、`direction`、`interactType`、`visibleCondition`、`shopId`、`innId`、`events[]`、`location`。
  2. `treasureBoxes[]`：全部宝箱变体（`TreasureBox*` / 含 `m_TreasureBoxData`），记录 `uniqueId`、`chestId`、`mapId`、`itemId`、`itemNum`、`money`、`isTrap`、`battleGroupId`、`battleFinishEventId`、`location`。
  3. `eventTriggers[]`：`EventTrigger`、`InteractEventTrigger`、`ChatTrigger`、`TalkPoint` 及含 `m_InteractEventId` 的对象，记录 `uniqueId`、`visibleCondition`、`flagCondition`、`interactEventId`、`events[]`、`boxExtent`、`location`。
  4. `transitions[]`：`MapTransitionTrigger` 出入口触发器，兼容直接字段（`m_MapId`/`m_PointID`/`m_IsSameMapTrasition`）与条件分支数组 `mTransitionDataList[]`，统一展开为 `targets[{condition, targetMapId, targetPointId, isEventOnly, events, afterTransitionEvents}]` 并附 `boxExtent` 与 `location`。
  5. `transitionPoints[]`：`MapTransitionPoint` 进图/传送落点（`pointId` 默认 `0`、`direction`、`pointName`、`location`）。
  6. `savePoints[]`：`SavePoint` 存档点，记录 `uniqueId`、`savePointType`、`isTeleport`、`transitionMapId`（`m_TranditionMapId`）、`transitionPointId`（`m_TranditionPoint`）、`parentMapId`/`parentUniqueId` 及解析后的传送目标 `targetMapId`/`targetPointId`。
  7. `shops[]`：带非负 `ShopEntry.ShopID` 的对象，标注 `isVariousShop`（`ShopID >= 10000` 对应 `DT_VariousShopsList.ID`，其余对应 `DT_ShopList.ShopID`）、`interactType`、`visibleCondition`、`location`。
  8. `inns[]`：带 `InnEntry` 或 `m_MapInteractType == eMapInteractType::Inn` 的旅店 NPC（未显式序列化 `InnID` 时按表默认 `0`）。
  9. `gimmicks[]`：除宝箱、存档点与事件触发器外的其余 `MapGimmick*` 机关，按 `category` 归类（`Blocking`、`FieldItem`、`NoEntry`、`Door`、`Ladder`、`Elevator`、`Switch`、`Lamp`、`Raft`、`MerchantShip`、`Warp`、`JumpPoint`、`VisionSmallShrine` 等）。
  10. `encounterZones[]`：`MapEncount*` 遇敌区域（`Box`、`Pile`、`Sphere`、`OverwriteBox`、`OverrideBattleStageBox`），记录 `encountDatatable`、`terrainDatatables[]`、`priority`、`battleStageId`、`overwriteRatio`、`isResetDistance`、`boxExtent`/`sphereRadius`、`location`。

## 四、地图图谱格式与全量规模（`_graph.json`）

`_graph.json` 将全部已映射子关卡按 `mapId` 聚合为离线寻路与规划骨架（`schemaVersion: 1`）：

- **`stats`**：全局统计（关卡数、各子关卡类型计数、映射地图数与各类对象总量）。
- **`maps[<mapId>]`**：每张地图一个节点，包含：
  - 元信息：`mapId`、`mapRowKey`、`mapType`、`persistentLevel`、`sublevels[{name, type, outputFile}]`；
  - 计数：`counts.{npcs, mapUnits, treasureBoxes, shops, inns, savePoints, eventTriggers, gimmicks, encounterZones, transitions, transitionPoints}`；
  - 有向边：`exits[]`，每条记录 `kind`（`transition_trigger` 或 `savepoint_teleport`）、`sourceLevel`、`actorName`、`uniqueId`、`targetMapId`、`targetPointId`、`isSameMap`、`isAutoTransition`、`isEventOnly`、`condition`、`location`；
  - 落点与关键设施明细：`transitionPoints[]`、`shops[]`、`savePoints[]`、`treasureBoxes[]`。
- **`shopsByMap` / `shopToMaps`**：`mapId → ShopID[]` 与 `ShopID → mapId[]` 双向索引，补齐了 `Datatable/` 静态表缺失的“地图 → 商店”映射。
- **`unmappedLevels[]`**：无对应 `mapId` 的非探索关卡列表。
- **全量统计概览**：覆盖全部 **186** 张地图，汇总出 **791** 条有向出入口边（跨图 374、同图切区/传送 407，其中含存档点往返传送边 50 条）、**824** 个传送落点、**807** 个 NPC（地图单位总计 1,482）、**251** 个宝箱实例、**88** 个存档点、**69** 个商店 NPC（62 个唯一 `ShopID`，分布在 24 个 `*_EV` 关卡 / 23 张地图，命中 19/20 个 `DT_VariousShopsList` 分组，展开后覆盖 89/105 张 `DT_ShopList` 货单）、**35** 个旅店 NPC、**336** 个事件触发器、**1,493** 个其余机关与 **344** 个遇敌区域。
