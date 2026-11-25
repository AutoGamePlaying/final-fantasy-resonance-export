//! Command-line tool `ffr-db` for building and inspecting the local SQLite game database.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use ffr_data::FFR_DEPS_ENV;
use ffr_db::{
    BuildInputs, BuildOptions, GameDb, ResolvedBuildPaths, Stage, build, hash_database,
    resolve_db_path,
};

#[derive(Debug, Parser)]
#[command(name = "ffr-db", version, about = "本地游戏数据库构建与只读查询工具")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 从游戏导出目录构建本地 SQLite 数据库
    Build(BuildArgs),
    /// 计算数据库文件的完整文件哈希（SHA-256）和规范化内容哈希
    Hash(HashArgs),
    /// 显示数据库文件大小、meta 信息与各表行数
    Stats(DbTargetArgs),
    /// 执行单条只读 SQL 查询，结果按 JSON 每行一条输出
    Sql(SqlArgs),
    /// 查看只读 SQL 的 EXPLAIN QUERY PLAN 执行计划
    Explain(SqlArgs),
    /// 在数据库上运行读取延迟基准测试（冷启动首次 + 重复 p50/p99）
    Bench(BenchArgs),
}

#[derive(Debug, Args)]
struct BuildArgs {
    /// 数据表与本地化导出目录（默认 $FFR_DEPS/ffr/export）
    #[arg(long, value_name = "DIR")]
    export: Option<PathBuf>,

    /// 全量资源导出目录（默认 $FFR_DEPS/ffr/export-full）
    #[arg(long, value_name = "DIR")]
    full: Option<PathBuf>,

    /// 资源清单目录（默认 $FFR_DEPS/ffr/inventory）
    #[arg(long, value_name = "DIR")]
    inventory: Option<PathBuf>,

    /// 关卡导出目录（默认 $FFR_DEPS/ffr/export-full/maps）
    #[arg(long, value_name = "DIR")]
    maps: Option<PathBuf>,

    /// 反射转储文件（.jmap），可重复指定多个并合并，不给则跳过 reflection 阶段
    #[arg(long = "jmap", value_name = "FILE")]
    jmaps: Vec<PathBuf>,

    /// 真机快照目录，必须显式指定，不给则跳过 snapshots 阶段
    #[arg(long, value_name = "DIR")]
    snapshots: Option<PathBuf>,

    /// 只执行指定的阶段（逗号分隔，例如 text,battle）
    #[arg(long, value_name = "STAGES", value_delimiter = ',')]
    stages: Option<Vec<String>>,

    /// 输出数据库文件路径（默认 $FFR_DEPS/ffr/db/ffr.sqlite）
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,

    /// 并行扫描线程数（默认自动检测，上限 8；也可通过 FFR_DB_THREADS 设置）
    #[arg(long, value_name = "N")]
    threads: Option<usize>,
}

#[derive(Debug, Args)]
struct HashArgs {
    /// 数据库文件路径（省略时使用 $FFR_DEPS/ffr/db/ffr.sqlite）
    #[arg(value_name = "FILE")]
    path: Option<PathBuf>,

    /// 以单行 JSON 格式输出哈希结果
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct DbTargetArgs {
    /// 数据库文件路径（默认 $FFR_DEPS/ffr/db/ffr.sqlite）
    #[arg(long, value_name = "FILE")]
    db: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct SqlArgs {
    /// 数据库文件路径（默认 $FFR_DEPS/ffr/db/ffr.sqlite）
    #[arg(long, value_name = "FILE")]
    db: Option<PathBuf>,

    /// 要执行的只读 SQL 语句
    #[arg(value_name = "SQL")]
    query: String,
}

#[derive(Debug, Args)]
struct BenchArgs {
    /// 数据库文件路径（默认 $FFR_DEPS/ffr/db/ffr.sqlite）
    #[arg(long, value_name = "FILE")]
    db: Option<PathBuf>,

    /// 每项读取函数的重复测量次数（默认 1000）
    #[arg(long, default_value_t = 1000, value_name = "N")]
    iterations: usize,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("错误：{err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> ffr_db::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Build(args) => run_build(args),
        Command::Hash(args) => run_hash(args),
        Command::Stats(args) => run_stats(args),
        Command::Sql(args) => run_sql(args),
        Command::Explain(args) => run_explain(args),
        Command::Bench(args) => run_bench(args),
    }
}

fn run_build(args: BuildArgs) -> ffr_db::Result<()> {
    let stages = match args.stages {
        Some(names) => {
            let mut parsed = Vec::with_capacity(names.len());
            for name in names {
                parsed.push(Stage::parse(&name)?);
            }
            Some(parsed)
        }
        None => None,
    };

    let explicit_inputs = BuildInputs {
        export: args.export,
        full: args.full,
        inventory: args.inventory,
        maps: args.maps,
        jmaps: args.jmaps,
        snapshots: args.snapshots,
    };
    let resolved =
        ResolvedBuildPaths::resolve(explicit_inputs, args.out, env::var_os(FFR_DEPS_ENV))?;

    let options = BuildOptions {
        inputs: resolved.inputs,
        output: resolved.output,
        stages,
        game_build_id: None,
        threads: args.threads,
    };

    let report = build(&options)?;

    println!(
        "构建完成：{}（schema v{}，耗时 {} ms）",
        report.output.display(),
        report.schema_version,
        report.total_millis
    );
    println!("清单文件：{}", report.manifest_path.display());
    println!("文件哈希（file_sha256）：{}", report.file_sha256);
    println!("内容哈希（content_sha256）：{}", report.content_sha256);
    println!("阶段报告：");
    for stage in &report.stages {
        let note_suffix = if stage.note.is_empty() {
            String::new()
        } else {
            format!("  说明={}", stage.note)
        };
        println!(
            "  {:<12} {:<8} rows={:<8} millis={:<6}{note_suffix}",
            stage.stage,
            stage.status.as_str(),
            stage.rows,
            stage.millis
        );
    }

    if report.total_issues == 0 {
        println!("构建问题（build_issue）：0 条");
    } else {
        println!("构建问题（build_issue）：共 {} 条", report.total_issues);
        for item in &report.issues {
            println!("  {:<12} {:<24} {}", item.stage, item.kind, item.count);
        }
    }
    Ok(())
}

fn run_hash(args: HashArgs) -> ffr_db::Result<()> {
    let db_path = resolve_db_path(args.path.as_deref())?;
    let report = hash_database(&db_path)?;

    if args.json {
        let json = serde_json::to_string(&report).map_err(|err| ffr_db::DbError::DocSerialize {
            table: "<hash>".to_owned(),
            key: db_path.display().to_string(),
            source: err,
        })?;
        println!("{json}");
    } else {
        println!("数据库：{}", db_path.display());
        println!(
            "格式版本：v{}（schema v{}，SQLite {}）",
            report.format, report.schema_version, report.sqlite_version
        );
        println!("文件哈希（file_sha256）：{}", report.file_sha256);
        println!("内容哈希（content_sha256）：{}", report.content_sha256);
        println!("普通表哈希（共 {} 张）：", report.tables.len());
        for table in &report.tables {
            println!("  {:<28} {:>8}  {}", table.name, table.rows, table.sha256);
        }
    }
    Ok(())
}

fn run_stats(args: DbTargetArgs) -> ffr_db::Result<()> {
    let db_path = resolve_db_path(args.db.as_deref())?;
    let db = GameDb::open(&db_path)?;
    let stats = db.stats()?;

    println!("数据库：{}", stats.path.display());
    println!(
        "文件大小：{} 字节（{}）",
        stats.file_size_bytes,
        format_bytes(stats.file_size_bytes)
    );
    println!("元信息（meta）：");
    for (key, value) in &stats.meta {
        println!("  {key} = {value}");
    }
    println!("数据表行数：");
    for table in &stats.tables {
        println!("  {:<28} {}", table.name, table.rows);
    }
    Ok(())
}

fn run_sql(args: SqlArgs) -> ffr_db::Result<()> {
    let db_path = resolve_db_path(args.db.as_deref())?;
    let db = GameDb::open(&db_path)?;
    let rows = db.query_readonly_json(&args.query)?;
    for row in rows {
        let line = serde_json::to_string(&row).map_err(|err| ffr_db::DbError::DocSerialize {
            table: "<sql>".to_owned(),
            key: "<row>".to_owned(),
            source: err,
        })?;
        println!("{line}");
    }
    Ok(())
}

fn run_explain(args: SqlArgs) -> ffr_db::Result<()> {
    let db_path = resolve_db_path(args.db.as_deref())?;
    let db = GameDb::open(&db_path)?;
    let plan = db.explain_query_plan(&args.query)?;
    for step in plan {
        println!("id={} parent={}  {}", step.id, step.parent, step.detail);
    }
    Ok(())
}

struct BenchRow {
    name: String,
    hits: usize,
    cold: Duration,
    p50: Duration,
    p99: Duration,
}

fn run_bench(args: BenchArgs) -> ffr_db::Result<()> {
    let iterations = args.iterations.max(1);
    let db_path = resolve_db_path(args.db.as_deref())?;

    // 1. Measure cold and repeated GameDb::open before touching the file elsewhere.
    let open_start = Instant::now();
    let probe_db = GameDb::open(&db_path)?;
    let open_cold = open_start.elapsed();

    let open_runs = iterations.min(100);
    let mut open_samples = Vec::with_capacity(open_runs);
    for _ in 0..open_runs {
        let t = Instant::now();
        let handle = GameDb::open(&db_path)?;
        open_samples.push(t.elapsed());
        drop(handle);
    }
    open_samples.sort_unstable();
    let open_p50 = percentile(&open_samples, 50);
    let open_p99 = percentile(&open_samples, 99);

    let file_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
    println!(
        "基准数据库：{}（{}，每项重复 {} 次）",
        db_path.display(),
        format_bytes(file_size),
        iterations
    );
    println!(
        "GameDb::open：首次 {:.3} ms | p50 {:.3} ms | p99 {:.3} ms（重复 {} 次）",
        ms(open_cold),
        ms(open_p50),
        ms(open_p99),
        open_runs
    );

    // Discover real sample keys present in this database so bench runs meaningfully on any build.
    let sample_unit_id = if probe_db.unit(2)?.is_some() {
        2
    } else {
        first_i64(&probe_db, "SELECT id FROM unit ORDER BY id LIMIT 1")?.unwrap_or(2)
    };
    let sample_skill_id = if probe_db.skill(220010)?.is_some() {
        220010
    } else {
        first_i64(&probe_db, "SELECT id FROM skill ORDER BY id LIMIT 1")?.unwrap_or(1)
    };
    let sample_group_id = if probe_db.battle_group(1)?.is_some() {
        1
    } else {
        first_i64(&probe_db, "SELECT id FROM battle_group ORDER BY id LIMIT 1")?.unwrap_or(1)
    };
    let sample_map_id = if probe_db.map(3001)?.is_some() {
        3001
    } else {
        first_i64(&probe_db, "SELECT id FROM map ORDER BY id LIMIT 1")?.unwrap_or(3001)
    };
    let sample_level_name = probe_db
        .map(sample_map_id)?
        .map(|m| m.level_name)
        .unwrap_or_else(|| "Dng_01Gra_43_PL".to_owned());
    let sample_item_id = if probe_db.item(1)?.is_some() {
        1
    } else {
        first_i64(&probe_db, "SELECT id FROM item ORDER BY id LIMIT 1")?.unwrap_or(1)
    };
    let sample_shop_id = if probe_db.shop(2)?.is_some() {
        2
    } else {
        first_i64(&probe_db, "SELECT id FROM shop ORDER BY id LIMIT 1")?.unwrap_or(1)
    };
    let sample_quest_id = if probe_db.quest(10)?.is_some() {
        10
    } else {
        first_i64(&probe_db, "SELECT id FROM quest ORDER BY id LIMIT 1")?.unwrap_or(1)
    };
    let sample_dt = first_dt_row_key(&probe_db)?;
    let sample_asset = first_string(&probe_db, "SELECT path FROM asset ORDER BY path LIMIT 1")?;
    let sample_class = first_string(
        &probe_db,
        "SELECT short_name FROM ue_class WHERE short_name != '' ORDER BY path LIMIT 1",
    )?;
    let sample_level_actor = first_level_actor_pos(&probe_db)?;
    drop(probe_db);

    let mut rows = Vec::new();
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("unit({sample_unit_id})"),
        |db| Ok(usize::from(db.unit(sample_unit_id)?.is_some())),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("enemy_profile({sample_unit_id})"),
        |db| Ok(usize::from(db.enemy_profile(sample_unit_id)?.is_some())),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        "enemies_by_exact_name_zh(\"钢蝠\")",
        |db| Ok(db.enemies_by_exact_name_zh("钢蝠")?.len()),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("skills_of_unit({sample_unit_id})"),
        |db| Ok(db.skills_of_unit(sample_unit_id)?.len()),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("skill({sample_skill_id})"),
        |db| Ok(usize::from(db.skill(sample_skill_id)?.is_some())),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        "skills_by_exact_name_zh(\"火炎\")",
        |db| Ok(db.skills_by_exact_name_zh("火炎")?.len()),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("battle_group({sample_group_id})"),
        |db| {
            Ok(db
                .battle_group(sample_group_id)?
                .map_or(0, |g| g.members.len()))
        },
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        "enemies_in_chapter(1)",
        |db| Ok(db.enemies_in_chapter(1)?.len()),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("map({sample_map_id})"),
        |db| Ok(usize::from(db.map(sample_map_id)?.is_some())),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("map_by_level(\"{sample_level_name}\")"),
        |db| Ok(usize::from(db.map_by_level(&sample_level_name)?.is_some())),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("map_contents({sample_map_id})"),
        |db| Ok(usize::from(db.map_contents(sample_map_id)?.is_some())),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("item({sample_item_id})"),
        |db| Ok(usize::from(db.item(sample_item_id)?.is_some())),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("item_sources({sample_item_id})"),
        |db| Ok(db.item_sources(sample_item_id)?.len()),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("shop({sample_shop_id})"),
        |db| Ok(db.shop(sample_shop_id)?.map_or(0, |s| s.stock.len())),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        format!("quest({sample_quest_id})"),
        |db| Ok(usize::from(db.quest(sample_quest_id)?.is_some())),
    )?);
    rows.push(bench_case(&db_path, iterations, "chapter(1)", |db| {
        Ok(usize::from(db.chapter(1)?.is_some()))
    })?);
    rows.push(bench_case(
        &db_path,
        iterations,
        "main_scenario_for_progress(1020)",
        |db| Ok(usize::from(db.main_scenario_for_progress(1020)?.is_some())),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        "text(\"ST_UnitName\",\"UnitName2\",\"zh-Hans\")",
        |db| {
            Ok(usize::from(
                db.text("ST_UnitName", "UnitName2", "zh-Hans")?.is_some(),
            ))
        },
    )?);
    if let Some((table_path, row_name)) = sample_dt {
        rows.push(bench_case(
            &db_path,
            iterations,
            format!("dt_row(\"{table_path}\",\"{row_name}\")"),
            |db| Ok(usize::from(db.dt_row(&table_path, &row_name)?.is_some())),
        )?);
    }
    if let Some(asset_path) = sample_asset {
        rows.push(bench_case(&db_path, iterations, "asset(<sample>)", |db| {
            Ok(usize::from(db.asset(&asset_path)?.is_some()))
        })?);
    }
    if let Some(class_name) = sample_class {
        rows.push(bench_case(
            &db_path,
            iterations,
            format!("ue_class(\"{class_name}\")"),
            |db| Ok(usize::from(db.ue_class(&class_name)?.is_some())),
        )?);
    }
    if let Some((level_name, pos)) = sample_level_actor {
        rows.push(bench_case(
            &db_path,
            iterations,
            format!("actors_near(\"{level_name}\", r=1500)"),
            |db| {
                Ok(db
                    .actors_near(
                        ffr_db::query::levels::ActorScope::Level(&level_name),
                        pos,
                        1500.0,
                        None,
                    )?
                    .len())
            },
        )?);
    }

    // Text search: 2-char and 4-char zh-Hans queries.
    rows.push(bench_case(
        &db_path,
        iterations,
        "search_text(\"钢蝠\", zh-Hans, 20) [2字]",
        |db| Ok(db.search_text("钢蝠", Some("zh-Hans"), 20)?.len()),
    )?);
    rows.push(bench_case(
        &db_path,
        iterations,
        "search_text(\"无法战斗\", zh-Hans, 20) [4字]",
        |db| Ok(db.search_text("无法战斗", Some("zh-Hans"), 20)?.len()),
    )?);

    println!();
    println!(
        "{:<46} {:>6} {:>10} {:>10} {:>10}",
        "查询", "命中", "首次(ms)", "p50(ms)", "p99(ms)"
    );
    println!("{}", "-".repeat(86));
    for row in &rows {
        println!(
            "{:<46} {:>6} {:>10.3} {:>10.3} {:>10.3}",
            row.name,
            row.hits,
            ms(row.cold),
            ms(row.p50),
            ms(row.p99),
        );
    }
    Ok(())
}

fn bench_case<F>(
    db_path: &Path,
    iterations: usize,
    name: impl Into<String>,
    mut f: F,
) -> ffr_db::Result<BenchRow>
where
    F: FnMut(&GameDb) -> ffr_db::Result<usize>,
{
    let db = GameDb::open(db_path)?;
    let cold_start = Instant::now();
    let hits = f(&db)?;
    let cold = cold_start.elapsed();

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        let _ = f(&db)?;
        samples.push(start.elapsed());
    }
    samples.sort_unstable();

    Ok(BenchRow {
        name: name.into(),
        hits,
        cold,
        p50: percentile(&samples, 50),
        p99: percentile(&samples, 99),
    })
}

fn percentile(sorted: &[Duration], pct: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = (sorted.len() * pct).div_ceil(100);
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx]
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn first_i64(db: &GameDb, sql: &str) -> ffr_db::Result<Option<i64>> {
    let rows = db.query_readonly_json(sql)?;
    Ok(rows
        .first()
        .and_then(|row| row.values().next())
        .and_then(|v| v.as_i64()))
}

fn first_string(db: &GameDb, sql: &str) -> ffr_db::Result<Option<String>> {
    let rows = db.query_readonly_json(sql)?;
    Ok(rows
        .first()
        .and_then(|row| row.values().next())
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned))
}

fn first_dt_row_key(db: &GameDb) -> ffr_db::Result<Option<(String, String)>> {
    let rows = db.query_readonly_json(
        "SELECT t.path, r.row_name \
         FROM dt_table t \
         JOIN dt_row r ON r.table_id = t.id \
         WHERE t.path = 'Unit/DT_UnitParameter' \
         ORDER BY r.row_name LIMIT 1",
    )?;
    let row = match rows.first() {
        Some(r) => Some(r.clone()),
        None => db
            .query_readonly_json(
                "SELECT t.path, r.row_name \
                 FROM dt_table t \
                 JOIN dt_row r ON r.table_id = t.id \
                 LIMIT 1",
            )?
            .into_iter()
            .next(),
    };
    Ok(row.and_then(|obj| {
        Some((
            obj.get("path")?.as_str()?.to_owned(),
            obj.get("row_name")?.as_str()?.to_owned(),
        ))
    }))
}

fn first_level_actor_pos(
    db: &GameDb,
) -> ffr_db::Result<Option<(String, ffr_data::assets::level::Vec3)>> {
    let rows = db.query_readonly_json(
        "SELECT l.name, a.x, a.y, a.z \
         FROM actor a \
         JOIN level l ON l.id = a.level_id \
         WHERE a.x IS NOT NULL AND a.y IS NOT NULL AND a.z IS NOT NULL \
         LIMIT 1",
    )?;
    Ok(rows.first().and_then(|obj| {
        Some((
            obj.get("name")?.as_str()?.to_owned(),
            ffr_data::assets::level::Vec3 {
                x: obj.get("x")?.as_f64()?,
                y: obj.get("y")?.as_f64()?,
                z: obj.get("z")?.as_f64()?,
            },
        ))
    }))
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.2} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.2} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}
