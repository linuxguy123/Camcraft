//! M3.2 stage A acceptance: setups are entities like everything else -
//! one clamping, one stock, one datum system - and the whole document
//! resolves through them by the laws in `crate::setup` and the commands'
//! module headers: materialization never renames, furniture is private
//! and lazily spawned, datum transfer clones, a setup with operations
//! does not delete, activation preserves operation membership, and
//! each operation stays in the setup where it was created.
//!
//! Gate map (M4.4): covers app::commands, app::geometry, app::model, app::operation, app::posting, app::rest, app::schema, app::setup, app::stock, app::tools, app::world.

use flecs_ecs::prelude::World;

use camcraft_app::commands::{
    ActivateSetup, AddTool, CommandError, CreateOperation, CreateSetup, DeleteOperation,
    DeleteSetup, GenerateOperation, History, MoveModel, SetSetupMachineCs, SetStock, SetWcsOrigin,
};
use camcraft_app::geometry::{IDENTITY_Q, Point3, quat_from_axis_angle};
use camcraft_app::model;
use camcraft_app::operation::{self, Height, HeightReference, OpParams, OpStatus, SrcRef};
use camcraft_app::project;
use camcraft_app::posting;
use camcraft_app::rest;
use camcraft_app::schema::{Location, PostRef, STOCK_NAME, WCS_SLOTS};
use camcraft_app::setup;
use camcraft_app::stock::StockShape;
use camcraft_app::tools::{catalog, Tool};
use camcraft_app::world::new_world;

fn catalog_tool(name: &str) -> Tool {
    catalog()
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| t)
        .expect("catalog entry")
}

fn at(x: f64, y: f64, z: f64) -> Location {
    Location {
        pos: Point3::new(x, y, z),
        rot: IDENTITY_Q,
    }
}

/// Face parameters for the 60x60x20 box stock with a 4 mm cutter - the
/// same fixture the operations tests face on.
fn face_params() -> OpParams {
    OpParams::Face { holes: Default::default(), tilt: None, blend_turns: false,
        stepover: 3.0,
        clearance_z: Height::fixed(25.0),
        cut_z: Height::fixed(19.5),
        top_z: None,
        stepdown: None,
        feed: 1200.0,
        plunge_feed: 300.0,
        climb: true,
    }
}

fn box_stock(world: &World, history: &mut History, dx: f64, dy: f64, dz: f64) {
    let cmd = SetStock::new(world, StockShape::Box { dx, dy, dz }, Location::default());
    history.execute(world, cmd.expect("box stock"));
}

fn add_tool(world: &World, history: &mut History, name: &str) {
    let cmd = AddTool::new(world, name, catalog_tool(name));
    history.execute(world, cmd.expect("catalog tool"));
}

fn create_face(world: &World, history: &mut History, name: &str, cut_z: f64) {
    let mut params = face_params();
    if let OpParams::Face { cut_z: c, .. } = &mut params {
        *c = Height::fixed(cut_z);
    }
    let cmd = CreateOperation::new(world, name, "EM-0400-2FL", SrcRef::Stock, params);
    history.execute(world, cmd.expect("create face op"));
    let cmd = GenerateOperation::new(world, name).expect("generate face op");
    history.execute(world, cmd);
}

fn run(
    world: &World,
    history: &mut History,
    cmd: Result<impl camcraft_app::commands::Command + 'static, CommandError>,
    what: &str,
) {
    history.execute(world, cmd.unwrap_or_else(|e| panic!("{what}: {e:?}")));
}

/// The command refuses at construction; the unit map keeps the
/// assertion from needing Debug on the success type.
fn refused<T>(cmd: Result<T, CommandError>) -> CommandError {
    match cmd {
        Ok(_) => panic!("construction must refuse"),
        Err(e) => e,
    }
}

fn status(world: &World, name: &str) -> OpStatus {
    let e = model::find_by_name(world, name).expect("entity");
    operation::status_of(world, &e)
}

#[test]
fn part_placements_and_stock_follow_only_their_own_setup() {
    let world = new_world();
    let mut history = History::default();
    let part = model::spawn(&world, "part");
    let stock_pose = |context: &str| {
        let stock = setup::stock_entity(&world, Some(context)).expect("stock");
        model::transform_of(&world, &stock).expect("stock pose")
    };
    box_stock(&world, &mut history, 20.0, 20.0, 10.0);
    run(&world, &mut history, CreateSetup::new(&world, None, None), "first setup");
    run(&world, &mut history, CreateSetup::new(&world, None, Some("Setup1")), "second setup");

    let second_pose = at(30.0, 0.0, 0.0);
    run(&world, &mut history, MoveModel::new(&world, "part", second_pose), "place second setup");
    posting::post_set(&world, &PostRef::Named("second-post".into()));
    assert_eq!(model::transform_of(&world, &part), Some(second_pose));
    assert_eq!(stock_pose("Setup2").pos.x, 30.0);
    assert_eq!(face_stock(&world, Some("Setup1")), face_stock(&world, Some("Setup2")),
        "both setups use the same stock entity");

    run(&world, &mut history, ActivateSetup::new(&world, Some("Setup1")), "show first setup");
    assert_eq!(model::transform_of(&world, &part), Some(Location::default()));
    assert_eq!(stock_pose("Setup1").pos.x, 0.0);
    assert_eq!(posting::post_of(&world), PostRef::Builtin);
    run(&world, &mut history, ActivateSetup::new(&world, Some("Setup2")), "show second setup");
    assert_eq!(model::transform_of(&world, &part), Some(second_pose));
    assert_eq!(stock_pose("Setup2").pos.x, 30.0);
    assert_eq!(posting::post_of(&world), PostRef::Named("second-post".into()));
    assert!(history.undo(&world));
    assert_eq!(model::transform_of(&world, &part), Some(Location::default()), "undo restores the previous view");
    assert_eq!(posting::post_of(&world), PostRef::Builtin);
    assert!(history.redo(&world));
    assert_eq!(model::transform_of(&world, &part), Some(second_pose));
    assert_eq!(posting::post_of(&world), PostRef::Named("second-post".into()));

    let path = std::env::temp_dir().join(format!("camcraft_setup_poses_{}.camproj", std::process::id()));
    project::save(&world, &path).expect("save setup poses");
    let reopened = new_world();
    project::load(&reopened, &path).expect("load setup poses");
    let _ = std::fs::remove_file(path);
    let loaded_part = model::find_by_name(&reopened, "part").expect("loaded part");
    assert_eq!(model::transform_of(&reopened, &loaded_part), Some(second_pose));
    assert_eq!(setup::stock_entity(&reopened, Some("Setup2"))
        .and_then(|entity| model::transform_of(&reopened, &entity)).map(|pose| pose.pos.x), Some(30.0));
    assert_eq!(posting::post_of(&reopened), PostRef::Named("second-post".into()));
    let back = ActivateSetup::new(&reopened, Some("Setup1")).expect("activate loaded first setup");
    camcraft_app::commands::Command::apply(&back, &reopened);
    assert_eq!(model::transform_of(&reopened, &loaded_part), Some(Location::default()));
    assert_eq!(setup::stock_entity(&reopened, Some("Setup1"))
        .and_then(|entity| model::transform_of(&reopened, &entity)).map(|pose| pose.pos.x), Some(0.0));
    assert_eq!(posting::post_of(&reopened), PostRef::Builtin);
}

#[test]
fn rotating_a_setup_turns_shared_stock_and_its_height_references() {
    let world = new_world();
    let mut history = History::default();
    model::spawn(&world, "part");
    box_stock(&world, &mut history, 60.0, 20.0, 10.0);
    run(&world, &mut history, CreateSetup::new(&world, None, None), "first setup");
    run(&world, &mut history, CreateSetup::new(&world, None, None), "second setup");
    let turned = Location {
        pos: Point3::new(0.0, 0.0, 0.0),
        rot: quat_from_axis_angle(1, std::f64::consts::FRAC_PI_2),
    };
    run(&world, &mut history, MoveModel::new(&world, "part", turned), "turn Setup2");
    let stock = face_stock(&world, Some("Setup2"));
    assert_eq!(model::transform_of(&world, &stock).expect("stock pose").rot, turned.rot);
    let top = Height::of(HeightReference::TopOfStock, 0.0);
    assert!((top.z_for(&world, Some("Setup2")).expect("top") - 30.0).abs() < 1e-6);

    run(&world, &mut history, ActivateSetup::new(&world, Some("Setup1")), "first setup");
    assert_eq!(face_stock(&world, Some("Setup1")), stock);
    assert_eq!(model::transform_of(&world, &stock).expect("stock pose").rot, IDENTITY_Q);
    assert!((top.z_for(&world, Some("Setup1")).expect("top") - 10.0).abs() < 1e-6);
    run(&world, &mut history, ActivateSetup::new(&world, Some("Setup2")), "second setup");
    assert_eq!(model::transform_of(&world, &stock).expect("stock pose").rot, turned.rot);
    assert!((top.z_for(&world, Some("Setup2")).expect("top") - 30.0).abs() < 1e-6);
}

fn op_setup(world: &World, name: &str) -> Option<String> {
    let e = model::find_by_name(world, name).expect("entity");
    model::operation_of(world, &e).expect("operation").setup
}

/// Stock, tool, one materialized setup and one private one, the private
/// stock set to the same 60x60x20 box at the same spot: the pair every
/// cross-setup law is judged against.
fn two_setup_world() -> (World, History) {
    let world = new_world();
    let mut history = History::default();
    box_stock(&world, &mut history, 60.0, 60.0, 20.0);
    add_tool(&world, &mut history, "EM-0400-2FL");
    run(&world, &mut history, CreateSetup::new(&world, None, None), "materialize");
    run(&world, &mut history, CreateSetup::new(&world, None, None), "second setup");
    box_stock(&world, &mut history, 60.0, 60.0, 20.0);
    (world, history)
}

#[test]
fn the_first_setup_materializes_and_changes_nothing() {
    // The whole compat law, in one run: a generated path predating setups
    // must still read Ok, byte-identical, after the first CreateSetup -
    // because Setup1 POINTS at the existing entities instead of renaming
    // them.
    let world = new_world();
    let mut history = History::default();
    box_stock(&world, &mut history, 60.0, 60.0, 20.0);
    add_tool(&world, &mut history, "EM-0400-2FL");
    create_face(&world, &mut history, "Face1", 19.5);
    let face_e = model::find_by_name(&world, "Face1").expect("entity");
    let before = model::toolpath_of(&world, &face_e).expect("cached path");
    assert_eq!(status(&world, "Face1"), OpStatus::Ok);

    run(&world, &mut history, CreateSetup::new(&world, None, None), "create");
    assert_eq!(setup::names(&world), vec!["Setup1".to_string()]);
    assert_eq!(setup::active(&world).as_deref(), Some("Setup1"));
    let def = setup::def_of(&world, "Setup1").expect("def");
    assert_eq!(def.stock, STOCK_NAME, "materialization points, never renames");
    assert_eq!(def.wcs, WCS_SLOTS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(def.datum_from, "");
    assert!(model::find_by_name(&world, "Setup1/Stock").is_none(), "no private copies");
    assert!(model::find_by_name(&world, "Setup1/G54").is_none());

    let after = model::toolpath_of(&world, &face_e).expect("cached path");
    assert_eq!(before.path.moves.len(), after.path.moves.len());
    assert_eq!(status(&world, "Face1"), OpStatus::Ok, "the stamp still answers");
    assert_eq!(op_setup(&world, "Face1"), None, "creation never retro-declares");
    assert_eq!(setup::stock_entity_name(&world, Some("Setup1")), STOCK_NAME);
    assert!(setup::stock_entity(&world, None).is_some_and(|e| e == face_stock(&world, None)));

    // Undo drops the setup entity but not the furniture it merely named.
    assert!(history.undo(&world));
    assert!(setup::names(&world).is_empty());
    assert!(model::find_by_name(&world, STOCK_NAME).is_some());
    assert_eq!(status(&world, "Face1"), OpStatus::Ok);
    assert!(history.redo(&world));
    assert_eq!(setup::names(&world), vec!["Setup1".to_string()]);
}

fn face_stock(world: &World, setup: Option<&str>) -> flecs_ecs::prelude::Entity {
    setup::stock_entity(world, setup).expect("stock entity")
}

#[test]
fn new_setups_share_stock_and_keep_private_legacy_offsets() {
    let (world, mut history) = two_setup_world();
    assert_eq!(setup::names(&world), vec!["Setup1".to_string(), "Setup2".to_string()]);
    let def = setup::def_of(&world, "Setup2").expect("def");
    assert_eq!(def.stock, STOCK_NAME);
    assert_eq!(def.wcs[0], "Setup2/G54");
    // Stock is one shared solid; the old placed-offset entities remain
    // private when explicitly edited.
    assert!(setup::stock_entity(&world, Some("Setup2")).is_some());
    assert_eq!(face_stock(&world, Some("Setup1")), face_stock(&world, Some("Setup2")));
    assert!(setup::wcs_entity(&world, Some("Setup2"), 0).is_none());
    assert!(setup::wcs_entity(&world, None, 0).is_none(), "legacy G54 never placed");

    run(
        &world,
        &mut history,
        SetWcsOrigin::new(&world, "G54", at(10.0, 0.0, 0.0)),
        "place G54 in Setup2",
    );
    let placed = setup::wcs_entity(&world, Some("Setup2"), 0);
    assert!(placed.is_some());
    let entity = model::find_by_name(&world, "Setup2/G54").expect("furniture");
    assert_eq!(placed, Some(entity));
    assert_eq!(model::transform_of(&world, &entity).expect("loc").pos.x, 10.0);
    // The G-code WORD stays the literal; only the lookup followed the setup.
    assert_eq!(setup::wcs_entity_name(&world, Some("Setup2"), 0).as_deref(), Some("Setup2/G54"));
    assert!(model::find_by_name(&world, "G54").is_none());

    // A stock edit changes the one shared definition.
    run(&world, &mut history, SetStock::new(&world, StockShape::Box { dx: 40.0, dy: 40.0, dz: 30.0 }, Location::default()), "grow Setup2 stock");
    let shared = model::find_by_name(&world, STOCK_NAME).expect("shared stock");
    assert_eq!(
        model::stock_def_of(&world, &shared).expect("shape"),
        StockShape::Box { dx: 40.0, dy: 40.0, dz: 30.0 }
    );
}

#[test]
fn explicit_setups_use_toolpath_zero_for_work_offset_heights() {
    let (world, mut history) = two_setup_world();
    run(
        &world,
        &mut history,
        SetWcsOrigin::new(&world, "G54", at(0.0, 0.0, 5.0)),
        "place G54 in Setup2",
    );
    let h = Height::of(HeightReference::WorkOffset(0), 0.0);
    assert_eq!(h.z_for(&world, Some("Setup2")), Some(0.0));
    assert_eq!(h.z_for(&world, None), Some(0.0));
    // A physical register is selected at the machine; it does not add
    // another virtual translation to the setup's Toolpath CS.
    run(&world, &mut history, ActivateSetup::new(&world, Some("Setup1")), "activate Setup1");
    assert_eq!(h.z_for(&world, Some("Setup2")), Some(0.0));
    let top = Height::of(HeightReference::TopOfStock, 0.0);
    assert_eq!(top.z_for(&world, None), Some(20.0));
    assert_eq!(top.z_for(&world, Some("Setup2")), Some(20.0), "same box, same top");
}

#[test]
fn materializing_a_legacy_offset_keeps_its_origin_arithmetic() {
    let world = new_world();
    let mut history = History::default();
    box_stock(&world, &mut history, 60.0, 60.0, 20.0);
    run(&world, &mut history, SetWcsOrigin::new(&world, "G55", at(0.0, 0.0, 5.0)), "legacy G55");
    run(&world, &mut history, CreateSetup::new(&world, None, None), "materialize");
    assert!(!setup::def_of(&world, "Setup1").expect("setup").modern_cs);
    let h = Height::of(HeightReference::WorkOffset(1), 2.0);
    assert_eq!(h.z_for(&world, Some("Setup1")), Some(7.0));
}

#[test]
fn program_order_is_setup_rank_then_name() {
    // Setup2's `A_Op` is alphabetically first but machined second: rank
    // dominates name, and a nameless (None) op shares rank 0 with the
    // first setup - the implicit law ordering exactly as it resolves.
    let (world, mut history) = two_setup_world();
    create_face(&world, &mut history, "Z_Face", 19.5); // joins Setup2 (active)
    run(&world, &mut history, ActivateSetup::new(&world, Some("Setup1")), "back to Setup1");
    create_face(&world, &mut history, "Y_Face", 19.5); // joins Setup1
    let order: Vec<String> = setup::operation_order(&world).into_iter().map(|(n, _)| n).collect();
    assert_eq!(order, vec!["Y_Face".to_string(), "Z_Face".to_string()]);
    assert!(setup::active(&world).as_deref() == Some("Setup1"));
    // Each setup's post contains only its own operations, in that order.
    let posted: Vec<String> = camcraft_app::posting::program_ops(&world)
        .expect("program")
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(posted, vec!["Y_Face".to_string()]);
    run(&world, &mut history, ActivateSetup::new(&world, Some("Setup2")), "post Setup2");
    let posted: Vec<String> = camcraft_app::posting::program_ops(&world)
        .expect("second program").into_iter().map(|(n, _)| n).collect();
    assert_eq!(posted, vec!["Z_Face".to_string()]);
}

#[test]
fn machine_cs_selects_the_posted_register_without_moving_the_toolpath() {
    let (world, mut history) = two_setup_world();
    create_face(&world, &mut history, "Face2", 19.5);
    let before = model::find_by_name(&world, "Face2")
        .and_then(|entity| model::toolpath_of(&world, &entity))
        .expect("generated path");
    run(&world, &mut history, SetSetupMachineCs::new(&world, "Setup2", 1), "choose G55");
    let records = posting::stream_records(&world).expect("active setup stream");
    assert!(records.iter().any(|record| matches!(record,
        camcraft_app::post::cnc::Record::Param { name, value }
            if name == "WorkOffset" && value.as_str() == Some("G55"))));
    let after = model::find_by_name(&world, "Face2")
        .and_then(|entity| model::toolpath_of(&world, &entity))
        .expect("same path");
    assert_eq!(before.path, after.path);
    let path = std::env::temp_dir().join(format!("camcraft_machine_cs_{}.camproj", std::process::id()));
    project::save(&world, &path).expect("save machine CS");
    let reopened = new_world();
    project::load(&reopened, &path).expect("load machine CS");
    let _ = std::fs::remove_file(path);
    assert_eq!(setup::def_of(&reopened, "Setup2").expect("setup").machine_wcs, 1);
}

#[test]
fn datum_transfer_clones_the_world_and_undoes_exactly() {
    let (world, mut history) = two_setup_world();
    run(&world, &mut history, SetWcsOrigin::new(&world, "G54", at(12.0, 0.0, 3.0)), "place offset");
    run(
        &world,
        &mut history,
        CreateSetup::new(&world, None, Some("Setup2")),
        "transfer datum",
    );
    assert_eq!(setup::names(&world).len(), 3);
    assert_eq!(setup::active(&world).as_deref(), Some("Setup3"));
    let def = setup::def_of(&world, "Setup3").expect("def");
    assert_eq!(def.datum_from, "Setup2", "provenance rides the setup");
    assert_eq!(def.stock, STOCK_NAME);
    // The setup inherits the active placement but references the same stock.
    let s3 = setup::stock_entity(&world, Some("Setup3")).expect("shared stock");
    let s2 = setup::stock_entity(&world, Some("Setup2")).expect("source stock");
    assert_eq!(s3, s2);
    assert_eq!(
        model::stock_def_of(&world, &s3).expect("shape"),
        model::stock_def_of(&world, &s2).expect("shape")
    );
    assert_eq!(
        model::transform_of(&world, &s3).expect("loc"),
        model::transform_of(&world, &s2).expect("loc")
    );
    let w3 = setup::wcs_entity(&world, Some("Setup3"), 0).expect("cloned offset");
    let w2 = setup::wcs_entity(&world, Some("Setup2"), 0).expect("source offset");
    assert_ne!(w3, w2);
    assert_eq!(model::transform_of(&world, &w3).expect("loc").pos.z, 3.0);

    assert!(history.undo(&world));
    assert_eq!(setup::names(&world), vec!["Setup1".to_string(), "Setup2".to_string()]);
    assert!(model::find_by_name(&world, STOCK_NAME).is_some());
    assert!(model::find_by_name(&world, "Setup3/G54").is_none());
    // The SOURCE's furniture survived the inverse untouched - a clone
    // removes only its own copies.
    assert!(setup::stock_entity(&world, Some("Setup2")).is_some());
    assert!(setup::wcs_entity(&world, Some("Setup2"), 0).is_some());
    assert_eq!(setup::active(&world).as_deref(), Some("Setup2"));
    assert!(history.redo(&world));
    assert!(setup::stock_entity(&world, Some("Setup3")).is_some());
    assert!(setup::wcs_entity(&world, Some("Setup3"), 0).is_some());
}

#[test]
fn a_setup_with_operations_does_not_delete() {
    let (world, mut history) = two_setup_world();
    run(&world, &mut history, ActivateSetup::new(&world, Some("Setup1")), "Setup1 active");
    create_face(&world, &mut history, "Face1", 19.5);
    run(&world, &mut history, ActivateSetup::new(&world, Some("Setup2")), "Setup2 active");
    create_face(&world, &mut history, "Face2", 19.5);

    let err = refused(DeleteSetup::new(&world, "Setup1"));
    match err {
        CommandError::Invalid(msg) => {
            assert!(msg.contains("Face1") && msg.contains("Setup1"), "names both: {msg}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    assert!(setup::exists(&world, "Setup1"), "refusal writes nothing");

    // Operations stay in their creating setup. Once Face1 is deleted,
    // the first setup can be deleted while the shared stock survives.
    run(&world, &mut history, DeleteOperation::new(&world, "Face1"), "empty Setup1");
    run(&world, &mut history, DeleteSetup::new(&world, "Setup1"), "delete");
    assert_eq!(setup::names(&world), vec!["Setup2".to_string()]);
    assert!(model::find_by_name(&world, STOCK_NAME).is_some(), "shared furniture stays");
    assert_eq!(
        setup::active(&world).as_deref(),
        Some("Setup2"),
        "a dangling cursor reads as the first that exists"
    );

    // Once Setup2's operation is removed, it is the last setup and
    // must remain. The shared stock stays with the document.
    run(&world, &mut history, DeleteOperation::new(&world, "Face2"), "empty it");
    assert!(matches!(refused(DeleteSetup::new(&world, "Setup2")), CommandError::Invalid(_)));
    assert!(model::find_by_name(&world, "Setup2").is_some());
    assert!(model::find_by_name(&world, STOCK_NAME).is_some());
    let shared_stock = model::find_by_name(&world, STOCK_NAME).expect("shared stock remains");
    assert_eq!(
        model::stock_def_of(&world, &shared_stock).expect("shape"),
        StockShape::Box { dx: 60.0, dy: 60.0, dz: 20.0 },
        "deleting an operation leaves the stock alone"
    );
    assert_eq!(setup::names(&world), vec!["Setup2".to_string()]);
}

#[test]
fn activation_is_inert_data() {
    let (world, mut history) = two_setup_world();
    create_face(&world, &mut history, "Face2", 19.5); // joins Setup2
    let face_e = model::find_by_name(&world, "Face2").expect("entity");
    let stamp_before = model::operation_of(&world, &face_e).expect("op").stamp;
    let moves_before = model::toolpath_of(&world, &face_e).expect("path").path.moves.len();

    run(&world, &mut history, ActivateSetup::new(&world, Some("Setup1")), "activate");
    assert_eq!(setup::stored_active(&world).as_deref(), Some("Setup1"));
    let face_e = model::find_by_name(&world, "Face2").expect("entity");
    assert_eq!(
        model::operation_of(&world, &face_e).expect("op").stamp,
        stamp_before,
        "a cursor move digests into nothing"
    );
    assert_eq!(status(&world, "Face2"), OpStatus::Ok);
    assert_eq!(model::toolpath_of(&world, &face_e).expect("path").path.moves.len(), moves_before);
    assert_eq!(op_setup(&world, "Face2"), Some("Setup2".to_string()), "membership is untouched");

    // The one thing activation changes: where NEW things join.
    create_face(&world, &mut history, "Face3", 19.5);
    assert_eq!(op_setup(&world, "Face3"), Some("Setup1".to_string()));
    assert!(history.undo(&world)); // Face3's generate
    assert!(history.undo(&world)); // Face3's create
    assert!(history.undo(&world)); // the activation
    assert_eq!(setup::stored_active(&world).as_deref(), Some("Setup2"));
    assert!(model::find_by_name(&world, "Face3").is_none(), "Face3 gone");
}

#[test]
fn setup_names_are_validated_at_construction() {
    let world = new_world();
    // Reserved furniture names and the separator belong to the scheme.
    for bad in ["Stock", "G54", "G59", "Setup1/G54", "", "  "] {
        assert!(
            matches!(refused(CreateSetup::new(&world, Some(bad), None)), CommandError::Invalid(_)),
            "'{bad}' must be refused"
        );
    }
    // Tools cannot squat furniture names either (the law is shared).
    let tool = catalog_tool("EM-0400-2FL");
    assert!(matches!(
        refused(AddTool::new(&world, "Setup9/Stock", tool.clone())),
        CommandError::Invalid(_) | CommandError::NameConflict(_)
    ));
    // One entity, one name: an existing entity's name conflicts.
    let mut history = History::default();
    box_stock(&world, &mut history, 60.0, 60.0, 20.0);
    assert!(matches!(
        refused(CreateSetup::new(&world, Some(STOCK_NAME), None)),
        CommandError::NameConflict(_) | CommandError::Invalid(_)
    ));
    // Materialization refuses a datum (there is nothing to transfer);
    // after the first setup, a named source that does not exist is NotFound.
    assert!(matches!(
        refused(CreateSetup::new(&world, None, Some("Setup0"))),
        CommandError::Invalid(_)
    ));
    run(&world, &mut history, CreateSetup::new(&world, None, None), "materialize");
    assert!(
        matches!(refused(CreateSetup::new(&world, None, Some("Nope"))), CommandError::NotFound(_))
    );
    assert!(
        matches!(refused(ActivateSetup::new(&world, Some("Nope"))), CommandError::NotFound(_))
    );
    assert!(matches!(refused(DeleteSetup::new(&world, "Nope")), CommandError::NotFound(_)));
    // `DeleteSetup` on a non-setup entity is Invalid, not a silent no-op.
    assert!(matches!(
        refused(DeleteSetup::new(&world, STOCK_NAME)),
        CommandError::Invalid(_) | CommandError::NotFound(_)
    ));
}

#[test]
fn rest_masks_are_same_setup_only() {
    // An operation's rest digest covers the program AHEAD of it IN ITS
    // OWN SETUP: another setup's cuts belong to another block of metal.
    // The proof is equality: Face2's mask is the same whether or not a
    // first setup machined anything at all.
    let build = |with_face1: bool| {
        let (world, mut history) = two_setup_world();
        if with_face1 {
            run(&world, &mut history, ActivateSetup::new(&world, Some("Setup1")), "active 1");
            create_face(&world, &mut history, "Face1", 19.5);
            run(&world, &mut history, ActivateSetup::new(&world, Some("Setup2")), "active 2");
        }
        create_face(&world, &mut history, "A_Rough", 10.0);
        create_face(&world, &mut history, "Face2", 19.5);
        assert_eq!(status(&world, "Face2"), OpStatus::Ok);
        let d = rest::prior_digest(&world, "Face2", Some("Setup2"));
        assert!(d.is_some(), "A_Rough is ahead in the same setup");
        let d1 = rest::prior_digest(&world, "Face1", Some("Setup1"));
        (world, d, d1)
    };
    let (w0, d_without, d1_without) = build(false);
    let (w1, d_with, d1_with) = build(true);
    assert_eq!(d_with, d_without, "Setup1's program is not Setup2's rest");
    assert_eq!(d1_with, None, "nothing machined ahead of Face1 in Setup1");
    assert_eq!(d1_without, None);
    // And the ORDER law agrees face-to-face: in w1 the post walks
    // Face1 before the Setup2 pair - ordering across setups exists,
    // masking across setups does not.
    let order: Vec<String> = setup::operation_order(&w1).into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        order,
        vec!["Face1".to_string(), "A_Rough".to_string(), "Face2".to_string()]
    );
    let _ = w0;
}

#[test]
fn deleting_the_active_furniture_owner_falls_the_cursor() {
    // Deleting the active setup selects the first remaining setup.
    let (world, mut history) = two_setup_world();
    assert_eq!(setup::active(&world).as_deref(), Some("Setup2"));
    run(&world, &mut history, DeleteSetup::new(&world, "Setup2"), "delete active");
    assert_eq!(setup::stored_active(&world).as_deref(), Some("Setup1"));
    assert_eq!(setup::active(&world).as_deref(), Some("Setup1"));
    assert!(history.undo(&world));
    assert_eq!(setup::active(&world).as_deref(), Some("Setup2"));
}
