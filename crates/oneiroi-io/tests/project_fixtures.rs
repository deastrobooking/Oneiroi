use std::fs;
use std::path::{Path, PathBuf};

use oneiroi_graph::{CompileBudget, GraphCompiler, GraphRevision, builtin_registry};
use oneiroi_io::{
    AudioAnalysisProject, BlendModeProject, ControlTargetProject, EffectProject,
    MasterEffectKindProject, MasterEffectsProject, MasterModulationProject, OutputProject,
    PROJECT_VERSION, ProjectFile, SourceModeProject, ThemeProject, TransformProject, load_project,
    save_project_atomic,
};

const V1_FIXTURE: &str = "tests/fixtures/project-v1.oneiroi";
const V2_FIXTURE: &str = "tests/fixtures/project-v2.oneiroi";
const V3_FIXTURE: &str = "tests/fixtures/project-v3.oneiroi";
const V4_FIXTURE: &str = "tests/fixtures/project-v4.oneiroi";
const V5_FIXTURE: &str = "tests/fixtures/project-v5.oneiroi";

fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

#[test]
fn v1_golden_project_migrates_to_v5_and_round_trips() {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/project-v1.oneiroi")).unwrap();
    assert_eq!(raw["version"], 1);
    assert!(raw.get("project_id").is_none());
    assert!(raw["settings"].get("output").is_none());
    assert!(raw["decks"][0].get("transform").is_none());
    assert!(raw["decks"][0]["effects"].get("slots").is_none());

    let project = load_project(fixture_path(V1_FIXTURE)).unwrap();
    assert_eq!(project.version, PROJECT_VERSION);
    assert!(
        project.project_id.len() == 32
            && project
                .project_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
    );
    assert!(project.graph.is_some());
    assert_eq!(project.settings.output, OutputProject::default());
    assert_eq!(
        project.settings.audio_analysis,
        AudioAnalysisProject::default()
    );
    assert_eq!(
        project.settings.master_effects,
        MasterEffectsProject::default()
    );
    assert_eq!(
        project.settings.master_modulation,
        MasterModulationProject::default()
    );
    assert_eq!(project.settings.theme, ThemeProject::default());
    assert!(project.midi_mappings.is_empty());

    assert_eq!(project.settings.bpm, 128.0);
    assert_eq!(project.settings.crossfader, 0.25);
    assert_eq!(project.settings.master_opacity, 0.8);
    assert_eq!(project.decks[0].clips[0], Some("media/legacy-a.mov".into()));
    assert_eq!(project.decks[0].selected_slot, 2);
    assert_eq!(project.decks[0].active_slot, Some(0));
    assert_eq!(project.decks[0].transport.position, 3.5);
    assert_eq!(project.decks[0].transform, TransformProject::default());
    assert_eq!(
        project.decks[0].effects.slots,
        EffectProject::default().slots
    );

    let round_trip_path = std::env::temp_dir().join(format!(
        "oneiroi-project-v1-golden-{}.oneiroi",
        std::process::id()
    ));
    save_project_atomic(&round_trip_path, &project).unwrap();
    let reloaded: ProjectFile = load_project(&round_trip_path).unwrap();
    assert_eq!(reloaded, project);
    fs::remove_file(round_trip_path).unwrap();
}

#[test]
fn v2_golden_project_migrates_to_v5_and_round_trips() {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/project-v2.oneiroi")).unwrap();
    assert_eq!(raw["version"], 2);
    assert!(raw.get("project_id").is_none());
    assert!(raw.get("graph").is_none());
    assert!(raw["settings"].get("theme").is_none());
    assert!(raw["settings"].get("master_modulation").is_none());
    assert!(
        raw["settings"]["master_effects"]["slots"][0]
            .get("package_id")
            .is_none()
    );

    let project = load_project(fixture_path(V2_FIXTURE)).unwrap();
    assert_eq!(project.version, PROJECT_VERSION);
    assert!(project.graph.is_some());
    assert_eq!(project.settings.bpm, 132.0);
    assert_eq!(project.settings.output.composition_extent, [1280, 720]);
    assert!(project.settings.output.fullscreen);
    assert_eq!(project.settings.output.display_id, "legacy-stage-display");
    assert_eq!(project.settings.audio_analysis.gain, 1.5);
    assert!(project.settings.audio_analysis.normalization);
    assert_eq!(
        project.settings.master_effects.slots[0].kind,
        MasterEffectKindProject::Blur
    );
    assert_eq!(project.settings.master_effects.slots[0].amount, 12.0);
    assert!(
        project.settings.master_effects.slots[0]
            .package_id
            .is_empty()
    );
    assert!(
        project.settings.master_effects.slots[0]
            .parameters
            .is_empty()
    );
    assert_eq!(
        project.settings.master_modulation,
        MasterModulationProject::default()
    );
    assert_eq!(project.settings.theme, ThemeProject::default());

    let deck = &project.decks[0];
    assert_eq!(deck.clips[1], Some("media/v2-feature.mov".into()));
    assert_eq!(deck.clip_playback[1].in_point, 1.5);
    assert_eq!(deck.clip_playback[1].out_point, Some(9.0));
    assert_eq!(deck.transform.source_mode, SourceModeProject::Fit);
    assert_eq!(deck.blend_mode, BlendModeProject::Screen);
    assert!(deck.solo);
    assert_eq!(deck.effects.slots[1].mix, 0.8);

    let round_trip_path = std::env::temp_dir().join(format!(
        "oneiroi-project-v2-golden-{}.oneiroi",
        std::process::id()
    ));
    save_project_atomic(&round_trip_path, &project).unwrap();
    let reloaded: ProjectFile = load_project(&round_trip_path).unwrap();
    assert_eq!(reloaded, project);
    fs::remove_file(round_trip_path).unwrap();
}

#[test]
fn v3_golden_project_migrates_to_v5_and_round_trips() {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/project-v3.oneiroi")).unwrap();
    assert_eq!(raw["version"], 3);
    assert!(raw.get("project_id").is_none());
    assert!(raw.get("takes").is_none());
    assert!(raw.get("graph").is_none());
    assert!(raw.get("random_seeds").is_none());
    assert_eq!(
        raw["settings"]["master_effects"]["slots"][0]["package_id"],
        "chromatic-split"
    );

    let project = load_project(fixture_path(V3_FIXTURE)).unwrap();
    assert_eq!(project.version, PROJECT_VERSION);
    assert!(project.graph.is_some());
    assert!(project.takes.is_empty());
    assert!(project.random_seeds.is_empty());
    assert_eq!(project.settings.theme, ThemeProject::default());

    let custom = &project.settings.master_effects.slots[0];
    assert_eq!(custom.kind, MasterEffectKindProject::Custom);
    assert_eq!(custom.package_id, "chromatic-split");
    assert_eq!(custom.parameters.len(), 2);
    assert_eq!(custom.parameters[0].id, "amount");
    assert_eq!(custom.parameters[0].value, 0.025);
    assert!(project.settings.master_modulation.lfos[0].enabled);
    assert_eq!(
        project.settings.master_modulation.routes[0].parameter_key,
        4242
    );
    assert_eq!(project.settings.master_modulation.routes[0].amount, -0.6);
    assert_eq!(project.midi_mappings.len(), 1);
    assert_eq!(
        project.midi_mappings[0].target,
        ControlTargetProject::MasterEffectParameter {
            slot: 0,
            parameter_key: 4242,
        }
    );

    let round_trip_path = std::env::temp_dir().join(format!(
        "oneiroi-project-v3-golden-{}.oneiroi",
        std::process::id()
    ));
    save_project_atomic(&round_trip_path, &project).unwrap();
    let reloaded: ProjectFile = load_project(&round_trip_path).unwrap();
    assert_eq!(reloaded, project);
    fs::remove_file(round_trip_path).unwrap();
}

#[test]
fn v4_golden_project_migrates_to_v5_and_round_trips() {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/project-v4.oneiroi")).unwrap();
    assert_eq!(raw["version"], 4);
    assert_eq!(raw["project_id"], "11111111111111111111111111111111");
    assert!(raw.get("graph").is_none());
    assert!(raw.get("random_seeds").is_none());

    let project = load_project(fixture_path(V4_FIXTURE)).unwrap();
    assert_eq!(project.version, PROJECT_VERSION);
    assert_eq!(project.project_id, "11111111111111111111111111111111");
    assert_eq!(project.takes.len(), 1);
    assert_eq!(project.takes[0].take_id, "22222222222222222222222222222222");
    assert_eq!(project.takes[0].name, "V4 opening take");
    assert_eq!(project.takes[0].journal_file, "session-v4-opening.jsonl");
    assert_eq!(project.takes[0].created_unix_ms, 1_785_528_000_000);
    assert!(project.graph.is_some());
    assert!(project.random_seeds.is_empty());
    assert_eq!(project.decks[0].clips[2], Some("media/v4-take.mov".into()));
    assert_eq!(
        project.settings.master_effects.slots[0].package_id,
        "chromatic-split"
    );

    let round_trip_path = std::env::temp_dir().join(format!(
        "oneiroi-project-v4-golden-{}.oneiroi",
        std::process::id()
    ));
    save_project_atomic(&round_trip_path, &project).unwrap();
    let reloaded: ProjectFile = load_project(&round_trip_path).unwrap();
    assert_eq!(reloaded, project);
    fs::remove_file(round_trip_path).unwrap();
}

#[test]
fn v5_golden_project_loads_compiles_and_round_trips_without_migration() {
    let source: ProjectFile =
        serde_json::from_str(include_str!("fixtures/project-v5.oneiroi")).unwrap();
    source.validate().unwrap();
    assert_eq!(source.version, PROJECT_VERSION);

    let project = load_project(fixture_path(V5_FIXTURE)).unwrap();
    assert_eq!(project, source);
    assert_eq!(project.project_id, "33333333333333333333333333333333");
    assert_eq!(project.takes[0].take_id, "44444444444444444444444444444444");
    assert_eq!(project.random_seeds.get("particles"), Some(&42));
    assert_eq!(project.random_seeds.get("show.primary"), Some(&8_675_309));
    assert_eq!(project.settings.theme.preset, "ultraviolet");
    assert_eq!(
        project.settings.midi_devices,
        vec!["v5-performance-controller"]
    );

    let graph = project
        .graph
        .as_ref()
        .expect("v5 fixture carries its graph");
    assert_eq!(graph.revision, GraphRevision(7));
    let plan = GraphCompiler::new(&builtin_registry(), CompileBudget::default())
        .compile(graph)
        .unwrap();
    assert_eq!(plan.nodes().len(), 11);
    assert_eq!(plan.edges().len(), 10);

    let round_trip_path = std::env::temp_dir().join(format!(
        "oneiroi-project-v5-golden-{}.oneiroi",
        std::process::id()
    ));
    save_project_atomic(&round_trip_path, &project).unwrap();
    let reloaded: ProjectFile = load_project(&round_trip_path).unwrap();
    assert_eq!(reloaded, project);
    fs::remove_file(round_trip_path).unwrap();
}
