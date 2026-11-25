# 本地游戏数据库（`ffr-db`）

把从游戏包里导出的静态数据（数据表、多语言文本、资源索引、关卡、输入、蓝图、界面、过场、反射数据）和可选的真机快照，统一装进一个本地 SQLite 文件。代码里每类数据都有一个 Rust 对象表示，对象之间的关系用关系表连起来，读取函数都走索引。

数据库文件和导出数据一样**不进仓库**；测试只用自编的假数据。

## 1. 分层

| 层 | 位置 | 职责 |
| --- | --- | --- |
| 对象 | `crates/ffr-data` | 每类数据的 Rust 结构体（`Serialize` + `Deserialize`），以及从导出 JSON 解析出对象的函数。已有：战斗（`Unit`、`Skill`、`BattleGroup`……）、世界（`MapInfo`、`Item`、`Shop`、`Quest`……）、剧情、旗标。新增的资源类对象放在 `ffr_data::assets::*` |
| 数据库 | `crates/ffr-db` | 表结构（`schema/*.sql`）、构建（导出 → 数据库）、读取 API（`GameDb`）、命令行 `ffr-db` |
| 使用者 | `crates/ffr-module` 等 | 只通过 `GameDb` 读数据；给模型看什么、藏什么（公平规则，见 `docs/fairness.md`）仍由工具层决定，数据库里什么都有 |

## 2. 存储方式：索引列 + 对象文档 + 关系表

每类对象一张表：

- **索引列**：编号、名字的文本键、解析好的中文名、分类等常用于查找和筛选的字段，各自建索引；
- **`doc` 列**：整个 Rust 对象的 JSON（serde 序列化）。读出来直接反序列化成对象，不用把每个嵌套列表都拆成表；
- **关系表**：对象之间的引用（单位 → 招式、商店 → 道具、地图 → 宝箱、关卡 → Actor……）单独成表，两端都建索引，正查、反查都快。

另有一层**原始数据表**：全部 2618 个数据表的每一行原样存进 `dt_row`，没有专门对象的表也能按表名 + 行名直接取，或者用 SQLite 的 JSON 函数临时查询。

## 3. 区域和表

| 区域 | 来源 | 主要的表 |
| --- | --- | --- |
| 构建信息 | 构建器 | `meta`（版本、游戏构建号、构建时间）、`source`（每个来源的路径和 manifest 哈希）、`stage_report`（每段的行数、耗时、状态）、`build_issue`（悬空引用等问题，照实记录，不静默丢弃） |
| 文本 | `Localization/<lang>.json`、字符串表 | `text`（命名空间、键、语言、文本）、`text_fts`（FTS5 全文索引，trigram 分词，中日文可以按子串搜）、`string_table_entry` |
| 原始数据表 | `FFRS/Content/Datatable/` | `dt_table`、`dt_row` |
| 战斗 | 数据表 | `unit`、`unit_resistance`、`skill`、`unit_skill`、`command_skill`、`battle_group`、`battle_group_member`、`encounter`、`unit_ai`、`enemy_chapter` |
| 世界 | 数据表 | `map`、`island`、`region`、`map_release_condition`、`save_point`、`teleport_point`、`map_link`、`treasure_box`、`item`、`shop`、`shop_item`、`inn`、`quest`、`quest_prize`、`chapter`、`main_scenario`、`flag_condition`、`playable_unit` |
| 资源索引 | `AssetRegistry.bin`（`inventory/registry.jsonl`）、全量导出、`Config/*.ini` | `asset`、`asset_tag`、`asset_ref`（资源之间的引用，从导出 JSON 里收集）、`config_entry` |
| 反射 | `.jmap` | `ue_class`（类、结构体、枚举）、`ue_property`、`ue_function`、`ue_enum_value` |
| 输入、蓝图、界面 | 全量导出 | `input_action`、`input_context`、`input_mapping`、`input_set`；`blueprint`（类默认值里改过的属性）、`blueprint_component`；`widget_node`（界面控件树） |
| 关卡 | [final-fantasy-resonance-export](https://github.com/AutoGamePlaying/final-fantasy-resonance-export) `export-maps` 的关卡文件和 `_graph.json` | `level`、`actor`、`actor_rtree`（R*Tree 空间索引）、`level_object`（NPC、宝箱、事件、出入口、存档点、商店、机关、遭遇区域，带指向对应数据的编号）、`map_edge` |
| 过场 | 全量导出里的 `LevelSequence` | `sequence`、`sequence_binding` |
| 真机快照（可选） | `.deps/r1/*` | `snapshot`、`snapshot_object`、`snapshot_widget`、`snapshot_text`、`marker` |
| 跨区域连接 | 前面各区域的表和导出 | `talk_event`、`talk_event_next`、`talk_line`（对话事件 → 过场、台词）、`level_object_talk`、`map_unit`（`DT_MapUnitAsset`）、`class_ref`（各表出现的类名统一后连到反射或蓝图） |

跨区域的连接：

- 所有名字列都存文本键，连到 `text`；
- 关卡的 `map_id` 连到 `map`，`level_object` 里的商店、宝箱、地图单位编号分别连到 `shop`、`treasure_box`、`playable_unit`/`unit`；
- `blueprint.parent_class`、`actor.class`、`snapshot_object.class` 连到 `ue_class`；
- 关卡、蓝图、过场、输入资产都连到 `asset`；
- 对话链：关卡事件触发的 `interactEventId` → `talk_event`（`CDT_TalkEventData_Prod`）→ 过场 `sequence` 和对话表 → `talk_line` → `text`。

暂不入库的数据（原始数据仍在 `.deps/` 下，需要时再导）：

- 资源注册表的 `ActorsMetaData` 标签：每个关卡对象的描述（对象路径、类、二进制描述），内容和关卡导出重复，正文约 120 MB。原始数据在 `inventory/registry.jsonl`；
- 真机快照默认不导入，只有构建时给了 `--snapshots` 才导入。部署到游戏机的库不带快照。

## 4. 索引

- 每张表的主键；关系表两端各建索引；名字列、分类列建索引；
- `text_fts`：FTS5 trigram，用来按子串搜中、英、日文本；
- `actor_rtree`：R*Tree，用来查“某关卡里某个坐标附近有什么”；
- 原始数据表在 `(table_id, row_name)` 上有主键索引；以后某个 JSON 字段查得多，再加表达式索引；
- **每个读取函数都有测试，用 `EXPLAIN QUERY PLAN` 确认走了索引**，不允许在大表上全表扫描。

## 5. 构建

```
ffr-db build [--export DIR] [--full DIR] [--inventory DIR] [--maps DIR]
             [--jmap FILE]... [--snapshots DIR] [--stages a,b,...] [--out FILE]
```

- 默认输入都在 `$FFR_DEPS/ffr/` 下，默认输出是 `$FFR_DEPS/ffr/db/ffr.sqlite`；`.jmap` 和快照目录要显式给出，`--jmap` 可以给多次（合并，冲突以后给的为准）；
- 阶段顺序：`text`、`datatable`、`battle`、`world`、`assets`、`reflection`、`input_ui`、`levels`、`sequences`、`snapshots`、`links`；每段一个事务；
- 某段的输入不存在时，这一段跳过，并在 `stage_report` 里写明原因；
- 先写临时文件，全部成功后再替换目标文件；数据装完再建索引，最后 `ANALYZE`、`VACUUM`；
- 悬空引用（比如单位引用了不存在的招式）写进 `build_issue`，构建结束时汇总打印。

## 6. 读取

```rust
let db = GameDb::open(path)?;          // 只读打开，检查表结构版本
let bat = db.unit(2)?;                 // Option<Unit>
let skills = db.skills_of_unit(2)?;    // Vec<Skill>
let src = db.item_sources(item_id)?;   // 宝箱、商店、任务奖励……
let hits = db.search_text("钢蝠", "zh-Hans", 20)?;
let near = db.actors_near(level, pos, 1500.0)?;
```

- 只读连接：`query_only`、`mmap`、较大的页缓存；语句用 `prepare_cached` 复用；
- `GameDb` 可以跨线程共享（内部是小连接池）；
- 表结构版本和程序不一致时直接报错，提示重新构建，不会读旧格式。

命令行：`ffr-db stats`（各表行数和文件大小）、`ffr-db sql "<SELECT …>"`（只读查询）、`ffr-db explain "<SELECT …>"`（看执行计划）、`ffr-db bench`（常用读取函数的耗时）。

## 7. 部署

数据库在开发机上构建（不带 `--snapshots`），把单个文件复制到游戏机的数据目录；`ffr-module --db <文件>` 使用它，默认 `$FFR_DEPS/ffr/db/ffr.sqlite`。只有显式给 `--data-dir` 且不给 `--db` 时，`ffr-module` 才走读导出 JSON 的老路径（留作对照）；两个都给会报错。

## 8. 实测（11-25，开发机）

- 完整构建（不含快照，两份 jmap）：约 10 秒，库文件约 610 MB，`build_issue` 约 4,400 条（大多是解析不到的软引用，原因都写在 `detail` 里）；加 `--snapshots` 会再大约 620 MB；
- `GameDb::open` 约 2 ms；单个对象的读取 p99 不到 0.3 ms；文本搜索两字约 2 ms（走 `LIKE`）、四字约 0.07 ms（走 FTS5）；按坐标查附近对象约 2 ms；
- `ffr-module` 第一次调用数据工具：数据库约 3 ms，导出 JSON 约 18–74 ms；进程内存约 12 MB，导出 JSON 约 30–97 MB。
