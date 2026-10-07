//! The transaction engine.
//!
//! A transaction is a list of operation calls that either all happen or none do.
//! The guarantees, and what each rests on:
//!
//! * **No partial document.** The steps run on a private copy of the document.
//!   The caller's document is a value that is never touched, so a failed
//!   transaction needs no undo: the copy is simply dropped.
//! * **No leaked pixels.** An operation that produces pixels registers an
//!   immutable buffer in the session store. Each registration is written to a
//!   journal as it happens, and on failure exactly those buffers are discarded —
//!   not "everything not referenced", which could take a buffer undo history still
//!   needs.
//! * **No invalid document, ever shown.** The authoritative validator
//!   (`LayerDocument::validate`) runs on the input, and again after every step. A
//!   step that would produce a document the compositor, the project format or the
//!   store limits would refuse is the step that fails, with its number.
//! * **No stale plan.** A plan that names the revision it was made against is
//!   refused if the document is not that revision.
//! * **No wasted work.** Before any pixel is produced the whole list is run once
//!   *dry*: every selector is resolved, every lock and kind and invariant checked,
//!   with placeholders where a step would make pixels. A reference to a layer an
//!   earlier step removed fails in milliseconds, not after the earlier step has
//!   spent seconds rendering a merge.
//! * **One undo.** The result is one document. Committing it is one history entry,
//!   however many steps there were.
//!
//! What it does not promise: if the process is killed mid-transaction the pixel
//! store is not durable and nothing is left to clean up; and two transactions
//! started against the same document will both succeed on their own copies, so the
//! caller serialises them (the interface does, and `expected_revision` catches a
//! caller that did not).
use super::model::{
    OperationCall, Origin, StepReport, TransactionRequest, TransactionResult, MAX_LABEL_CHARS,
    MAX_TRANSACTION_STEPS,
};
use super::registry::{LockPolicy, Operation, OperationKind};
use super::structure::{
    duplicate_layer, ensure_subtree_unlocked, ensure_unlocked, group_layers, insert_layer,
    new_layer, remove_layer, reset_transform, ungroup_layer, update,
};
use super::support::{document_revision, Clock, IdSource, RandomIds, SequenceIds, SystemClock};
use crate::application::AppState;
use crate::commands::layers::{
    apply_edit_to_layer, mask_for_layer, merge_targets, render_subset_into_buffer,
};
use crate::domain::EditOperation;
use crate::error::AppError;
use crate::layers::workflow::{resolve, LayerSelector};
use crate::layers::{LayerContent, LayerDocument, LayerKind, LayerMask};
use crate::mask::MaskSnapshot;
use image::{Rgba, RgbaImage};

/// Runs a transaction with random identifiers and the system clock.
pub async fn execute(
    state: &AppState,
    request: TransactionRequest,
) -> Result<TransactionResult, AppError> {
    let mut ids = RandomIds::default();
    execute_with(state, request, &mut ids, &SystemClock).await
}

/// What a step reports back to the engine.
#[derive(Default)]
struct Outcome {
    created_layers: Vec<String>,
    created_pixels: Vec<String>,
}

/// The state a transaction works in.
struct Tx<'a> {
    state: &'a AppState,
    document: LayerDocument,
    selection: Option<MaskSnapshot>,
    origin: Origin,
    /// The plugin a `Plugin` transaction acts for.
    plugin: Option<String>,
    /// Every pixel buffer registered so far, in order. The journal.
    created_pixels: Vec<String>,
    last_created: Option<String>,
    ids: &'a mut dyn IdSource,
    clock: &'a dyn Clock,
    /// A dry run resolves and checks everything but produces no pixels and
    /// touches no store. Pixel-producing steps substitute a placeholder.
    dry: bool,
    placeholders: u32,
}

impl Tx<'_> {
    fn placeholder(&mut self) -> String {
        self.placeholders += 1;
        format!("dry{:04}", self.placeholders)
    }

    fn resolve(&self, selector: &LayerSelector) -> Result<String, AppError> {
        resolve(&self.document, selector, self.last_created.as_deref())
    }

    /// Applies the operation's lock policy to a layer it is about to change.
    fn check_lock(&self, kind: OperationKind, id: &str, subtree: bool) -> Result<(), AppError> {
        let enforced = match kind.spec().lock {
            LockPolicy::Always => true,
            LockPolicy::AutomationOnly => self.origin != Origin::User,
            LockPolicy::Ignored => false,
        };
        if !enforced {
            return Ok(());
        }
        if subtree {
            ensure_subtree_unlocked(&self.document, id)
        } else {
            ensure_unlocked(&self.document, id)
        }
    }

    fn journal(&mut self, pixel_id: &str, outcome: &mut Outcome) {
        self.created_pixels.push(pixel_id.to_string());
        outcome.created_pixels.push(pixel_id.to_string());
    }

    fn created(&mut self, layer_id: &str, outcome: &mut Outcome) {
        self.last_created = Some(layer_id.to_string());
        outcome.created_layers.push(layer_id.to_string());
    }

    fn now(&self) -> String {
        self.clock.now()
    }
}

fn name_or(name: &Option<String>, fallback: &str) -> Result<String, AppError> {
    match name {
        Some(value) if value.trim().is_empty() || value.chars().count() > 120 => Err(
            AppError::InvalidOperation("layer names must contain 1 to 120 characters".into()),
        ),
        Some(value) => Ok(value.trim().to_string()),
        None => Ok(fallback.to_string()),
    }
}

/// What validation and the dry run learned, ready for the real run.
struct Prepared {
    label: String,
    operations: Vec<Operation>,
    /// Which steps' conditions did not hold in the dry run.
    skipped: Vec<bool>,
}

/// Whether a step runs: its condition, if it has one, asked of the working document.
fn runs(call: &OperationCall, tx: &Tx<'_>) -> bool {
    call.when
        .as_ref()
        .is_none_or(|condition| condition.holds(&tx.document, tx.last_created.as_deref()))
}

/// Everything that can be decided without producing a pixel: the request's shape, the
/// document, the revision, every step's parameters and the rules of who is asking,
/// and then the whole list run once dry.
async fn prepare(
    state: &AppState,
    request: &TransactionRequest,
    clock: &dyn Clock,
) -> Result<Prepared, AppError> {
    let label = request.label.trim().to_string();
    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        return Err(AppError::InvalidOperation(format!(
            "a transaction's label must contain 1 to {MAX_LABEL_CHARS} characters"
        )));
    }
    if request.steps.is_empty() {
        return Err(AppError::InvalidOperation(
            "a transaction needs at least one step".into(),
        ));
    }
    if request.steps.len() > MAX_TRANSACTION_STEPS {
        return Err(AppError::InvalidOperation(format!(
            "a transaction may have at most {MAX_TRANSACTION_STEPS} steps"
        )));
    }

    // The authoritative validator, on the way in.
    request.document.validate()?;
    if let Some(expected) = &request.expected_revision {
        if *expected != document_revision(&request.document)? {
            return Err(AppError::StaleRevision);
        }
    }

    // Everything that can be refused without running anything is refused first,
    // so a transaction that cannot succeed never starts.
    if request.origin == Origin::Plugin && request.plugin.is_none() {
        return Err(AppError::InvalidOperation(
            "a plugin transaction must say which plugin it is for".into(),
        ));
    }
    let operations = preflight(&request.steps, request.origin, request.selection.is_some())?;

    // A selection that does not decode (a corrupt checksum, a truncated payload) is
    // found now, not after the first step has rendered.
    if let Some(selection) = &request.selection {
        if let Some(index) = operations
            .iter()
            .position(|operation| operation.kind().spec().needs_selection)
        {
            selection.decode().map_err(|error| AppError::Transaction {
                step: index + 1,
                operation: request.steps[index].op.clone(),
                reason: error.to_string(),
            })?;
        }
    }

    // The dry run: all the checks, none of the pixels.
    let mut skipped = Vec::with_capacity(operations.len());
    let mut dry_ids = SequenceIds::default();
    let mut dry = Tx {
        state,
        document: request.document.clone(),
        selection: request.selection.clone(),
        origin: request.origin,
        plugin: request.plugin.clone(),
        created_pixels: Vec::new(),
        last_created: None,
        ids: &mut dry_ids,
        clock,
        dry: true,
        placeholders: 0,
    };
    for (index, (call, operation)) in request.steps.iter().zip(&operations).enumerate() {
        if !runs(call, &dry) {
            skipped.push(true);
            continue;
        }
        skipped.push(false);
        let result = match run_step(&mut dry, operation).await {
            Ok(_) => dry.document.validate(),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            return Err(AppError::Transaction {
                step: index + 1,
                operation: call.op.clone(),
                reason: error.to_string(),
            });
        }
    }
    Ok(Prepared {
        label,
        operations,
        skipped,
    })
}

/// What a transaction would do, without doing it: every check and a dry run of every
/// step, reporting which steps' conditions hold. Nothing is produced and nothing in
/// the pixel store changes.
pub async fn plan(
    state: &AppState,
    request: &TransactionRequest,
) -> Result<Vec<StepReport>, AppError> {
    let prepared = prepare(state, request, &SystemClock).await?;
    Ok(request
        .steps
        .iter()
        .enumerate()
        .map(|(index, call)| StepReport {
            index,
            op: call.op.clone(),
            skipped: prepared.skipped[index],
            created_layers: Vec::new(),
            created_pixels: Vec::new(),
        })
        .collect())
}

/// Runs a transaction with the given sources of identifiers and time.
pub async fn execute_with(
    state: &AppState,
    request: TransactionRequest,
    ids: &mut dyn IdSource,
    clock: &dyn Clock,
) -> Result<TransactionResult, AppError> {
    let Prepared {
        label,
        operations,
        skipped,
    } = prepare(state, &request, clock).await?;

    let mut tx = Tx {
        state,
        document: request.document,
        selection: request.selection,
        origin: request.origin,
        plugin: request.plugin.clone(),
        created_pixels: Vec::new(),
        last_created: None,
        ids,
        clock,
        dry: false,
        placeholders: 0,
    };
    let mut reports = Vec::with_capacity(operations.len());
    for (index, (call, operation)) in request.steps.iter().zip(&operations).enumerate() {
        if !runs(call, &tx) {
            // The dry run reached the same answer for this step; the two cannot
            // disagree, because neither produces anything the condition reads.
            debug_assert!(skipped[index]);
            reports.push(StepReport {
                index,
                op: call.op.clone(),
                skipped: true,
                created_layers: Vec::new(),
                created_pixels: Vec::new(),
            });
            continue;
        }
        let result = match run_step(&mut tx, operation).await {
            Ok(outcome) => tx.document.validate().map(|()| outcome),
            Err(error) => Err(error),
        };
        match result {
            Ok(outcome) => reports.push(StepReport {
                index,
                op: call.op.clone(),
                skipped: false,
                created_layers: outcome.created_layers,
                created_pixels: outcome.created_pixels,
            }),
            Err(error) => {
                roll_back(state, &tx.created_pixels);
                return Err(AppError::Transaction {
                    step: index + 1,
                    operation: call.op.clone(),
                    reason: error.to_string(),
                });
            }
        }
    }

    let revision = match document_revision(&tx.document) {
        Ok(revision) => revision,
        Err(error) => {
            roll_back(state, &tx.created_pixels);
            return Err(error);
        }
    };
    Ok(TransactionResult {
        document: tx.document,
        revision,
        label,
        steps: reports,
        created_pixel_ids: tx.created_pixels,
        last_created: tx.last_created,
    })
}

/// Parses every call and applies the origin's restrictions, running nothing.
fn preflight(
    steps: &[OperationCall],
    origin: Origin,
    has_selection: bool,
) -> Result<Vec<Operation>, AppError> {
    let mut operations = Vec::with_capacity(steps.len());
    let mut created_before = false;
    for (index, call) in steps.iter().enumerate() {
        let fail = |reason: String| AppError::Transaction {
            step: index + 1,
            operation: call.op.clone(),
            reason,
        };
        let operation = Operation::parse(call).map_err(|error| fail(error.to_string()))?;
        let spec = operation.kind().spec();
        if let Some(condition) = &call.when {
            condition
                .validate()
                .map_err(|error| fail(error.to_string()))?;
            // A condition names layers like a step does, and is held to the same
            // rules about who may name which.
            if origin == Origin::Planner
                && condition
                    .selectors()
                    .iter()
                    .any(|selector| !selector.is_planner_safe())
            {
                return Err(fail(
                    "a planner may only refer to the selected layer or the one just created."
                        .into(),
                ));
            }
            if condition
                .selectors()
                .iter()
                .any(|selector| matches!(selector, LayerSelector::LastCreated))
                && !created_before
            {
                return Err(fail(
                    "its condition refers to the layer an earlier step created, and none has."
                        .into(),
                ));
            }
        }
        if origin == Origin::Planner {
            if !spec.planner_safe {
                return Err(fail(
                    "a planner may propose suggestions, not decisions like this one.".into(),
                ));
            }
            // A planner never sees layer identifiers, so letting it name one
            // would let it invent one.
            if operation
                .selectors()
                .iter()
                .any(|selector| !selector.is_planner_safe())
            {
                return Err(fail(
                    "a planner may only refer to the selected layer or the one just created."
                        .into(),
                ));
            }
        }
        if spec.needs_selection && !has_selection {
            return Err(fail("this needs a selection, and there is none.".into()));
        }
        // `last created` before anything was created could only mean a layer the
        // caller imagined.
        if operation
            .selectors()
            .iter()
            .any(|selector| matches!(selector, LayerSelector::LastCreated))
            && !created_before
        {
            return Err(fail(
                "it refers to the layer an earlier step created, and none has.".into(),
            ));
        }
        if matches!(
            operation.kind(),
            OperationKind::AddGroup
                | OperationKind::AddAdjustment
                | OperationKind::AddPixel
                | OperationKind::Duplicate
                | OperationKind::Group
                | OperationKind::MergeDown
                | OperationKind::AddPluginAdjustment
                | OperationKind::Flatten
        ) {
            created_before = true;
        }
        operations.push(operation);
    }
    Ok(operations)
}

/// Discards the buffers a failed transaction registered, and only those.
fn roll_back(state: &AppState, created: &[String]) {
    if created.is_empty() {
        return;
    }
    // A poisoned lock means another thread panicked holding the store; the data is
    // intact (it is a map), so recover it rather than leak the buffers.
    let mut store = state
        .layers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    store.discard(created);
}

fn store_unavailable() -> AppError {
    AppError::ProcessingFailure("layer store is unavailable".into())
}

async fn run_step(tx: &mut Tx<'_>, operation: &Operation) -> Result<Outcome, AppError> {
    let mut outcome = Outcome::default();
    let kind = operation.kind();
    match operation {
        Operation::Select(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.document.active_layer_id = Some(id);
        }
        Operation::SetVisible(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            let now = tx.now();
            update(&mut tx.document, &id, &now, |layer| {
                layer.visible = p.visible;
                Ok(())
            })?;
        }
        Operation::SetLocked(p) => {
            let id = tx.resolve(&p.selector)?;
            let now = tx.now();
            update(&mut tx.document, &id, &now, |layer| {
                layer.locked = p.locked;
                Ok(())
            })?;
        }
        Operation::SetOpacity(p) => {
            if !p.opacity.is_finite() || !(0.0..=1.0).contains(&p.opacity) {
                return Err(AppError::InvalidOperation(
                    "layer opacity must be a finite value between 0 and 1".into(),
                ));
            }
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            let now = tx.now();
            update(&mut tx.document, &id, &now, |layer| {
                layer.opacity = p.opacity;
                Ok(())
            })?;
        }
        Operation::SetBlendMode(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            let now = tx.now();
            update(&mut tx.document, &id, &now, |layer| {
                layer.blend_mode = p.blend_mode;
                Ok(())
            })?;
        }
        Operation::Rename(p) => {
            let name = name_or(&Some(p.name.clone()), "")?;
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            let now = tx.now();
            update(&mut tx.document, &id, &now, |layer| {
                layer.name = name;
                Ok(())
            })?;
        }
        Operation::SetCollapsed(p) => {
            let id = tx.resolve(&p.selector)?;
            let now = tx.now();
            update(&mut tx.document, &id, &now, |layer| {
                layer.collapsed = p.collapsed;
                Ok(())
            })?;
        }
        Operation::Move(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            let parent = p.parent.as_ref().map(|s| tx.resolve(s)).transpose()?;
            tx.document.move_layer(&id, parent.as_deref(), p.index)?;
        }
        Operation::Delete(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            remove_layer(&mut tx.document, &id)?;
        }
        Operation::Duplicate(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            let copy = duplicate_layer(&mut tx.document, &id, tx.ids, tx.clock)?;
            tx.created(&copy, &mut outcome);
        }
        Operation::Group(p) => {
            let name = name_or(&p.name, "Group")?;
            let mut resolved = Vec::with_capacity(p.selectors.len());
            for selector in &p.selectors {
                let id = tx.resolve(selector)?;
                tx.check_lock(kind, &id, false)?;
                resolved.push(id);
            }
            let group = group_layers(&mut tx.document, &resolved, &name, tx.ids, tx.clock)?;
            tx.created(&group, &mut outcome);
        }
        Operation::Ungroup(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            ungroup_layer(&mut tx.document, &id)?;
        }
        Operation::ResetTransform(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            let now = tx.now();
            update(&mut tx.document, &id, &now, |layer| {
                reset_transform(layer);
                Ok(())
            })?;
        }
        Operation::AddGroup(p) => {
            let name = name_or(&p.name, "Group")?;
            let id = tx.ids.next("l");
            let layer = new_layer(
                &id,
                &name,
                LayerContent::Group {
                    children: Vec::new(),
                    isolated: true,
                },
                &tx.now(),
            );
            let top = tx.document.layers.len();
            insert_layer(&mut tx.document, layer, None, top)?;
            tx.created(&id, &mut outcome);
        }
        Operation::AddAdjustment(p) => {
            if !p.operation.supports_adjustment_layer() {
                return Err(AppError::UnsupportedAdjustmentLayer(
                    p.operation.kind().to_string(),
                ));
            }
            p.operation.validate()?;
            let name = name_or(&p.name, "Adjustment")?;
            let id = tx.ids.next("l");
            let layer = new_layer(
                &id,
                &name,
                LayerContent::Adjustment {
                    operation: p.operation.clone(),
                },
                &tx.now(),
            );
            let top = tx.document.layers.len();
            insert_layer(&mut tx.document, layer, None, top)?;
            tx.created(&id, &mut outcome);
        }
        Operation::AddPixel(p) => {
            let name = name_or(&p.name, "Layer")?;
            let (width, height) = (tx.document.canvas_width, tx.document.canvas_height);
            crate::layers::validate_dimensions(width, height)?;
            let pixel_id = if tx.dry {
                tx.placeholder()
            } else {
                let mut store = tx.state.layers.lock().map_err(|_| store_unavailable())?;
                let registered =
                    store.register(RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0])))?;
                drop(store);
                tx.journal(&registered, &mut outcome);
                registered
            };
            let id = tx.ids.next("l");
            let layer = new_layer(
                &id,
                &name,
                LayerContent::Pixel {
                    pixel_id,
                    width,
                    height,
                },
                &tx.now(),
            );
            let top = tx.document.layers.len();
            insert_layer(&mut tx.document, layer, None, top)?;
            tx.created(&id, &mut outcome);
        }
        Operation::ApplyEdit(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            if p.operations.is_empty() {
                return Err(AppError::InvalidOperation(
                    "apply_edit needs at least one edit".into(),
                ));
            }
            let layer = tx
                .document
                .find(&id)
                .ok_or_else(|| AppError::LayerNotFound(id.clone()))?;
            let (width, height) = layer.pixel_dimensions().ok_or_else(|| {
                AppError::InvalidOperation(format!(
                    "Apply to layer needs a pixel layer, but {} is not one",
                    layer.name
                ))
            })?;
            for edit in &p.operations {
                edit.validate()?;
                if !edit.supports_adjustment_layer() {
                    return Err(AppError::InvalidOperation(format!(
                        "{} changes document geometry and cannot be applied to a single layer",
                        edit.kind()
                    )));
                }
            }
            if tx.dry {
                return Ok(outcome);
            }
            let result =
                apply_edit_to_layer(&tx.document, &id, p.operations.clone(), tx.state).await?;
            tx.journal(&result.pixel_id, &mut outcome);
            if result.width != width || result.height != height {
                return Err(AppError::ProcessingFailure(
                    "the edit returned pixels of an unexpected size".into(),
                ));
            }
            let now = tx.now();
            update(&mut tx.document, &id, &now, |layer| {
                layer.content = LayerContent::Pixel {
                    pixel_id: result.pixel_id.clone(),
                    width: result.width,
                    height: result.height,
                };
                Ok(())
            })?;
        }
        Operation::MaskFromSelection(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            let selection = tx
                .selection
                .clone()
                .ok_or_else(|| AppError::InvalidOperation("there is no selection".into()))?;
            if selection.width != tx.document.canvas_width
                || selection.height != tx.document.canvas_height
            {
                return Err(AppError::InvalidOperation(
                    "the selection must be mapped to the document canvas first".into(),
                ));
            }
            if tx.dry {
                return Ok(outcome);
            }
            let mask = mask_for_layer(&tx.document, &id, &selection).await?;
            let now = tx.now();
            update(&mut tx.document, &id, &now, |layer| {
                layer.mask = Some(LayerMask {
                    snapshot: mask.snapshot.clone(),
                    enabled: true,
                    inverted: false,
                });
                Ok(())
            })?;
        }
        Operation::MergeDown(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, true)?;
            let layer = tx
                .document
                .find(&id)
                .ok_or_else(|| AppError::LayerNotFound(id.clone()))?;
            if layer.kind() != LayerKind::Pixel {
                return Err(AppError::InvalidOperation(
                    "Merge down needs a pixel layer.".into(),
                ));
            }
            let path = tx
                .document
                .path_to(&id)
                .ok_or_else(|| AppError::LayerNotFound(id.clone()))?;
            let (position, parent_path) = path
                .split_last()
                .ok_or_else(|| AppError::LayerNotFound(id.clone()))?;
            if *position == 0 {
                return Err(AppError::InvalidOperation(
                    "There is no layer beneath this one to merge into.".into(),
                ));
            }
            let parent = if parent_path.is_empty() {
                None
            } else {
                tx.document.layer_at(parent_path).map(|l| l.id.clone())
            };
            let below_path = {
                let mut below = parent_path.to_vec();
                below.push(position - 1);
                below
            };
            let below = tx
                .document
                .layer_at(&below_path)
                .ok_or_else(|| AppError::LayerNotFound(id.clone()))?;
            let (below_id, below_name) = (below.id.clone(), below.name.clone());
            ensure_subtree_unlocked(&tx.document, &below_id)?;
            let ordered = merge_targets(&tx.document, &[below_id.clone(), id.clone()])?;
            let merged = if tx.dry {
                crate::commands::layers::LayerPixelsResult {
                    pixel_id: tx.placeholder(),
                    width: tx.document.canvas_width,
                    height: tx.document.canvas_height,
                    filename: None,
                    raw: None,
                    origin: None,
                }
            } else {
                let merged =
                    render_subset_into_buffer(tx.document.clone(), ordered, tx.state).await?;
                tx.journal(&merged.pixel_id, &mut outcome);
                merged
            };
            if merged.width != tx.document.canvas_width
                || merged.height != tx.document.canvas_height
            {
                return Err(AppError::ProcessingFailure(
                    "the merge returned pixels of an unexpected size".into(),
                ));
            }
            remove_layer(&mut tx.document, &id)?;
            remove_layer(&mut tx.document, &below_id)?;
            let replacement_id = tx.ids.next("l");
            let replacement = new_layer(
                &replacement_id,
                &below_name,
                LayerContent::Pixel {
                    pixel_id: merged.pixel_id.clone(),
                    width: merged.width,
                    height: merged.height,
                },
                &tx.now(),
            );
            insert_layer(
                &mut tx.document,
                replacement,
                parent.as_deref(),
                position - 1,
            )?;
            tx.created(&replacement_id, &mut outcome);
        }
        Operation::Flatten => {
            if tx.document.iter_all().any(|layer| layer.locked) {
                return Err(AppError::InvalidOperation(
                    "Unlock the document layers before flattening.".into(),
                ));
            }
            let flat = if tx.dry {
                crate::commands::layers::LayerPixelsResult {
                    pixel_id: tx.placeholder(),
                    width: tx.document.canvas_width,
                    height: tx.document.canvas_height,
                    filename: None,
                    raw: None,
                    origin: None,
                }
            } else {
                let layers = tx.document.layers.clone();
                let flat = render_subset_into_buffer(tx.document.clone(), layers, tx.state).await?;
                tx.journal(&flat.pixel_id, &mut outcome);
                flat
            };
            let id = tx.ids.next("l");
            let background = new_layer(
                &id,
                "Background",
                LayerContent::Pixel {
                    pixel_id: flat.pixel_id.clone(),
                    width: flat.width,
                    height: flat.height,
                },
                &tx.now(),
            );
            tx.document = LayerDocument {
                schema_version: tx.document.schema_version,
                precision: tx.document.precision,
                canvas_width: tx.document.canvas_width,
                canvas_height: tx.document.canvas_height,
                layers: vec![background],
                smart_sources: Default::default(),
                active_layer_id: Some(id.clone()),
            };
            tx.created(&id, &mut outcome);
        }
        Operation::ApplyPluginFilter(p) => {
            let id = tx.resolve(&p.selector)?;
            tx.check_lock(kind, &id, false)?;
            let operation = plugin_operation(tx, &p.plugin, &p.filter, &p.parameters)?;
            let layer = tx
                .document
                .find(&id)
                .ok_or_else(|| AppError::LayerNotFound(id.clone()))?;
            let (width, height) = layer.pixel_dimensions().ok_or_else(|| {
                AppError::InvalidOperation(format!(
                    "a plugin filter needs a pixel layer, but {} is not one",
                    layer.name
                ))
            })?;
            if tx.dry {
                return Ok(outcome);
            }
            let result = apply_edit_to_layer(&tx.document, &id, vec![operation], tx.state).await?;
            tx.journal(&result.pixel_id, &mut outcome);
            if result.width != width || result.height != height {
                return Err(AppError::ProcessingFailure(
                    "the plugin returned pixels of an unexpected size".into(),
                ));
            }
            let now = tx.now();
            update(&mut tx.document, &id, &now, |layer| {
                layer.content = LayerContent::Pixel {
                    pixel_id: result.pixel_id.clone(),
                    width: result.width,
                    height: result.height,
                };
                Ok(())
            })?;
        }
        Operation::AddPluginAdjustment(p) => {
            if tx.document.precision != crate::pixel::DocumentPrecision::LinearSrgbF32 {
                return Err(AppError::InvalidOperation(
                    "plugin filters need a linear float document: convert it in Color and precision first"
                        .into(),
                ));
            }
            let operation = plugin_operation(tx, &p.plugin, &p.filter, &p.parameters)?;
            let fallback = filter_title(&p.plugin, &p.filter);
            let name = name_or(&p.name, &fallback)?;
            let id = tx.ids.next("l");
            let layer = new_layer(
                &id,
                &name,
                LayerContent::Adjustment {
                    operation: Box::new(operation),
                },
                &tx.now(),
            );
            let top = tx.document.layers.len();
            insert_layer(&mut tx.document, layer, None, top)?;
            tx.created(&id, &mut outcome);
        }
    }
    Ok(outcome)
}

/// The title of a filter, for naming a layer made from it.
fn filter_title(plugin: &str, filter: &str) -> String {
    use crate::plugins::store::{global, Resolution};
    match global().active(plugin) {
        Resolution::Available(loaded) => loaded
            .manifest
            .filter(filter)
            .map_or_else(|| filter.to_string(), |(_, decl)| decl.title.clone()),
        Resolution::Unavailable(_) => filter.to_string(),
    }
}

/// Turns a plugin id, a filter id and named parameters into the operation that
/// records exactly the installed version that will run.
///
/// Everything a person or a plugin could get wrong is refused here, before any
/// pixel is touched: a plugin that is not installed, that is turned off, that was
/// not granted what its filters need, a filter it does not have, a parameter it did
/// not declare, a value outside the declared range.
fn plugin_operation(
    tx: &Tx<'_>,
    plugin: &str,
    filter: &str,
    parameters: &serde_json::Value,
) -> Result<EditOperation, AppError> {
    use crate::plugins::store::{global, Resolution};
    if tx.origin == Origin::Plugin && tx.plugin.as_deref() != Some(plugin) {
        return Err(AppError::InvalidOperation(format!(
            "a plugin may apply only its own filters, not {plugin}'s"
        )));
    }
    let loaded = match global().active(plugin) {
        Resolution::Available(loaded) => loaded,
        Resolution::Unavailable(why) => {
            return Err(AppError::PluginUnavailable {
                plugin: plugin.to_string(),
                reason: why.describe(plugin, ""),
            })
        }
    };
    let (_, decl) = loaded
        .manifest
        .filter(filter)
        .ok_or_else(|| AppError::PluginUnavailable {
            plugin: plugin.to_string(),
            reason: format!("it has no filter called {filter}"),
        })?;
    let given = match parameters {
        serde_json::Value::Null => serde_json::Map::new(),
        serde_json::Value::Object(map) => map.clone(),
        _ => {
            return Err(AppError::InvalidOperation(
                "a filter's parameters must be an object of names and values".into(),
            ))
        }
    };
    if let Some(unknown) = given
        .keys()
        .find(|key| !decl.parameters.iter().any(|p| &p.id == *key))
    {
        return Err(AppError::InvalidOperation(format!(
            "{filter} has no parameter called {unknown}"
        )));
    }
    let mut values = Vec::with_capacity(decl.parameters.len());
    for declared in &decl.parameters {
        let value = match given.get(&declared.id) {
            None => declared.default_value(),
            Some(serde_json::Value::Bool(flag)) => f64::from(u8::from(*flag)),
            Some(other) => other.as_f64().ok_or_else(|| {
                AppError::InvalidOperation(format!(
                    "the parameter {} must be a number",
                    declared.id
                ))
            })?,
        };
        if !declared.accepts(value) {
            return Err(AppError::InvalidOperation(format!(
                "{value} is not a value the parameter {} accepts",
                declared.id
            )));
        }
        values.push(value);
    }
    Ok(EditOperation::PluginFilter {
        plugin: plugin.to_string(),
        version: loaded.manifest.version.clone(),
        sha256: loaded.content_hash.clone(),
        filter: filter.to_string(),
        locality: decl.locality,
        parameters: values,
    })
}
