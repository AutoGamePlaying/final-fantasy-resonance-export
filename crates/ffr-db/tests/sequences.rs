//! Integration tests for the `09_sequences` database area:
//! parsing synthetic UE 5.6 `LevelSequence` fixtures, importing into SQLite,
//! exercising every read query, verifying index usage via `assert_db_uses_index`,
//! handling malformed/dangling assets via `build_issue`, and optional real-data verification.

use std::fs;
use std::path::PathBuf;

use ffr_data::assets::sequence::{BindingKind, SequenceCategory};
use ffr_db::query::sequences::{
    SQL_SEQUENCE_BINDINGS_BY_PATH, SQL_SEQUENCE_BY_PATH, SQL_SEQUENCE_EVENTS_BY_PATH,
    SQL_SEQUENCE_TRACKS_BY_PATH, SQL_SEQUENCES_BY_EVENT_ID, SQL_SEQUENCES_BY_FLAG,
    SQL_SEQUENCES_CALLING, SQL_SEQUENCES_IN_DIR, SQL_SEQUENCES_WITH_UNIT_CLASS,
    SQL_SEQUENCES_WITH_UNIT_ID, SequenceUnitFilter, directory_prefix_bounds,
};
use ffr_db::testing::{assert_db_uses_index, build_fixture_db_stages, build_temp_db};
use ffr_db::{BuildInputs, Stage, StageStatus};
use rusqlite::params;

const ALL_SEQUENCE_TABLES: &[&str] = &[
    "sequence",
    "sequence_binding",
    "sequence_track",
    "sequence_event",
];

#[test]
fn fixture_build_imports_sequences_bindings_tracks_and_events() {
    let test_db = build_fixture_db_stages("sequences", Some(vec![Stage::Sequences]))
        .expect("build sequences fixture db");
    let report = test_db.report();
    assert_eq!(report.total_issues, 0, "issues={:?}", report.issues);

    let seq_stage = report
        .stages
        .iter()
        .find(|s| s.stage == "sequences")
        .expect("sequences stage report");
    assert_eq!(seq_stage.status, StageStatus::Ok);
    // 4 sequences + 7 bindings + 11 tracks + 8 events = 30 total rows (BP_Helper_Dummy skipped).
    assert_eq!(seq_stage.rows, 30, "note={}", seq_stage.note);

    let db = test_db.db();

    // 1. sequence(path): canonical, AssetPathName, relative, and missing path.
    let master_path = "/Game/Sequencer/Event/Main_Event/C01/Ev_C01_Demo/SEQ_Ev_C01_Demo_Master";
    let master = db
        .sequence(master_path)
        .expect("query master sequence")
        .expect("master sequence must exist");
    assert_eq!(master.name, "SEQ_Ev_C01_Demo_Master");
    assert_eq!(master.category, SequenceCategory::Event);
    assert!((master.display_rate.as_fps() - 30.0).abs() < 1e-6);
    assert_eq!(master.start_frame, Some(0.0));
    assert_eq!(master.end_frame, Some(450.0));
    assert!((master.duration_secs - 15.0).abs() < 1e-6);
    assert_eq!(master.binding_count, 3);
    assert_eq!(master.track_count, 5);
    assert_eq!(master.event_count, 4);
    assert_eq!(
        master.director_class.as_deref(),
        Some("SEQ_Ev_C01_Demo_Master_DirectorBP_C")
    );
    assert_eq!(
        master.director_super_class.as_deref(),
        Some("CPP_TalkEventSequenceDirector")
    );
    assert!(master.has_skip_markers);
    assert!(master.has_skip_ban_param);
    assert_eq!(master.marked_frames.len(), 2);
    assert_eq!(master.marked_frames[0].label, "SkipBegin");
    assert_eq!(master.marked_frames[0].frame, 30.0);
    assert_eq!(
        master.sub_sequences,
        vec!["/Game/Sequencer/Event/Main_Event/C01/Ev_C01_Demo/Track/SEQ_Ev_C01_Demo_Cut001"]
    );
    assert!(
        master
            .ubergraph_calls
            .contains(&"ChangeAnimationAssetWithStruct".to_owned())
    );
    assert!(master.ubergraph_calls.contains(&"GetNpcTalkId".to_owned()));

    // AssetPathName and relative path normalization match the same record.
    assert_eq!(
        db.sequence(&format!("{master_path}.SEQ_Ev_C01_Demo_Master"))
            .expect("query AssetPathName"),
        Some(master.clone())
    );
    assert_eq!(
        db.sequence("Event/Main_Event/C01/Ev_C01_Demo/SEQ_Ev_C01_Demo_Master.json")
            .expect("query relative path"),
        Some(master)
    );
    assert_eq!(
        db.sequence("/Game/Sequencer/DoesNotExist")
            .expect("query missing"),
        None
    );

    // 2. sequences_in_dir(prefix): recursive + exact directory boundary check.
    let c01_seqs = db
        .sequences_in_dir("/Game/Sequencer/Event/Main_Event/C01")
        .expect("query C01 dir");
    let c01_names: Vec<&str> = c01_seqs.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        c01_names,
        vec!["SEQ_Ev_C01_Demo_Master", "SEQ_Ev_C01_Demo_Cut001"]
    );

    // Relative prefix and trailing slash produce identical results.
    assert_eq!(
        db.sequences_in_dir("Event/Main_Event/C01/")
            .expect("query relative dir"),
        c01_seqs
    );
    // Partial directory name must NOT leak sibling directories.
    assert!(
        db.sequences_in_dir("/Game/Sequencer/Event/Main_Event/C0")
            .expect("query partial prefix")
            .is_empty()
    );

    // 3. sequence_bindings(path) & sequence_tracks(path).
    let bindings = db
        .sequence_bindings(master_path)
        .expect("query master bindings");
    assert_eq!(bindings.len(), 3);
    assert_eq!(bindings[0].name, "Rain");
    assert_eq!(bindings[0].kind, BindingKind::Spawnable);
    assert_eq!(bindings[0].template_class.as_deref(), Some("CPP_MapUnit"));
    assert_eq!(bindings[0].map_unit_id, Some(10));
    assert_eq!(bindings[1].name, "Guardian");
    assert_eq!(
        bindings[1].visible_flag_condition.as_deref(),
        Some("!{flag:12}")
    );
    assert_eq!(bindings[2].name, "MoonMesh");
    assert_eq!(bindings[2].kind, BindingKind::Possessable);
    assert_eq!(
        bindings[2].locator_path.as_deref(),
        Some("/Game/Map/Demo/Demo_BG.Demo_BG:PersistentLevel.StaticMeshActor_1")
    );

    // Battle possessable binding backfills template_class from ENTRYPOINT ObjectProperty.
    let battle_path = "/Game/Sequencer/Battle/EnemySkill/500010/SEQ_Battle_500010_Master";
    let battle = db
        .sequence(battle_path)
        .expect("query battle")
        .expect("battle exists");
    assert_eq!(battle.category, SequenceCategory::Battle);
    assert_eq!(
        battle.director_super_class.as_deref(),
        Some("BP_BtlLevelSequenceDirector_C")
    );
    let battle_bindings = db
        .sequence_bindings(battle_path)
        .expect("query battle bindings");
    assert_eq!(battle_bindings.len(), 2);
    assert_eq!(battle_bindings[0].kind, BindingKind::Possessable);
    assert_eq!(
        battle_bindings[0].template_class.as_deref(),
        Some("CPP_BtlSequencerEvent")
    );

    let cut_tracks = db
        .sequence_tracks("Event/Main_Event/C01/Ev_C01_Demo/Track/SEQ_Ev_C01_Demo_Cut001")
        .expect("query cut tracks");
    assert_eq!(cut_tracks.len(), 4);
    assert!(
        cut_tracks
            .iter()
            .any(|t| t.track_type == "MovieSceneAtomTrack" && t.section_count == 1)
    );
    assert!(
        cut_tracks
            .iter()
            .any(|t| t.track_type == "MovieScene3DAttachTrack" && t.section_count == 1)
    );

    // 4. sequence_events(path): chronological order, ENTRYPOINT resolution, payload & inherited IDs.
    let events = db
        .sequence_events(master_path)
        .expect("query master events");
    assert_eq!(events.len(), 4);
    assert_eq!(events[0].frame, 0.0);
    assert_eq!(events[0].function, "SequenceEvent");
    assert!(events[0].raw_entrypoint.is_some());

    assert_eq!(events[1].frame, 60.0);
    assert_eq!(events[1].function, "Rain_Event");
    assert_eq!(events[1].map_unit_id, Some(10));

    assert_eq!(events[2].frame, 120.0);
    assert_eq!(events[2].function, "TriggerStoryBeat");
    assert_eq!(events[2].talk_event_id.as_deref(), Some("C01_DEMO_01"));
    assert_eq!(events[2].flag_id, Some(12));
    assert_eq!(events[2].map_unit_id, Some(60));

    assert_eq!(events[3].frame, 180.0);
    assert_eq!(events[3].function, "PlayTalkEvent");
    assert_eq!(events[3].talk_event_id.as_deref(), Some("C01_DEMO_02"));
    assert_eq!(events[3].flag_id, Some(35));

    // Ambiguous ENTRYPOINT on Other/SEQ_Title_Logo stays honest without guessing.
    let title_events = db
        .sequence_events("/Game/Sequencer/Other/SEQ_Title_Logo")
        .expect("query title events");
    assert_eq!(title_events.len(), 1);
    assert_eq!(
        title_events[0].function,
        "SequenceEvent__ENTRYPOINTSEQ_Title_Logo_DirectorBP_0"
    );
    assert_eq!(title_events[0].raw_entrypoint, None);
    assert_eq!(
        title_events[0].params.get("candidate_functions"),
        Some(&serde_json::json!(["PlayFade", "ShowTitle"]))
    );

    // 5. sequences_calling(function).
    let calling_btl = db
        .sequences_calling("SeqBattleEvent")
        .expect("query SeqBattleEvent callers");
    assert_eq!(calling_btl.len(), 1);
    assert_eq!(calling_btl[0].name, "SEQ_Battle_500010_Master");

    let calling_talk = db
        .sequences_calling("PlayTalkEvent")
        .expect("query PlayTalkEvent callers");
    assert_eq!(calling_talk.len(), 2);

    // 6. sequences_with_unit (by m_MapUnitId and by template_class).
    let with_unit_60 = db
        .sequences_with_unit(SequenceUnitFilter::MapUnitId(60))
        .expect("query unit 60");
    assert_eq!(with_unit_60.len(), 1);
    assert_eq!(with_unit_60[0].name, "SEQ_Ev_C01_Demo_Master");

    let with_map_unit_cls = db
        .sequences_with_unit(SequenceUnitFilter::TemplateClass("CPP_MapUnit"))
        .expect("query CPP_MapUnit");
    assert_eq!(with_map_unit_cls.len(), 2);

    let with_cam = db
        .sequences_with_unit_class("CineCameraActor")
        .expect("query CineCameraActor");
    assert_eq!(with_cam.len(), 1);
    assert_eq!(with_cam[0].name, "SEQ_Ev_C01_Demo_Cut001");

    // 7. sequences_by_event_id & sequences_by_flag.
    let by_ev_01 = db
        .sequences_by_event_id("C01_DEMO_01")
        .expect("query C01_DEMO_01");
    assert_eq!(by_ev_01.len(), 2);

    let by_ev_02 = db
        .sequences_by_event_id("C01_DEMO_02")
        .expect("query C01_DEMO_02");
    assert_eq!(by_ev_02.len(), 1);
    assert_eq!(by_ev_02[0].name, "SEQ_Ev_C01_Demo_Master");

    let by_flag_12 = db.sequences_by_flag(12).expect("query flag 12");
    assert_eq!(by_flag_12.len(), 1);
    assert_eq!(by_flag_12[0].name, "SEQ_Ev_C01_Demo_Master");
}

#[test]
fn every_sequence_query_uses_index_without_full_table_scan() {
    let test_db = build_fixture_db_stages("sequences", Some(vec![Stage::Sequences]))
        .expect("build sequences fixture db");
    let db = test_db.db();
    let master_path = "/Game/Sequencer/Event/Main_Event/C01/Ev_C01_Demo/SEQ_Ev_C01_Demo_Master";
    let (lower, upper) =
        directory_prefix_bounds("/Game/Sequencer/Event/Main_Event/C01").expect("prefix bounds");

    assert_db_uses_index(db, SQL_SEQUENCE_BY_PATH, [master_path], ALL_SEQUENCE_TABLES);
    assert_db_uses_index(
        db,
        SQL_SEQUENCES_IN_DIR,
        [lower.as_str(), upper.as_str()],
        ALL_SEQUENCE_TABLES,
    );
    assert_db_uses_index(
        db,
        SQL_SEQUENCE_BINDINGS_BY_PATH,
        [master_path],
        ALL_SEQUENCE_TABLES,
    );
    assert_db_uses_index(
        db,
        SQL_SEQUENCE_TRACKS_BY_PATH,
        [master_path],
        ALL_SEQUENCE_TABLES,
    );
    assert_db_uses_index(
        db,
        SQL_SEQUENCE_EVENTS_BY_PATH,
        [master_path],
        ALL_SEQUENCE_TABLES,
    );
    assert_db_uses_index(
        db,
        SQL_SEQUENCES_CALLING,
        ["SeqBattleEvent"],
        ALL_SEQUENCE_TABLES,
    );
    assert_db_uses_index(
        db,
        SQL_SEQUENCES_WITH_UNIT_ID,
        params![60_i64],
        ALL_SEQUENCE_TABLES,
    );
    assert_db_uses_index(
        db,
        SQL_SEQUENCES_WITH_UNIT_CLASS,
        ["CPP_MapUnit"],
        ALL_SEQUENCE_TABLES,
    );
    assert_db_uses_index(
        db,
        SQL_SEQUENCES_BY_EVENT_ID,
        ["C01_DEMO_01"],
        ALL_SEQUENCE_TABLES,
    );
    assert_db_uses_index(
        db,
        SQL_SEQUENCES_BY_FLAG,
        params![12_i64],
        ALL_SEQUENCE_TABLES,
    );
}

#[test]
fn malformed_files_and_dangling_refs_record_build_issues_without_aborting() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let seq_dir = dir.path().join("FFRS/Content/Sequencer/Event/Broken");
    fs::create_dir_all(&seq_dir).expect("create broken dir");

    // 1. Corrupt JSON file -> parse_error build_issue.
    fs::write(seq_dir.join("SEQ_Corrupt.json"), "{ not valid json").expect("write corrupt json");

    // 2. Valid JSON with missing PlaybackRange, missing BindingReference, and dangling CustomBinding.
    let partial_seq = serde_json::json!([
        {
            "Type": "LevelSequence",
            "Name": "SEQ_Partial",
            "Package": "/Game/Sequencer/Event/Broken/SEQ_Partial",
            "Properties": {
                "MovieScene": { "ObjectName": "MovieScene'SEQ_Partial:MovieScene_0'" },
                "BindingReferences": {
                    "SortedReferences": [
                        {
                            "ID": "AAAA0000-00000000-00000000-00000001",
                            "Locator": { "Fragments": [] },
                            "CustomBinding": {
                                "ObjectName": "MovieSceneSpawnableActorBinding'SEQ_Partial:MovieScene_0.MissingBinding'"
                            }
                        }
                    ]
                }
            }
        },
        {
            "Type": "MovieScene",
            "Name": "MovieScene_0",
            "Outer": { "ObjectName": "LevelSequence'SEQ_Partial'" },
            "Properties": {
                "TickResolution": { "Numerator": 24000 },
                "DisplayRate": { "Numerator": 30 },
                "Possessables": [
                    {
                        "Guid": "AAAA0000-00000000-00000000-00000001",
                        "Name": "DanglingSpawnable",
                        "ParentGuid": "00000000-00000000-00000000-00000000"
                    },
                    {
                        "Guid": "AAAA0000-00000000-00000000-00000002",
                        "Name": "UnreferencedPossessable",
                        "ParentGuid": "00000000-00000000-00000000-00000000"
                    }
                ],
                "ObjectBindings": [],
                "Tracks": [
                    { "ObjectName": "MovieSceneEventTrack'SEQ_Partial:MovieScene_0.MissingTrack'" }
                ]
            }
        }
    ]);
    fs::write(
        seq_dir.join("SEQ_Partial.json"),
        serde_json::to_vec_pretty(&partial_seq).expect("serialize partial"),
    )
    .expect("write partial json");

    let test_db = build_temp_db(
        "sequences-issues",
        BuildInputs {
            full: Some(dir.path().to_path_buf()),
            ..BuildInputs::default()
        },
        Some(vec![Stage::Sequences]),
    )
    .expect("build must succeed despite issues");

    let db = test_db.db();
    let seq = db
        .sequence("/Game/Sequencer/Event/Broken/SEQ_Partial")
        .expect("query partial")
        .expect("partial sequence must still be imported");
    assert_eq!(seq.duration_secs, 0.0);
    assert_eq!(seq.binding_count, 2);

    let issues = db.build_issues().expect("read build_issues");
    let kinds: Vec<&str> = issues.iter().map(|i| i.kind.as_str()).collect();
    assert!(kinds.contains(&"parse_error"), "kinds={kinds:?}");
    assert!(kinds.contains(&"missing_playback_range"), "kinds={kinds:?}");
    assert!(kinds.contains(&"dangling_object_ref"), "kinds={kinds:?}");
    assert!(kinds.contains(&"missing_binding_ref"), "kinds={kinds:?}");
}

#[test]
fn real_export_chapter_one_sequence_smoke_test() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sample_file = repo_root.join(
        ".deps/ffr/export-full/FFRS/Content/Sequencer/Event/Main_Event/C01/Ev_C01_001A/SEQ_Ev_C01_001A_01_MasterSequencer.json",
    );
    if !sample_file.is_file() {
        // Real export data is not present in CI or clean checkouts; skip automatically.
        return;
    }
    let parsed = ffr_data::assets::sequence::load_level_sequence_file(&sample_file, None)
        .expect("parse real chapter 1 sequence")
        .expect("must contain LevelSequence");
    assert_eq!(parsed.sequence.category, SequenceCategory::Event);
    assert!(parsed.sequence.duration_secs > 50.0);
    assert_eq!(parsed.sequence.binding_count, 3);
    assert!(
        parsed.bindings.iter().any(
            |b| b.template_class.as_deref() == Some("CPP_MapUnit") && b.map_unit_id == Some(60)
        )
    );
    assert!(!parsed.events.is_empty());
}
