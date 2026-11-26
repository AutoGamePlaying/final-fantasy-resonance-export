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

- **索引列**：编号、名字的文本键、解析好的三语名（`_ja`、`_en`、`_zh`）、分类等常用于查找和筛选的字段，各自建索引；
- **`doc` 列**：整个 Rust 对象的 JSON（serde 序列化）。读出来直接反序列化成对象，不用把每个嵌套列表都拆成表；
- **关系表**：对象之间的引用（单位 → 招式、商店 → 道具、地图 → 宝箱、关卡 → Actor……）单独成表，两端都建索引，正查、反查都快。

另有一层**原始数据表**：全部 2618 个数据表的每一行原样存进 `dt_row`，没有专门对象的表也能按表名 + 行名直接取，或者用 SQLite 的 JSON 函数临时查询。

## 3. 区域和表

| 区域 | 来源 | 主要的表 |
| --- | --- | --- |
| 构建信息 | 构建器 | `meta`（`schema_version = 4`、游戏构建号）、`source`（每个来源的路径和 manifest 哈希）、`stage_report`（每段的行数、状态）、`build_issue`（悬空引用等问题，照实记录，不静默丢弃） |
| 文本 | `Localization/<lang>.json`、字符串表 | `text(id, ns, key, ja, en, zh)`（按 `(ns, key)` 合并为一行，仅保留真正翻译过的条目和全部 `ja` 原文，缺失或未翻译的列为 `NULL`）、`text_language(lang, column_name)`（语言标签到列名的映射）、`text_untranslated`（稀疏例外，保持每种语言一行，含原样 `value` 和判定依据 `reason`）、`text_fts`（三列 `ja, en, zh` 的 FTS5 trigram 索引，仅索引 `text`）、`string_table`、`string_table_entry` |
| 原始数据表 | `FFRS/Content/Datatable/` | `dt_table`、`dt_row` |
| 战斗 | 数据表 | `unit`、`unit_name`、`unit_name_fts`、`unit_resistance`、`skill`、`skill_name`、`skill_name_fts`、`unit_skill`、`command_skill`、`battle_group`、`battle_group_member`、`encounter`、`unit_ai`、`enemy_chapter`；未翻译伴随表 `unit_untranslated`、`skill_untranslated`、`command_skill_untranslated` |
| 世界 | 数据表 | `map`、`island`、`region`、`map_release_condition`、`save_point`、`save_point_command`、`teleport_point`、`map_link`、`treasure_box`、`item`、`shop`、`shop_item`、`inn`、`quest`、`quest_prize`、`quest_board`、`chapter`、`main_scenario`、`main_quest`、`flag_condition`、`playable_unit`、`item_drop`、`world_name`、`world_name_fts`；未翻译伴随表 `map_untranslated`、`island_untranslated`、`save_point_command_untranslated`、`item_untranslated`、`shop_untranslated`、`quest_untranslated`、`quest_prize_untranslated`、`quest_board_untranslated`、`chapter_untranslated`、`main_scenario_untranslated`、`main_quest_untranslated`、`playable_unit_untranslated`、`item_drop_untranslated` |
| 资源索引 | `AssetRegistry.bin`（`inventory/registry.jsonl`）、全量导出、`Config/*.ini` | `asset`、`asset_tag`、`asset_ref`（资源之间的引用，从导出 JSON 里收集）、`config_entry` |
| 反射 | `.jmap` | `ue_class`（类、结构体、枚举）、`ue_property`、`ue_function`、`ue_enum_value` |
| 输入、蓝图、界面 | 全量导出 | `input_action`、`input_context`、`input_mapping`、`input_set`；`blueprint`（类默认值里改过的属性）、`blueprint_component`；`widget_node`（界面控件树） |
| 关卡 | [final-fantasy-resonance-export](https://github.com/AutoGamePlaying/final-fantasy-resonance-export) `export-maps` 的关卡文件和 `_graph.json` | `level`、`actor`、`actor_rtree`（R*Tree 空间索引）、`level_object`（NPC、宝箱、事件、出入口、存档点、商店、机关、遭遇区域，带指向对应数据的编号）、`map_edge` |
| 过场 | 全量导出里的 `LevelSequence` | `sequence`、`sequence_binding` |
| 真机快照（可选） | `.deps/r1/*` | `snapshot`、`snapshot_object`、`snapshot_widget`、`snapshot_text`、`marker` |
| 跨区域连接 | 前面各区域的表和导出 | `talk_event`、`talk_event_next`、`talk_line`（对话事件 → 过场、台词）、`level_object_talk`、`map_unit`（`DT_MapUnitAsset`）、`class_ref`（各表出现的类名统一后连到反射或蓝图）；未翻译伴随表 `talk_line_untranslated`、`map_unit_untranslated` |

跨区域的连接：

- 所有名字列都存文本键，连到 `text`；
- 关卡的 `map_id` 连到 `map`，`level_object` 里的商店、宝箱、地图单位编号分别连到 `shop`、`treasure_box`、`playable_unit`/`unit`；
- `blueprint.parent_class`、`actor.class`、`snapshot_object.class` 连到 `ue_class`；
- 关卡、蓝图、过场、输入资产都连到 `asset`；
- 对话链：关卡事件触发的 `interactEventId` → `talk_event`（`CDT_TalkEventData_Prod`）→ 过场 `sequence` 和对话表 → `talk_line` → `text`。

暂不入库的数据（原始数据仍在 `.deps/` 下，需要时再导）：

- 资源注册表的 `ActorsMetaData` 标签：每个关卡对象的描述（对象路径、类、二进制描述），内容和关卡导出重复，正文约 120 MB。原始数据在 `inventory/registry.jsonl`；
- 真机快照默认不导入，只有构建时给了 `--snapshots` 才导入。部署到游戏机的库不带快照。

### 多语言宽表结构（Schema v4）

游戏固定提供三种语言（`ja`、`en`、`zh-Hans`）。自 schema v4 起，多语言表从“每种语言一行”改为“一行一个对象、每种语言一列”，列名统一为 `ja`、`en`、`zh`（`zh` 对应 `zh-Hans`）：

- **语言列映射表**：`text_language(lang TEXT PRIMARY KEY, column_name TEXT NOT NULL UNIQUE)`，固定记录 `('en', 'en')`、`('ja', 'ja')`、`('zh-Hans', 'zh')`，供程序和人工查询。
- **四张多语言宽表与 FTS5 索引**：
  - `text(id INTEGER PRIMARY KEY, ns, key, ja, en, zh, UNIQUE(ns, key))`；`text_fts(ja, en, zh, content='text', content_rowid='id', tokenize='trigram')`；
  - `unit_name(unit_id INTEGER PRIMARY KEY, ja, en, zh)`；`unit_name_fts(ja, en, zh, content='unit_name', content_rowid='unit_id', tokenize='trigram')`；
  - `skill_name(skill_id INTEGER PRIMARY KEY, ja, en, zh)`；`skill_name_fts(ja, en, zh, content='skill_name', content_rowid='skill_id', tokenize='trigram')`；
  - `world_name(id INTEGER PRIMARY KEY, entity_kind, entity_id, ja, en, zh, en_lower, UNIQUE(entity_kind, entity_id))`；`world_name_fts(ja, en, zh, content='world_name', content_rowid='id', tokenize='trigram')`。其中 `en_lower` 保存 `en` 的小写形式并建部分索引 `idx_world_name_en_lower(entity_kind, en_lower, entity_id) WHERE en_lower IS NOT NULL`，保留英文不区分大小写的精确查找和短词查找语义。
- **对象表三语同源列**（列顺序统一为 `_ja`、`_en`、`_zh`，缺失或未翻译时为 `NULL`）：
  - `name_ja, name_en, name_zh`：`unit`、`skill`、`command_skill`、`map`、`island`、`item`、`shop`、`quest`、`quest_board`、`chapter`、`main_quest`、`playable_unit`、`map_unit`；
  - `label_ja, label_en, label_zh`：`save_point_command`；
  - `text_ja, text_en, text_zh`：`quest_prize`、`main_scenario`；
  - `unit_name_ja, unit_name_en, unit_name_zh`：`item_drop`。

示例 SQL：

```sql
-- 1. 按 (ns, key) 一次取三种语言
SELECT ja, en, zh FROM text WHERE ns = 'ST_UnitName' AND key = 'UnitName1';

-- 2. FTS5 按语言列过滤搜索（>= 3 字符）
SELECT t.ns, t.key, t.zh
FROM text_fts
JOIN text AS t ON t.id = text_fts.rowid
WHERE text_fts MATCH 'zh : "钢铁蝙"'
LIMIT 20;

-- 3. FTS5 不限语言搜全部三列
SELECT t.ns, t.key, t.ja, t.en, t.zh
FROM text_fts
JOIN text AS t ON t.id = text_fts.rowid
WHERE text_fts MATCH '"Potion"'
LIMIT 20;

-- 4. world_name 英文不区分大小写精确查找（走 idx_world_name_en_lower）
SELECT m.id, m.name_en, m.name_zh
FROM world_name AS w
JOIN map AS m ON m.id = w.entity_id
WHERE w.entity_kind = 'map' AND w.en_lower = lower('Grandshelt');

-- 5. 未翻译记录查询（保持每种语言一行）
SELECT unit_id, field, lang, text_ns, text_key, value
FROM unit_untranslated
WHERE lang = 'zh-Hans'
ORDER BY unit_id, field;
```

### 文本与“游戏未翻译”分表

游戏官方的 `Localization/<lang>.json` 里有部分尚未翻译的条目，原文直接留着日文。为了遵守**不拿日文冒充别的语言**的原则，构建时会把真正翻译过的文本与游戏未翻译的文本分表存放：

- **判定规则**（对 `lang != 'ja'` 的条目，与其同 `(ns, key)` 的 `ja` 值比较）：
  - 与 `ja` 值**完全相同**且含有日文假名（平假名或片假名，不含中点 `・` 等标点）的，判定为未翻译，依据记为 `same_as_ja_with_kana`；
  - 英文条目（`en` / `en-*`）放宽为：与 `ja` 完全相同且含有任何日文假名或 CJK 汉字的，判定为未翻译，仅含汉字时依据记为 `same_as_ja_with_cjk`；
  - 简体中文条目（`zh-Hans` / `zh-Hans-*` / `zh-CN` / `zh-SG`）与 `ja` 完全相同、不含假名、且至少含有一个不在 GB2312（6,763 个汉字）集合里的汉字（通常是日文字形，如“剣技”“盗賊技”“説明文”）时，判定为未翻译，依据记为 `same_as_ja_with_non_simplified_hanzi`；
  - 取舍：简体中文里与 `ja` 完全相同但**全部由 GB2312 常用简体字组成**的条目（如“恋人”“青年”“兵法”及纯汉字人名、地名），在中文里本身成立，仍当作有效译文保留在 `text.zh` 中。
- **文本分表**：
  - `text` 按 `(ns, key)` 合并为一行，三列 `ja`、`en`、`zh` 只保留真正翻译过的非日文条目以及全部 `ja` 原文；某语言包里没有该键或判定为未翻译时，对应列为 `NULL`（原始空字符串 `""` 仍保留为 `""`）。`text_fts` 仅索引 `text` 的三列，不把冒充译文的日文混进普通文本搜索；
  - `text_untranslated(ns, key, lang, value, reason)` 属于稀疏的例外记录，**保持“每种语言一行”不变**，原样保存被判为未翻译的条目，主键 `(ns, key, lang)`，建有 `(lang, ns, key)` 索引；可通过 `GameDb::text_untranslated()` 和 `GameDb::untranslated_texts(lang)` 查询。
- **实体未翻译伴随表**：
  - 所有含按语言解析文本字段（名称、说明、目标、台词等）的 18 张实体表，各配一张 `<表名>_untranslated(<主表主键列>, field, lang, text_ns, text_key, value)`，同样**保持“每种语言一行”不变**，在 `(<主表主键列>, field, lang)` 上建主键，在 `(lang, <主表主键列>, field)` 上建索引；
  - 主表的 `*_ja` / `*_en` / `*_zh`（`name_*`、`label_*`、`text_*`、`unit_name_*`）在对应语言缺失或未翻译时保持 `NULL`；`stage_report.note` 记录 `text_untranslated` 及每张伴随表各语言的行数；
  - 每张伴随表都提供按语言和按主表主键读取的 `GameDb` 方法（例如 `untranslated_items(lang)` / `item_untranslated_fields(item_id)`、`untranslated_skills(lang)` / `skill_untranslated_fields(skill_id)`、`untranslated_talk_lines(lang)` / `talk_line_untranslated_fields(table_path, row)` 等）。
- **读取与解析**：
  - `GameDb::text()`、`GameDb::text_all_langs()`、`GameDb::search_text()`、`GameDb::talk_lines()` 只查 `text`（按语言映射到 `ja` / `en` / `zh` 列），因此不会把未翻译的日文当成其他语言返回；
  - `GameDb::resolve_ftext()` 先查 `text`，查不到再查 `text_untranslated`：命中时默认按**缺失**处理，原因为“游戏官方未翻译（该语言文件里是日文原文）”；仅在显式开启 `fallback_to_native` 回落时，返回日文原文并标明 `lang = "ja"`；
  - 读 JSON 的 `ffr_data::TextResolver` 使用完全一致的判定和回落规则，与数据库路径保持对齐。

## 4. 索引

- 每张表的主键；关系表两端各建索引；对象表三语名字列（`*_ja`、`*_en`、`*_zh`）与分类列建索引；
- `text_fts`、`unit_name_fts`、`skill_name_fts`、`world_name_fts`：三列（`ja, en, zh`）FTS5 trigram 外部内容索引，>= 3 字符的子串搜索按语言用 FTS5 列过滤（不限语言搜全部三列）；
- 1–2 字符短词子串搜索走各语言部分覆盖索引（`WHERE <col> IS NOT NULL`），`world_name` 英文精确与短词查找走 `idx_world_name_en_lower`；
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
- 悬空引用（比如单位引用了不存在的招式）写进 `build_issue`，构建结束时汇总打印；
- **来源清单的规范化哈希**：`source.manifest_sha256` 在计算导出目录 `manifest.json` 的 SHA-256 前，会先递归剔除与数据语义无关的易变键（`startedAt`、`finishedAt`、`elapsedSeconds`、`elapsedMs`、`generatedAt`、`jobs`、`usmapSha256`）并按字典序重排 JSON 对象键，原始文件哈希仅记录在旁路 `<db>.manifest.json` 中。剔除 `usmapSha256` 是因为同一份类型表在不同机器或不同转储中生成的文件字节可能不同，`.usmap` 的身份已由建库报告（`*.build.json`）与 `known-builds.json` 另行记录；若 `.usmap` 语义内容真的不同，导出数据表本身会跟着变化并反映在内容哈希中。

## 6. 读取

```rust
let db = GameDb::open(path)?;          // 只读打开，检查表结构版本（当前 v4）
let bat = db.unit(2)?;                 // Option<Unit>
let skills = db.skills_of_unit(2)?;    // Vec<UnitSkillEntry>
let src = db.item_sources(item_id)?;   // 宝箱、商店、任务奖励……
let hits = db.search_text("钢蝠", Some("zh-Hans"), 20)?;
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
