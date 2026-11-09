//! Setup commands: create, delete, activate, and choose a machine CS.
//!
//! Provenance: our own design; one clamping = one datum system is
//! standard machine-shop practice, specced from general machining
//! knowledge - no vendor implementation inspected. Like every sibling
//! here, construction reads the world, validates, and snapshots; `apply`
//! and `invert` are pure writes, so undo/redo replay bytes and never
//! re-derive.
//!
//! ## The laws these commands keep
//!
//! - **Materialization, never renaming.** Creating the first setup on a
//!   document that has none points it at the existing `Stock`/`G54`..
//!   entities ([`crate::setup`]'s implicit-setup law). Nothing is
//!   renamed: every cached stamp, path and saved file predating setups
//!   keeps resolving, byte for byte.
//! - **Datum transfer = clone, not alias.** Transferring a datum copies
//!   the source setup's stock (parameters, placement, facets) and its
//!   `G54` placement into the new setup's OWN entities. A shop then
//!   re-places the copy to describe the re-clamp - placement IS the
//!   flip (M3.3 adds the rotated form). `datum_from` records where the
//!   clone came from; it is sheet provenance, inert for every stamp.
//! - **A setup with operations is not deletable.** [`DeleteSetup`]
//!   refuses while any operation belongs to it ([`crate::setup::belongs`],
//!   by declaration or by the first/implicit position), naming the
//!   operation. Furniture the setup alone owns goes with it; names the
//!   legacy world or another setup also points at stay.
//! - **Activation restores the setup context.** The shared model gets
//!   the selected setup's part placements; machine and post choices
//!   follow it. The other setup's cached paths stay in their entities.
//! - **Operation membership is fixed at creation.** Each operation
//!   belongs to the setup active when it was created.

use flecs_ecs::core::EntityViewGet; // try_get is not re-exported in prelude (0.2.2)
use flecs_ecs::prelude::World;

use crate::commands::{Command, CommandError};
use crate::geometry::Triangle;
use crate::model;
use crate::schema::{Location, SetupDef, StockDef, Wcs, STOCK_NAME, WCS_SLOTS};
use crate::setup;
use crate::stock::StockShape;

/// A setup's stock as the transfer copies it: entity name, parameters,
/// placement, and the local facets a `FromModel` stock mirrors.
struct StockSnap {
    name: String,
    shape: StockShape,
    location: Location,
    facets: std::sync::Arc<[Triangle]>,
}

/// Create a setup using the global stock, with an independent placement
/// of the workpiece. An optional datum source copies placed legacy offsets.
pub struct CreateSetup {
    name: String,
    def: SetupDef,
    wcs: Vec<(String, Location)>,
    /// The active setup before this command, restored by its inverse.
    prev_active: Option<String>,
    prev_def: Option<(String, SetupDef)>,
    current_poses: Vec<(String, Location)>,
    current_context: crate::schema::SetupContext,
}

impl CreateSetup {
    /// `name` defaults to the next free `SetupN`. `datum_from` names an
    /// existing setup for legacy offset placement; `None` keeps those
    /// offsets unplaced. Stock is shared in either case.
    pub fn new(
        world: &World,
        name: Option<&str>,
        datum_from: Option<&str>,
    ) -> Result<Self, CommandError> {
        let name = match name {
            Some(n) => {
                if n.trim().is_empty() {
                    return Err(CommandError::Invalid("setup name must not be empty".into()));
                }
                if n.contains('/') {
                    return Err(CommandError::Invalid(
                        "a setup name must not contain '/' - that separator belongs to setup furniture".into(),
                    ));
                }
                n.to_string()
            }
            None => setup::next_setup_name(world).ok_or_else(|| {
                CommandError::Invalid("no free SetupN name - this document is out of setups".into())
            })?,
        };
        if model::find_by_name(world, &name).is_some() {
            return Err(CommandError::NameConflict(name.clone()));
        }
        if STOCK_NAME == name || WCS_SLOTS.contains(&name.as_str()) {
            return Err(CommandError::Invalid(format!(
                "'{name}' is reserved for setup furniture"
            )));
        }
        let materializing = setup::names(world).is_empty();
        let legacy_offsets_placed = (0..WCS_SLOTS.len())
            .any(|slot| setup::wcs_entity(world, None, slot).is_some());
        let current_poses = setup::model_poses(world);
        let current_context = setup::current_context(world);
        let inherited_context = datum_from
            .filter(|from| setup::active(world).as_deref() != Some(*from))
            .and_then(|from| setup::def_of(world, from))
            .and_then(|def| def.context)
            .unwrap_or_else(|| current_context.clone());
        let inherited_poses = datum_from
            .filter(|from| setup::active(world).as_deref() != Some(*from))
            .and_then(|from| setup::def_of(world, from))
            .map(|def| def.model_poses)
            .filter(|poses| !poses.is_empty())
            .unwrap_or_else(|| current_poses.clone());
        let prev_def = setup::active(world)
            .and_then(|active| setup::def_of(world, &active).map(|def| (active, def)));
        if materializing && datum_from.is_some() {
            return Err(CommandError::Invalid(
                "there is nothing to transfer a datum from yet".into(),
            ));
        }
        let def = if materializing {
            SetupDef {
                stock: STOCK_NAME.to_string(),
                wcs: WCS_SLOTS.iter().map(|s| s.to_string()).collect(),
                datum_from: String::new(),
                model_poses: inherited_poses.clone(),
                machine_wcs: 0,
                modern_cs: !legacy_offsets_placed,
                context: Some(inherited_context.clone()),
            }
        } else {
            SetupDef {
                stock: STOCK_NAME.to_string(),
                wcs: WCS_SLOTS.iter().map(|w| format!("{name}/{w}")).collect(),
                datum_from: datum_from.unwrap_or("").to_string(),
                model_poses: inherited_poses,
                machine_wcs: datum_from.and_then(|from| setup::def_of(world, from))
                    .map_or(0, |def| def.machine_wcs),
                modern_cs: true,
                context: Some(inherited_context),
            }
        };
        let mut wcs = Vec::new();
        if let Some(from) = datum_from {
            if !setup::exists(world, from) {
                return Err(CommandError::NotFound(from.to_string()));
            }
            for i in 0..WCS_SLOTS.len() {
                let Some(entity) = setup::wcs_entity(world, Some(from), i) else {
                    continue;
                };
                if let (Some(wname), Some(loc)) = (
                    setup::wcs_entity_name(world, Some(name.as_str()), i),
                    model::transform_of(world, &entity),
                ) {
                    wcs.push((wname, loc));
                }
            }
        }
        Ok(CreateSetup {
            name,
            def,
            wcs,
            prev_active: setup::active(world),
            prev_def,
            current_poses,
            current_context,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Command for CreateSetup {
    fn apply(&self, world: &World) {
        if let Some((name, def)) = &self.prev_def
            && let Some(entity) = model::find_by_name(world, name)
        {
            let mut updated = def.clone();
            updated.model_poses = self.current_poses.clone();
            updated.context = Some(self.current_context.clone());
            entity.entity_view(world).set(updated);
        }
        let entity = match model::find_by_name(world, &self.name) {
            Some(e) => e,
            None => model::spawn(world, &self.name),
        };
        entity.entity_view(world).set(self.def.clone());
        for (wname, loc) in &self.wcs {
            let e = match model::find_by_name(world, wname) {
                Some(e) => e,
                None => model::spawn(world, wname),
            };
            e.entity_view(world).add(Wcs);
            model::set_transform(world, &e, *loc);
        }
        // The new setup is the one you work in (module header: creation
        // activates; undo restores what was active).
        setup::restore_model_poses(world, &self.def.model_poses);
        if let Some(context) = &self.def.context {
            setup::restore_context(world, context);
        }
        setup::set_active(world, Some(self.name.clone()));
    }

    fn invert(&self, world: &World) {
        setup::restore_model_poses(world, &self.current_poses);
        setup::restore_context(world, &self.current_context);
        setup::set_active(world, self.prev_active.clone());
        if let Some((name, def)) = &self.prev_def
            && let Some(entity) = model::find_by_name(world, name)
        {
            entity.entity_view(world).set(def.clone());
        }
        for (wname, _) in &self.wcs {
            if let Some(e) = model::find_by_name(world, wname) {
                model::remove(world, &e);
            }
        }
        if let Some(e) = model::find_by_name(world, &self.name) {
            model::remove(world, &e);
        }
    }
}

/// Delete a setup, its private furniture, and its claim on the program.
/// Refused while any operation still machines in it.
pub struct DeleteSetup {
    name: String,
    def: SetupDef,
    /// Furniture that goes with the setup: entities this setup alone
    /// names. Shared legacy names (`Stock`, `G54`..) and anything
    /// another setup points at are NOT deleted.
    stock: Option<StockSnap>,
    wcs: Vec<(String, Location)>,
    prev_active: Option<String>,
    previous_poses: Vec<(String, Location)>,
    previous_context: crate::schema::SetupContext,
    next_active: Option<String>,
    next_poses: Vec<(String, Location)>,
    next_context: crate::schema::SetupContext,
}

impl DeleteSetup {
    pub fn new(world: &World, name: &str) -> Result<Self, CommandError> {
        let entity = model::find_by_name(world, name)
            .ok_or_else(|| CommandError::NotFound(name.to_string()))?;
        let def = entity
            .entity_view(world)
            .try_get::<&SetupDef>(|d| d.clone())
            .ok_or_else(|| CommandError::Invalid(format!("'{name}' is not a setup")))?;
        // Refusal: an operation that would orphan its own program
        // position. `belongs` is the single predicate the order law uses.
        for (op_name, op_e) in setup::operation_order(world) {
            let declared = model::operation_of(world, &op_e).and_then(|o| o.setup);
            if setup::belongs(world, declared.as_deref(), name) {
                return Err(CommandError::Invalid(format!(
                    "'{op_name}' machines in '{name}' - delete the operation first"
                )));
            }
        }
        if setup::names(world).len() == 1 {
            return Err(CommandError::Invalid("the last setup cannot be deleted".into()));
        }
        let owned = |n: &str| {
            n != STOCK_NAME && !WCS_SLOTS.contains(&n) && !setup::is_furniture_of_other(world, n, name)
        };
        let stock = if owned(&def.stock) {
            model::find_by_name(world, &def.stock).and_then(|e| {
                model::stock_def_of(world, &e).map(|shape| StockSnap {
                    name: def.stock.clone(),
                    shape,
                    location: model::transform_of(world, &e).unwrap_or_default(),
                    facets: model::facets_of(world, &e).unwrap_or_default(),
                })
            })
        } else {
            None
        };
        let mut wcs = Vec::new();
        for (i, wname) in def.wcs.iter().enumerate() {
            if i >= WCS_SLOTS.len() || !owned(wname) {
                continue;
            }
            if let Some(e) = model::find_by_name(world, wname)
                && model::is_wcs(world, &e)
                && let Some(loc) = model::transform_of(world, &e)
            {
                wcs.push((wname.clone(), loc));
            }
        }
        let prev_active = setup::active(world);
        let next_active = if prev_active.as_deref() == Some(name) {
            setup::names(world).into_iter().find(|candidate| candidate != name)
        } else {
            prev_active.clone()
        };
        let previous_poses = setup::model_poses(world);
        let previous_context = setup::current_context(world);
        let next_poses = next_active.as_deref()
            .filter(|_| prev_active.as_deref() == Some(name))
            .and_then(|candidate| setup::def_of(world, candidate))
            .map(|def| def.model_poses)
            .filter(|poses| !poses.is_empty())
            .unwrap_or_else(|| previous_poses.clone());
        let next_context = next_active.as_deref()
            .filter(|_| prev_active.as_deref() == Some(name))
            .and_then(|candidate| setup::def_of(world, candidate))
            .and_then(|def| def.context)
            .unwrap_or_else(|| previous_context.clone());
        Ok(DeleteSetup {
            name: name.to_string(),
            def,
            stock,
            wcs,
            prev_active,
            previous_poses,
            previous_context,
            next_active,
            next_poses,
            next_context,
        })
    }
}

impl Command for DeleteSetup {
    fn apply(&self, world: &World) {
        if let Some(s) = &self.stock
            && let Some(e) = model::find_by_name(world, &s.name)
        {
            model::remove(world, &e);
        }
        for (wname, _) in &self.wcs {
            if let Some(e) = model::find_by_name(world, wname) {
                model::remove(world, &e);
            }
        }
        if let Some(e) = model::find_by_name(world, &self.name) {
            model::remove(world, &e);
        }
        if self.prev_active.as_deref() == Some(self.name.as_str()) {
            setup::restore_model_poses(world, &self.next_poses);
            setup::restore_context(world, &self.next_context);
            setup::set_active(world, self.next_active.clone());
        }
        // The cursor follows the first remaining setup when its current
        // setup is deleted.
    }

    fn invert(&self, world: &World) {
        let entity = model::spawn(world, &self.name);
        entity.entity_view(world).set(self.def.clone());
        if let Some(s) = &self.stock {
            let e = model::spawn(world, &s.name);
            model::set_transform(world, &e, s.location);
            model::set_facets(world, &e, s.facets.clone());
            e.entity_view(world).set(StockDef {
                shape: s.shape.clone(),
            });
        }
        for (wname, loc) in &self.wcs {
            let e = model::spawn(world, wname);
            e.entity_view(world).add(Wcs);
            model::set_transform(world, &e, *loc);
        }
        setup::restore_model_poses(world, &self.previous_poses);
        setup::restore_context(world, &self.previous_context);
        setup::set_active(world, self.prev_active.clone());
    }
}

/// Choose the active setup and restore its part placement, machine and
/// post context. Undo restores the previous context exactly.
pub struct ActivateSetup {
    name: Option<String>,
    prev: Option<String>,
    before_poses: Vec<(String, Location)>,
    target_poses: Vec<(String, Location)>,
    prev_def: Option<SetupDef>,
    target_def: Option<SetupDef>,
    before_context: crate::schema::SetupContext,
    target_context: crate::schema::SetupContext,
}

impl ActivateSetup {
    pub fn new(world: &World, name: Option<&str>) -> Result<Self, CommandError> {
        if let Some(n) = name
            && !setup::exists(world, n)
        {
            return Err(CommandError::NotFound(n.to_string()));
        }
        let prev = setup::active(world);
        let target = name.map(str::to_string).or_else(|| setup::first(world));
        let before_poses = setup::model_poses(world);
        let before_context = setup::current_context(world);
        let prev_def = prev.as_deref().and_then(|s| setup::def_of(world, s));
        let target_def = target.as_deref().and_then(|s| setup::def_of(world, s));
        let target_poses = target_def.as_ref()
            .filter(|_| target != prev)
            .map(|def| def.model_poses.clone())
            .filter(|poses| !poses.is_empty())
            .unwrap_or_else(|| before_poses.clone());
        let target_context = target_def.as_ref()
            .filter(|_| target != prev)
            .and_then(|def| def.context.clone())
            .unwrap_or_else(|| before_context.clone());
        Ok(ActivateSetup {
            name: name.map(str::to_string), prev, before_poses,
            target_poses, prev_def, target_def,
            before_context, target_context,
        })
    }
}

impl Command for ActivateSetup {
    fn apply(&self, world: &World) {
        if self.name != self.prev {
            if let (Some(name), Some(def)) = (&self.prev, &self.prev_def)
                && let Some(entity) = model::find_by_name(world, name)
            {
                let mut updated = def.clone();
                updated.model_poses = self.before_poses.clone();
                updated.context = Some(self.before_context.clone());
                entity.entity_view(world).set(updated);
            }
            setup::restore_model_poses(world, &self.target_poses);
            setup::restore_context(world, &self.target_context);
        }
        setup::set_active(world, self.name.clone());
    }

    fn invert(&self, world: &World) {
        if self.name != self.prev {
            setup::restore_model_poses(world, &self.before_poses);
            setup::restore_context(world, &self.before_context);
            if let (Some(name), Some(def)) = (&self.prev, &self.prev_def)
                && let Some(entity) = model::find_by_name(world, name)
            {
                entity.entity_view(world).set(def.clone());
            }
            if let (Some(name), Some(def)) = (&self.name, &self.target_def)
                && let Some(entity) = model::find_by_name(world, name)
            {
                entity.entity_view(world).set(def.clone());
            }
        }
        setup::set_active(world, self.prev.clone());
    }
}

/// Select the controller's G54-G59 register for one setup. This changes
/// the posted work-offset word, never the virtual Toolpath CS placement.
pub struct SetSetupMachineCs {
    name: String,
    before: SetupDef,
    after: SetupDef,
}

impl SetSetupMachineCs {
    pub fn new(world: &World, name: &str, slot: usize) -> Result<Self, CommandError> {
        if slot >= WCS_SLOTS.len() {
            return Err(CommandError::Invalid("machine CS must be G54 through G59".into()));
        }
        let before = setup::def_of(world, name)
            .ok_or_else(|| CommandError::NotFound(name.to_owned()))?;
        if !before.modern_cs && setup::operation_order(world).into_iter().any(|(_, entity)| {
            model::operation_of(world, &entity).is_some_and(|op| setup::belongs(world, op.setup.as_deref(), name))
        }) {
            return Err(CommandError::Invalid(
                "this v0.14 setup uses placed work offsets; its existing operations cannot change setups".into()
            ));
        }
        let mut after = before.clone();
        after.machine_wcs = slot;
        after.modern_cs = true;
        Ok(Self { name: name.to_owned(), before, after })
    }
}

impl Command for SetSetupMachineCs {
    fn apply(&self, world: &World) {
        if let Some(entity) = model::find_by_name(world, &self.name) {
            entity.entity_view(world).set(self.after.clone());
        }
    }

    fn invert(&self, world: &World) {
        if let Some(entity) = model::find_by_name(world, &self.name) {
            entity.entity_view(world).set(self.before.clone());
        }
    }
}
