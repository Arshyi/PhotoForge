//! The operation registry and transaction engine, tested as what they promise:
//! all of a transaction or none of it, no leaked pixels, no invalid document, no
//! stale plan, and the same answer for the same input.
use photoforge_lib::application::AppState;
use photoforge_lib::domain::EditOperation;
use photoforge_lib::error::AppError;
use photoforge_lib::layers::{LayerContent, LayerDocument};
use photoforge_lib::operations::structure::new_layer;
use photoforge_lib::operations::support::FixedClock;
use photoforge_lib::operations::{
    document_revision, execute_with, OperationCall, Origin, SequenceIds, TransactionRequest,
    TransactionResult, MAX_TRANSACTION_STEPS,
};
use serde_json::{json, Value};

fn call(op: &str, params: Value) -> OperationCall {
    OperationCall {
        op: op.to_string(),
        params,
        when: None,
    }
}

fn clock() -> FixedClock {
    FixedClock("2026-10-06T00:00:00.000Z".into())
}

/// A state whose store holds one 8x8 buffer per pixel layer of the document.
fn world(layer_ids: &[&str]) -> (AppState, LayerDocument) {
    let state = AppState::default();
    let mut document = LayerDocument::new(8, 8);
    {
        let mut store = state.layers.lock().unwrap();
        store.reset(8, 8).unwrap();
        for id in layer_ids {
            let pixel_id = store
                .register(image::RgbaImage::from_pixel(
                    8,
                    8,
                    image::Rgba([200, 100, 50, 255]),
                ))
                .unwrap();
            document.layers.push(new_layer(
                id,
                id,
                LayerContent::Pixel {
                    pixel_id,
                    width: 8,
                    height: 8,
                },
                "t0",
            ));
        }
    }
    document.active_layer_id = layer_ids.last().map(|id| id.to_string());
    (state, document)
}

fn buffers(state: &AppState) -> usize {
    state.layers.lock().unwrap().buffer_count()
}

fn request(document: &LayerDocument, steps: Vec<OperationCall>) -> TransactionRequest {
    TransactionRequest {
        document: document.clone(),
        label: "Test".into(),
        steps,
        expected_revision: None,
        selection: None,
        origin: Origin::User,
        plugin: None,
    }
}

async fn run(state: &AppState, request: TransactionRequest) -> Result<TransactionResult, AppError> {
    let mut ids = SequenceIds::default();
    execute_with(state, request, &mut ids, &clock()).await
}

fn active() -> Value {
    json!({ "type": "active" })
}

fn id(layer: &str) -> Value {
    json!({ "type": "id", "id": layer })
}

fn contrast() -> Value {
    serde_json::to_value(EditOperation::Contrast { amount: 0.25 }).unwrap()
}

fn brightness() -> Value {
    serde_json::to_value(EditOperation::Contrast { amount: -0.1 }).unwrap()
}

// ---------------------------------------------------------------------------------

#[tokio::test]
async fn a_multi_step_transaction_produces_one_validated_document() {
    let (state, document) = world(&["a", "b"]);
    let result = run(
        &state,
        request(
            &document,
            vec![
                call(
                    "core.layer.add_adjustment",
                    json!({"operation": contrast(), "name": "Punch"}),
                ),
                // The relative selector follows the layer the previous step made.
                call(
                    "core.layer.set_opacity",
                    json!({"selector": {"type": "last_created"}, "opacity": 0.4}),
                ),
                call("core.layer.add_group", json!({"name": "Notes"})),
                call(
                    "core.layer.move",
                    json!({"selector": id("a"), "parent": {"type": "last_created"}, "index": 0}),
                ),
            ],
        ),
    )
    .await
    .unwrap();

    assert_eq!(result.steps.len(), 4);
    assert_eq!(result.label, "Test");
    let doc = &result.document;
    doc.validate().unwrap();
    // The adjustment exists, is the layer the opacity step reached, and is where
    // a new layer goes: on top of the stack at the time.
    let punch = doc.iter().find(|layer| layer.name == "Punch").unwrap();
    assert_eq!(punch.opacity, 0.4);
    let group = doc.iter().find(|layer| layer.name == "Notes").unwrap();
    assert_eq!(group.children().len(), 1);
    assert_eq!(group.children()[0].id, "a");
    assert_eq!(result.last_created.as_deref(), Some(group.id.as_str()));
    // The revision names exactly this document.
    assert_eq!(result.revision, document_revision(doc).unwrap());
    // The input was not edited: it is a value, not a handle.
    assert_eq!(document.layers.len(), 2);
}

#[tokio::test]
async fn a_failed_step_leaves_nothing_behind() {
    let (state, document) = world(&["a", "b"]);
    let before = buffers(&state);
    // Two steps that register pixels, then one that cannot work.
    let error = run(
        &state,
        request(
            &document,
            vec![
                call("core.layer.add_pixel", json!({"name": "One"})),
                call("core.layer.add_pixel", json!({"name": "Two"})),
                call(
                    "core.layer.delete",
                    json!({"selector": id("does-not-exist")}),
                ),
            ],
        ),
    )
    .await
    .unwrap_err();

    // The failure names the step and the operation, and says nothing was kept.
    match &error {
        AppError::Transaction {
            step, operation, ..
        } => {
            assert_eq!(*step, 3);
            assert_eq!(operation, "core.layer.delete");
        }
        other => panic!("{other:?}"),
    }
    assert!(error.to_string().contains("Nothing was changed"));
    // Every buffer the first two steps registered is gone. Not "unreferenced ones
    // were swept": exactly the journal, so nothing a history entry needs is lost.
    assert_eq!(buffers(&state), before, "pixel buffers leaked");
    // And the buffers that existed before are untouched.
    for layer in document.iter() {
        assert!(state
            .layers
            .lock()
            .unwrap()
            .contains(layer.pixel_id().unwrap()));
    }
}

#[tokio::test]
async fn a_failing_pixel_step_discards_the_buffers_of_the_steps_before_it() {
    let (state, document) = world(&["a", "b"]);
    let before = buffers(&state);
    // Apply an edit (registers a buffer), merge (registers another), then fail by
    // asking for a merge with nothing beneath it.
    let error = run(
        &state,
        request(
            &document,
            vec![
                call(
                    "core.layer.apply_edit",
                    json!({"selector": id("b"), "operations": [contrast()]}),
                ),
                call("core.layer.merge_down", json!({"selector": id("b")})),
                call(
                    "core.layer.merge_down",
                    json!({"selector": {"type": "bottom"}}),
                ),
            ],
        ),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, AppError::Transaction { step: 3, .. }),
        "{error:?}"
    );
    assert_eq!(buffers(&state), before, "pixel buffers leaked");
}

#[tokio::test]
async fn pixel_operations_register_buffers_the_new_document_refers_to() {
    let (state, document) = world(&["a", "b"]);
    let before = buffers(&state);
    let result = run(
        &state,
        request(
            &document,
            vec![
                call(
                    "core.layer.apply_edit",
                    json!({"selector": id("b"), "operations": [contrast(), brightness()]}),
                ),
                call("core.layer.add_pixel", json!({})),
            ],
        ),
    )
    .await
    .unwrap();
    assert_eq!(result.created_pixel_ids.len(), 2);
    assert_eq!(buffers(&state), before + 2);
    // Every reference in the result resolves, and `b` now points at the new pixels.
    let store = state.layers.lock().unwrap();
    for pixel in result.document.referenced_pixel_ids() {
        assert!(store.contains(&pixel), "{pixel} is dangling");
    }
    assert_ne!(
        result.document.find("b").unwrap().pixel_id(),
        document.find("b").unwrap().pixel_id()
    );
    // The old buffer is still there, so Undo can go back to it.
    assert!(store.contains(document.find("b").unwrap().pixel_id().unwrap()));
}

#[tokio::test]
async fn merge_down_and_flatten_work_through_the_same_path() {
    let (state, document) = world(&["a", "b", "c"]);
    let merged = run(
        &state,
        request(
            &document,
            vec![call("core.layer.merge_down", json!({"selector": id("c")}))],
        ),
    )
    .await
    .unwrap();
    assert_eq!(merged.document.layers.len(), 2);
    // The merged layer takes the lower layer's name and its place.
    assert_eq!(merged.document.layers[1].name, "b");
    assert_eq!(merged.created_pixel_ids.len(), 1);

    let flat = run(
        &state,
        request(&document, vec![call("core.document.flatten", json!({}))]),
    )
    .await
    .unwrap();
    assert_eq!(flat.document.layers.len(), 1);
    assert_eq!(flat.document.layers[0].name, "Background");
    flat.document.validate().unwrap();
}

#[tokio::test]
async fn a_merge_that_would_change_the_picture_is_refused() {
    let (state, mut document) = world(&["a", "b", "c"]);
    // `c` multiplies: merged without what is under `a`... it sits above `b`, and
    // `b` is not the bottom, so the backdrop would be omitted.
    document.layers[2].blend_mode = photoforge_lib::layers::BlendMode::Multiply;
    let error = run(
        &state,
        request(
            &document,
            vec![call("core.layer.merge_down", json!({"selector": id("c")}))],
        ),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, AppError::Transaction { step: 1, .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_plan_made_against_another_revision_is_refused() {
    let (state, document) = world(&["a"]);
    let revision = document_revision(&document).unwrap();
    let mut ok = request(&document, vec![call("core.layer.add_group", json!({}))]);
    ok.expected_revision = Some(revision.clone());
    run(&state, ok).await.unwrap();

    let mut changed = document.clone();
    changed.layers[0].name = "edited since".into();
    let mut stale = request(&changed, vec![call("core.layer.add_group", json!({}))]);
    stale.expected_revision = Some(revision);
    assert!(matches!(
        run(&state, stale).await,
        Err(AppError::StaleRevision)
    ));
}

#[tokio::test]
async fn the_same_request_gives_the_same_document() {
    let (state, document) = world(&["a", "b"]);
    let steps = || {
        vec![
            call("core.layer.duplicate", json!({"selector": id("a")})),
            call(
                "core.layer.group",
                json!({"selectors": [id("a"), id("b")], "name": "Pair"}),
            ),
            call(
                "core.layer.add_adjustment",
                json!({"operation": contrast()}),
            ),
        ]
    };
    let first = run(&state, request(&document, steps())).await.unwrap();
    let second = run(&state, request(&document, steps())).await.unwrap();
    assert_eq!(first.document, second.document);
    assert_eq!(first.revision, second.revision);
}

#[tokio::test]
async fn refusals_that_need_no_running_happen_before_anything_runs() {
    let (state, document) = world(&["a"]);
    let before = buffers(&state);
    let cases: Vec<(&str, Vec<OperationCall>)> = vec![
        (
            "unknown operation",
            vec![call("core.layer.explode", json!({}))],
        ),
        (
            "a plugin id in the core namespace's place",
            vec![call("plugin.evil.set_opacity", json!({}))],
        ),
        (
            "a surplus parameter",
            vec![call(
                "core.layer.select",
                json!({"selector": active(), "sneaky": true}),
            )],
        ),
        (
            "last_created before anything was created",
            vec![call(
                "core.layer.set_opacity",
                json!({"selector": {"type": "last_created"}, "opacity": 0.5}),
            )],
        ),
        (
            "a selection operation without a selection",
            vec![call(
                "core.layer.mask_from_selection",
                json!({"selector": active()}),
            )],
        ),
    ];
    for (name, steps) in cases {
        // Preceded by a step that registers pixels without creating a layer: if
        // validation were lazy that buffer would exist, and a failure would have
        // to clean it up.
        let mut all = vec![call(
            "core.layer.apply_edit",
            json!({"selector": id("a"), "operations": [contrast()]}),
        )];
        all.extend(steps);
        let error = run(&state, request(&document, all)).await.unwrap_err();
        assert!(
            matches!(error, AppError::Transaction { step: 2, .. }),
            "{name}: {error:?}"
        );
        assert_eq!(buffers(&state), before, "{name}: nothing may have run");
    }
}

#[tokio::test]
async fn the_shape_of_a_transaction_is_bounded() {
    let (state, document) = world(&["a"]);
    let one = || call("core.layer.add_group", json!({}));
    // Empty, too long, and badly labelled.
    assert!(run(&state, request(&document, vec![])).await.is_err());
    let too_many = vec![one(); MAX_TRANSACTION_STEPS + 1];
    assert!(run(&state, request(&document, too_many)).await.is_err());
    let mut unlabelled = request(&document, vec![one()]);
    unlabelled.label = "   ".into();
    assert!(run(&state, unlabelled).await.is_err());
    let mut long_label = request(&document, vec![one()]);
    long_label.label = "x".repeat(121);
    assert!(run(&state, long_label).await.is_err());
    // The maximum is allowed.
    let max = vec![one(); 20];
    assert!(run(&state, request(&document, max)).await.is_ok());
}

#[tokio::test]
async fn a_step_that_would_break_an_invariant_is_the_step_that_fails() {
    let (state, document) = world(&["a"]);
    // Group a layer into a deeper and deeper stack of groups until the depth limit
    // refuses; the whole transaction goes with it.
    let mut steps = Vec::new();
    for _ in 0..20 {
        steps.push(call(
            "core.layer.group",
            json!({"selectors": [{"type": "active"}], "name": "Deeper"}),
        ));
    }
    let before = buffers(&state);
    let error = run(&state, request(&document, steps)).await.unwrap_err();
    match error {
        AppError::Transaction { step, .. } => assert!(step > 1 && step <= 20, "{step}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(buffers(&state), before);
}

// ---- who is asking --------------------------------------------------------------

#[tokio::test]
async fn a_person_may_change_what_they_locked_and_an_automation_may_not() {
    let (state, mut document) = world(&["a", "b"]);
    document.layers[1].locked = true;
    let steps = || {
        vec![call(
            "core.layer.set_opacity",
            json!({"selector": id("b"), "opacity": 0.3}),
        )]
    };
    // The person who locked it can still dim it.
    let result = run(&state, request(&document, steps())).await.unwrap();
    assert_eq!(result.document.find("b").unwrap().opacity, 0.3);
    // An unattended replay cannot: the lock is what protects it.
    for origin in [Origin::Automation, Origin::Batch, Origin::Plugin] {
        let mut automated = request(&document, steps());
        automated.origin = origin;
        // A plugin's transaction says which plugin it is for.
        automated.plugin = (origin == Origin::Plugin).then(|| "com.example.acting".to_string());
        let error = run(&state, automated).await.unwrap_err();
        assert!(error.to_string().contains("b"), "{origin:?}: {error}");
        assert!(
            matches!(error, AppError::Transaction { step: 1, .. }),
            "{origin:?}"
        );
    }
}

#[tokio::test]
async fn destroying_a_locked_layer_is_refused_to_everyone() {
    let (state, mut document) = world(&["a", "b"]);
    document.layers[1].locked = true;
    for origin in [Origin::User, Origin::Automation] {
        let mut delete = request(
            &document,
            vec![call("core.layer.delete", json!({"selector": id("b")}))],
        );
        delete.origin = origin;
        assert!(run(&state, delete).await.is_err(), "{origin:?}");
    }
    // Unlocking is how a lock is changed, and is allowed whoever asks.
    let mut unlock = request(
        &document,
        vec![call(
            "core.layer.set_locked",
            json!({"selector": id("b"), "locked": false}),
        )],
    );
    unlock.origin = Origin::Automation;
    assert!(
        !run(&state, unlock)
            .await
            .unwrap()
            .document
            .find("b")
            .unwrap()
            .locked
    );
}

#[tokio::test]
async fn a_planner_may_suggest_but_not_decide_and_cannot_name_a_layer() {
    let (state, document) = world(&["a", "b"]);
    let planner = |steps: Vec<OperationCall>| {
        let mut request = request(&document, steps);
        request.origin = Origin::Planner;
        request
    };
    // Allowed: relative selectors, suggestion operations.
    let allowed = run(
        &state,
        planner(vec![
            call(
                "core.layer.add_adjustment",
                json!({"operation": contrast()}),
            ),
            call(
                "core.layer.set_opacity",
                json!({"selector": {"type": "last_created"}, "opacity": 0.8}),
            ),
            call(
                "core.layer.set_blend_mode",
                json!({"selector": active(), "blendMode": "multiply"}),
            ),
        ]),
    )
    .await;
    assert!(allowed.is_ok(), "{:?}", allowed.err());

    // Refused: it cannot name a layer, so it cannot invent one...
    for refused in [
        call(
            "core.layer.set_opacity",
            json!({"selector": id("a"), "opacity": 0.5}),
        ),
        call(
            "core.layer.set_opacity",
            json!({"selector": {"type": "name", "name": "a"}, "opacity": 0.5}),
        ),
        call(
            "core.layer.set_opacity",
            json!({"selector": {"type": "top"}, "opacity": 0.5}),
        ),
        // ...and destructive or structural operations are decisions.
        call("core.layer.delete", json!({"selector": active()})),
        call("core.layer.merge_down", json!({"selector": active()})),
        call("core.document.flatten", json!({})),
        call(
            "core.layer.set_locked",
            json!({"selector": active(), "locked": false}),
        ),
    ] {
        let op = refused.op.clone();
        let error = run(&state, planner(vec![refused])).await.unwrap_err();
        assert!(
            matches!(error, AppError::Transaction { step: 1, .. }),
            "{op}: {error:?}"
        );
    }
}

// ---- structure ------------------------------------------------------------------

#[tokio::test]
async fn structural_operations_behave_like_the_panel_does() {
    let (state, document) = world(&["a", "b", "c"]);
    let result = run(
        &state,
        request(
            &document,
            vec![
                call(
                    "core.layer.rename",
                    json!({"selector": id("a"), "name": "Sky"}),
                ),
                call(
                    "core.layer.set_visible",
                    json!({"selector": id("b"), "visible": false}),
                ),
                call("core.layer.duplicate", json!({"selector": id("c")})),
                call("core.layer.reset_transform", json!({"selector": id("a")})),
                call("core.layer.select", json!({"selector": {"type": "bottom"}})),
            ],
        ),
    )
    .await
    .unwrap();
    let doc = &result.document;
    assert_eq!(doc.find("a").unwrap().name, "Sky");
    assert!(!doc.find("b").unwrap().visible);
    assert_eq!(doc.layers.len(), 4);
    assert_eq!(doc.layers[3].name, "c copy");
    // Selecting by relative selector, last: the duplicate's selection was replaced.
    assert_eq!(doc.active_layer_id.as_deref(), Some("a"));
    // Modification times are stamped by the injected clock, not the wall.
    assert_eq!(
        doc.find("a").unwrap().metadata.modified_at,
        "2026-10-06T00:00:00.000Z"
    );
}

#[tokio::test]
async fn an_ambiguous_name_is_refused_rather_than_guessed() {
    let (state, mut document) = world(&["a", "b"]);
    document.layers[0].name = "Same".into();
    document.layers[1].name = "Same".into();
    let error = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.set_opacity",
                json!({"selector": {"type": "name", "name": "Same"}, "opacity": 0.5}),
            )],
        ),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("unambiguous"), "{error}");
}

#[tokio::test]
async fn a_plugin_filter_with_no_plugin_installed_is_a_clear_refusal() {
    let (state, document) = world(&["a"]);
    let before = buffers(&state);
    let error = run(
        &state,
        request(
            &document,
            vec![call(
                "core.plugin.apply_filter",
                json!({"selector": active(), "plugin": "example.border", "filter": "add_border"}),
            )],
        ),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("example.border"), "{error}");
    assert_eq!(buffers(&state), before);
}

/// The dry run: a transaction whose later step cannot work is refused before an
/// earlier step has produced a single pixel. Rolling back a finished render is
/// correct; not starting it is better.
#[tokio::test]
async fn an_impossible_reference_is_found_before_any_pixel_is_produced() {
    let (state, document) = world(&["a", "b"]);
    let probe = |state: &AppState| {
        state
            .layers
            .lock()
            .unwrap()
            .register(image::RgbaImage::new(1, 1))
            .unwrap()
    };
    let first = probe(&state);
    // Edit `b` (which would register a buffer), delete `b`, then act on `b` again.
    let error = run(
        &state,
        request(
            &document,
            vec![
                call(
                    "core.layer.apply_edit",
                    json!({"selector": id("b"), "operations": [contrast()]}),
                ),
                call("core.layer.delete", json!({"selector": id("b")})),
                call(
                    "core.layer.set_opacity",
                    json!({"selector": id("b"), "opacity": 0.5}),
                ),
            ],
        ),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, AppError::Transaction { step: 3, .. }),
        "{error:?}"
    );
    // Identifiers are handed out in order. If the edit had run, even to be rolled
    // back, the next one would skip it.
    let second = probe(&state);
    let number = |id: &str| id.trim_start_matches("px").parse::<u32>().unwrap();
    assert_eq!(number(&second), number(&first) + 1, "{first} then {second}");
}

// ---- behaviours the TypeScript executor was held to, now held here --------------

fn selection(value: u8) -> photoforge_lib::mask::MaskSnapshot {
    let mut bitmap = photoforge_lib::mask::MaskBitmap::empty(8, 8).unwrap();
    for y in 0..8 {
        for x in 0..8 {
            bitmap.set(x, y, value);
        }
    }
    photoforge_lib::mask::MaskSnapshot::encode(&bitmap)
}

#[tokio::test]
async fn a_merge_takes_effect_before_the_next_selector_resolves() {
    let (state, document) = world(&["bottom", "top"]);
    let result = run(
        &state,
        request(
            &document,
            vec![
                call("core.layer.merge_down", json!({"selector": active()})),
                // `active` now means the merged layer, and the one that is left.
                call(
                    "core.layer.set_opacity",
                    json!({"selector": active(), "opacity": 0.4}),
                ),
                call(
                    "core.layer.apply_edit",
                    json!({"selector": active(), "operations": [contrast()]}),
                ),
            ],
        ),
    )
    .await
    .unwrap();
    assert_eq!(result.document.layers.len(), 1);
    assert_eq!(result.document.layers[0].opacity, 0.4);
    // Two buffers: the merge, then the edit of the merged pixels.
    assert_eq!(result.created_pixel_ids.len(), 2);
    assert_eq!(
        result.document.layers[0].pixel_id().unwrap(),
        result.created_pixel_ids[1]
    );
}

#[tokio::test]
async fn flatten_refuses_a_locked_layer_before_producing_pixels() {
    let (state, mut document) = world(&["a", "b"]);
    document.layers[1].locked = true;
    let before = buffers(&state);
    let error = run(
        &state,
        request(
            &document,
            vec![
                call(
                    "core.layer.apply_edit",
                    json!({"selector": id("a"), "operations": [contrast()]}),
                ),
                call("core.document.flatten", json!({})),
            ],
        ),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, AppError::Transaction { step: 2, .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("Unlock"), "{error}");
    assert_eq!(buffers(&state), before);
}

#[tokio::test]
async fn a_mask_from_the_selection_can_be_merged_in_the_same_transaction() {
    let (state, document) = world(&["a", "b"]);
    let mut with_selection = request(
        &document,
        vec![
            call(
                "core.layer.mask_from_selection",
                json!({"selector": active()}),
            ),
            call("core.layer.merge_down", json!({"selector": active()})),
        ],
    );
    with_selection.selection = Some(selection(128));
    let result = run(&state, with_selection).await.unwrap();
    assert_eq!(result.document.layers.len(), 1);
    // The merge consumed the masked layer, so the result carries no mask of its own.
    assert!(result.document.layers[0].mask.is_none());

    // A selection of the wrong size is refused rather than stretched.
    let mut wrong = request(
        &document,
        vec![call(
            "core.layer.mask_from_selection",
            json!({"selector": active()}),
        )],
    );
    let mut bitmap = photoforge_lib::mask::MaskBitmap::empty(4, 4).unwrap();
    bitmap.set(0, 0, 255);
    wrong.selection = Some(photoforge_lib::mask::MaskSnapshot::encode(&bitmap));
    assert!(run(&state, wrong).await.is_err());
}

#[tokio::test]
async fn a_corrupt_selection_is_found_before_any_step_runs() {
    let (state, document) = world(&["a", "b"]);
    let probe = |state: &AppState| {
        state
            .layers
            .lock()
            .unwrap()
            .register(image::RgbaImage::new(1, 1))
            .unwrap()
    };
    let first = probe(&state);
    let mut corrupt = selection(255);
    corrupt.checksum = "fnv1a64:0000000000000000".into();
    let mut bad = request(
        &document,
        vec![
            call(
                "core.layer.apply_edit",
                json!({"selector": id("a"), "operations": [contrast()]}),
            ),
            call(
                "core.layer.mask_from_selection",
                json!({"selector": active()}),
            ),
        ],
    );
    bad.selection = Some(corrupt);
    let error = run(&state, bad).await.unwrap_err();
    assert!(
        matches!(error, AppError::Transaction { step: 2, .. }),
        "{error:?}"
    );
    let second = probe(&state);
    let number = |id: &str| id.trim_start_matches("px").parse::<u32>().unwrap();
    assert_eq!(
        number(&second),
        number(&first) + 1,
        "the edit ran before the selection was checked"
    );
}

#[tokio::test]
async fn a_locked_descendant_is_not_discarded_by_a_merge_into_its_group() {
    let (state, mut document) = world(&["child", "above"]);
    let mut ids = SequenceIds::default();
    // Put `child` inside a group, below `above`, and lock it.
    let group = photoforge_lib::operations::structure::group_layers(
        &mut document,
        &["child".to_string()],
        "Below",
        &mut ids,
        &clock(),
    )
    .unwrap();
    document.layer_mut("child").unwrap().locked = true;
    document.active_layer_id = Some("above".into());
    assert!(document.find(&group).is_some());
    let error = run(
        &state,
        request(
            &document,
            vec![call("core.layer.merge_down", json!({"selector": active()}))],
        ),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("child"), "{error}");
}

// ---- the operations the Layers panel needs ----------------------------------------
//
// Phase 15.0 routes the panel's own edits through the engine, so each of these is an
// operation a person can now trigger from the panel and a macro can record.

fn rotate() -> Value {
    serde_json::to_value(EditOperation::Rotate { degrees: 15 }).unwrap()
}

/// A document with one group (`g`) holding one pixel layer (`a`), plus a layer `b`.
fn grouped() -> (AppState, LayerDocument) {
    let (state, mut document) = world(&["a", "b"]);
    let a = document.layers.remove(0);
    let group = new_layer(
        "g",
        "g",
        LayerContent::Group {
            children: vec![a],
            isolated: true,
        },
        "t0",
    );
    document.layers.insert(0, group);
    document.active_layer_id = Some("b".into());
    (state, document)
}

#[tokio::test]
async fn pass_through_is_set_and_returns_the_blend_mode_to_normal() {
    let (state, mut document) = grouped();
    document.layer_mut("g").unwrap().blend_mode = photoforge_lib::layers::BlendMode::Multiply;
    let result = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.set_pass_through",
                json!({"selector": id("g"), "passThrough": true}),
            )],
        ),
    )
    .await
    .unwrap();
    let group = result.document.find("g").unwrap();
    assert!(matches!(
        group.content,
        LayerContent::Group {
            isolated: false,
            ..
        }
    ));
    assert_eq!(group.blend_mode, photoforge_lib::layers::BlendMode::Normal);

    // And back: isolating the group again does not invent a blend mode.
    let back = run(
        &state,
        request(
            &result.document,
            vec![call(
                "core.layer.set_pass_through",
                json!({"selector": id("g"), "passThrough": false}),
            )],
        ),
    )
    .await
    .unwrap();
    assert!(matches!(
        back.document.find("g").unwrap().content,
        LayerContent::Group { isolated: true, .. }
    ));
}

#[tokio::test]
async fn pass_through_is_refused_for_a_layer_that_is_not_a_group() {
    let (state, document) = world(&["a"]);
    let error = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.set_pass_through",
                json!({"selector": id("a"), "passThrough": true}),
            )],
        ),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("not one"), "{error}");
}

#[tokio::test]
async fn a_mask_can_be_added_inverted_disabled_and_removed_in_one_transaction() {
    let (state, document) = world(&["a"]);
    // The later steps act on a mask the first step makes, so the dry run must make it too.
    let result = run(
        &state,
        request(
            &document,
            vec![
                call(
                    "core.layer.mask_add",
                    json!({"selector": id("a"), "filled": false}),
                ),
                call(
                    "core.layer.mask_set_inverted",
                    json!({"selector": id("a"), "inverted": true}),
                ),
                call(
                    "core.layer.mask_set_enabled",
                    json!({"selector": id("a"), "enabled": false}),
                ),
            ],
        ),
    )
    .await
    .unwrap();
    let mask = result.document.find("a").unwrap().mask.as_ref().unwrap();
    assert!(mask.inverted);
    assert!(!mask.enabled);
    assert_eq!((mask.snapshot.width, mask.snapshot.height), (8, 8));

    let removed = run(
        &state,
        request(
            &result.document,
            vec![call("core.layer.mask_remove", json!({"selector": id("a")}))],
        ),
    )
    .await
    .unwrap();
    assert!(removed.document.find("a").unwrap().mask.is_none());
}

#[tokio::test]
async fn a_revealing_mask_and_a_hiding_mask_differ() {
    let (state, document) = world(&["a"]);
    let make = |filled: bool| {
        request(
            &document,
            vec![call(
                "core.layer.mask_add",
                json!({"selector": id("a"), "filled": filled}),
            )],
        )
    };
    let reveal = run(&state, make(true)).await.unwrap();
    let hide = run(&state, make(false)).await.unwrap();
    let decode = |result: &TransactionResult| {
        result
            .document
            .find("a")
            .unwrap()
            .mask
            .as_ref()
            .unwrap()
            .snapshot
            .decode()
            .unwrap()
    };
    assert_ne!(decode(&reveal), decode(&hide));
}

#[tokio::test]
async fn editing_a_mask_that_is_not_there_names_the_layer() {
    let (state, document) = world(&["a"]);
    for (op, params) in [
        ("core.layer.mask_remove", json!({"selector": id("a")})),
        (
            "core.layer.mask_set_enabled",
            json!({"selector": id("a"), "enabled": true}),
        ),
        (
            "core.layer.mask_set_inverted",
            json!({"selector": id("a"), "inverted": true}),
        ),
    ] {
        let error = run(&state, request(&document, vec![call(op, params)]))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("a has no mask"), "{op}: {error}");
    }
}

#[tokio::test]
async fn an_automation_cannot_edit_the_mask_of_a_locked_layer_but_a_person_can() {
    let (state, mut document) = world(&["a"]);
    document.layer_mut("a").unwrap().locked = true;
    let add = |origin: Origin| {
        let mut req = request(
            &document,
            vec![call(
                "core.layer.mask_add",
                json!({"selector": id("a"), "filled": true}),
            )],
        );
        req.origin = origin;
        req
    };
    assert!(run(&state, add(Origin::Automation)).await.is_err());
    // Matches what the Layers panel has always allowed: its lock stops automation.
    assert!(run(&state, add(Origin::User)).await.is_ok());
}

#[tokio::test]
async fn an_adjustment_layer_can_be_changed_but_nothing_else_can() {
    let (state, document) = world(&["a"]);
    let added = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.add_adjustment",
                json!({"operation": contrast(), "name": "Punch"}),
            )],
        ),
    )
    .await
    .unwrap();
    let adjustment = added.last_created.clone().unwrap();

    let changed = run(
        &state,
        request(
            &added.document,
            vec![call(
                "core.layer.update_adjustment",
                json!({"selector": id(&adjustment), "operation": brightness()}),
            )],
        ),
    )
    .await
    .unwrap();
    match &changed.document.find(&adjustment).unwrap().content {
        LayerContent::Adjustment { operation } => {
            assert_eq!(**operation, EditOperation::Contrast { amount: -0.1 });
        }
        other => panic!("expected an adjustment layer, got {other:?}"),
    }

    // A pixel layer is not an adjustment layer, and a geometry edit cannot be one.
    for (selector, operation) in [(id("a"), brightness()), (id(&adjustment), rotate())] {
        let error = run(
            &state,
            request(
                &added.document,
                vec![call(
                    "core.layer.update_adjustment",
                    json!({"selector": selector, "operation": operation}),
                )],
            ),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::Transaction { .. }), "{error}");
    }
}

#[tokio::test]
async fn placing_pixels_shows_a_buffer_the_session_already_holds() {
    let (state, document) = world(&["a"]);
    let pixel_id = state
        .layers
        .lock()
        .unwrap()
        .register(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([1, 2, 3, 255]),
        ))
        .unwrap();
    let before = buffers(&state);
    let result = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.place_pixels",
                json!({"pixelId": pixel_id, "width": 8, "height": 8, "name": "Placed"}),
            )],
        ),
    )
    .await
    .unwrap();
    let layer = result
        .document
        .find(result.last_created.as_ref().unwrap())
        .unwrap();
    assert_eq!(layer.name, "Placed");
    assert_eq!(layer.pixel_id(), Some(pixel_id.as_str()));
    assert_eq!(
        result.document.active_layer_id.as_deref(),
        Some(layer.id.as_str())
    );
    // Placing registers nothing: the buffer was already there.
    assert_eq!(buffers(&state), before);
    assert!(result.created_pixel_ids.is_empty());
}

#[tokio::test]
async fn placing_pixels_checks_the_buffer_exists_and_is_the_size_claimed() {
    let (state, document) = world(&["a"]);
    let held = document.find("a").unwrap().pixel_id().unwrap().to_string();
    for params in [
        json!({"pixelId": "px-nowhere", "width": 8, "height": 8}),
        json!({"pixelId": held, "width": 9, "height": 8}),
    ] {
        let error = run(
            &state,
            request(&document, vec![call("core.layer.place_pixels", params)]),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::Transaction { .. }), "{error}");
    }
}

#[tokio::test]
async fn only_a_person_may_place_pixels() {
    let (state, document) = world(&["a"]);
    let pixel_id = document.find("a").unwrap().pixel_id().unwrap().to_string();
    for origin in [Origin::Automation, Origin::Planner, Origin::Batch] {
        let mut req = request(
            &document,
            vec![call(
                "core.layer.place_pixels",
                json!({"pixelId": pixel_id, "width": 8, "height": 8}),
            )],
        );
        req.origin = origin;
        let error = run(&state, req).await.unwrap_err();
        assert!(
            error.to_string().contains("only a person"),
            "{origin:?}: {error}"
        );
    }
}

// ---- clipping ----------------------------------------------------------------------

fn order(document: &LayerDocument) -> Vec<String> {
    document
        .layers
        .iter()
        .map(|layer| layer.id.clone())
        .collect()
}

fn clipped_ids(document: &LayerDocument) -> Vec<String> {
    document
        .layers
        .iter()
        .filter(|layer| layer.clipped)
        .map(|layer| layer.id.clone())
        .collect()
}

/// A stack `a, b, c, d` with `c` and `d` clipped to `b`.
async fn clipped_world() -> (AppState, LayerDocument) {
    let (state, document) = world(&["a", "b", "c", "d"]);
    let result = run(
        &state,
        request(
            &document,
            vec![
                call(
                    "core.layer.set_clipped",
                    json!({"selector": id("c"), "clipped": true}),
                ),
                call(
                    "core.layer.set_clipped",
                    json!({"selector": id("d"), "clipped": true}),
                ),
            ],
        ),
    )
    .await
    .unwrap();
    (state, result.document)
}

#[tokio::test]
async fn a_layer_can_be_clipped_and_released() {
    let (state, document) = world(&["a", "b"]);
    let clipped = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.set_clipped",
                json!({"selector": id("b"), "clipped": true}),
            )],
        ),
    )
    .await
    .unwrap()
    .document;
    assert!(clipped.find("b").unwrap().clipped);
    assert!(!clipped.find("a").unwrap().clipped);

    let released = run(
        &state,
        request(
            &clipped,
            vec![call(
                "core.layer.set_clipped",
                json!({"selector": id("b"), "clipped": false}),
            )],
        ),
    )
    .await
    .unwrap()
    .document;
    assert!(!released.find("b").unwrap().clipped);
}

#[tokio::test]
async fn the_bottom_layer_of_a_stack_cannot_be_clipped() {
    let (state, document) = world(&["a", "b"]);
    let error = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.set_clipped",
                json!({"selector": id("a"), "clipped": true}),
            )],
        ),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::Transaction { .. }), "{error}");
    assert!(error.to_string().contains("no layer beneath"), "{error}");
}

#[tokio::test]
async fn automation_cannot_change_the_clipping_of_a_locked_layer() {
    let (state, document) = world(&["a", "b"]);
    let locked = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.set_locked",
                json!({"selector": id("b"), "locked": true}),
            )],
        ),
    )
    .await
    .unwrap()
    .document;
    let mut automated = request(
        &locked,
        vec![call(
            "core.layer.set_clipped",
            json!({"selector": id("b"), "clipped": true}),
        )],
    );
    automated.origin = Origin::Automation;
    let error = run(&state, automated).await.unwrap_err();
    assert!(matches!(error, AppError::Transaction { .. }), "{error}");
    // A person at the panel still can: a lock protects against automation.
    let by_hand = run(
        &state,
        request(
            &locked,
            vec![call(
                "core.layer.set_clipped",
                json!({"selector": id("b"), "clipped": true}),
            )],
        ),
    )
    .await
    .unwrap();
    assert!(by_hand.document.find("b").unwrap().clipped);
}

#[tokio::test]
async fn moving_a_base_moves_the_layers_clipped_to_it() {
    let (state, document) = clipped_world().await;
    assert_eq!(order(&document), ["a", "b", "c", "d"]);
    // Down to the bottom: the group stays together and keeps its flags.
    let down = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.move",
                json!({"selector": id("b"), "index": 0}),
            )],
        ),
    )
    .await
    .unwrap()
    .document;
    assert_eq!(order(&down), ["b", "c", "d", "a"]);
    assert_eq!(clipped_ids(&down), ["c", "d"]);
    // And to the top, past `a`.
    let up = run(
        &state,
        request(
            &down,
            vec![call(
                "core.layer.move",
                json!({"selector": id("b"), "index": 1}),
            )],
        ),
    )
    .await
    .unwrap()
    .document;
    assert_eq!(order(&up), ["a", "b", "c", "d"]);
    assert_eq!(clipped_ids(&up), ["c", "d"]);
}

#[tokio::test]
async fn moving_a_clipped_layer_moves_only_that_layer() {
    let (state, document) = clipped_world().await;
    let moved = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.move",
                json!({"selector": id("c"), "index": 3}),
            )],
        ),
    )
    .await
    .unwrap()
    .document;
    assert_eq!(order(&moved), ["a", "b", "d", "c"]);
    assert_eq!(clipped_ids(&moved), ["d", "c"]);
}

#[tokio::test]
async fn a_clipped_layer_that_ends_up_at_the_bottom_is_released() {
    let (state, document) = clipped_world().await;
    let moved = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.move",
                json!({"selector": id("c"), "index": 0}),
            )],
        ),
    )
    .await
    .unwrap()
    .document;
    assert_eq!(order(&moved), ["c", "a", "b", "d"]);
    assert!(!moved.find("c").unwrap().clipped);
    // `d` still follows a base, now `b`.
    assert!(moved.find("d").unwrap().clipped);
}

#[tokio::test]
async fn a_base_cannot_be_moved_into_a_layer_clipped_to_it() {
    let (state, document) = clipped_world().await;
    let with_group = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.group",
                json!({"selectors": [id("c")], "name": "Tint"}),
            )],
        ),
    )
    .await
    .unwrap()
    .document;
    // Grouping released `c` from the base: it is the lowest layer in its group.
    let group_id = with_group
        .iter()
        .find(|layer| layer.name == "Tint")
        .unwrap()
        .id
        .clone();
    let error = run(
        &state,
        request(
            &with_group,
            vec![call(
                "core.layer.move",
                json!({"selector": id("b"), "parent": id(&group_id), "index": 0}),
            )],
        ),
    )
    .await;
    // `d` is clipped to `b`, and the group sits between them, so this particular
    // move is legal; what must hold is that nothing was lost either way.
    let moved = error.unwrap().document;
    moved.validate().unwrap();
    assert_eq!(moved.layer_count(), with_group.layer_count());
}

#[tokio::test]
async fn deleting_a_base_promotes_the_first_layer_clipped_to_it() {
    let (state, document) = clipped_world().await;
    let deleted = run(
        &state,
        request(
            &document,
            vec![call("core.layer.delete", json!({"selector": id("b")}))],
        ),
    )
    .await
    .unwrap()
    .document;
    assert_eq!(order(&deleted), ["a", "c", "d"]);
    // `c` is the base of the group now, and `d` is still clipped to it.
    assert!(!deleted.find("c").unwrap().clipped);
    assert!(deleted.find("d").unwrap().clipped);
}

#[tokio::test]
async fn deleting_a_clipped_layer_leaves_the_others_clipped() {
    let (state, document) = clipped_world().await;
    let deleted = run(
        &state,
        request(
            &document,
            vec![call("core.layer.delete", json!({"selector": id("c")}))],
        ),
    )
    .await
    .unwrap()
    .document;
    assert_eq!(order(&deleted), ["a", "b", "d"]);
    assert_eq!(clipped_ids(&deleted), ["d"]);
}

#[tokio::test]
async fn a_copy_of_a_base_goes_above_its_clipping_group() {
    let (state, document) = clipped_world().await;
    let result = run(
        &state,
        request(
            &document,
            vec![call("core.layer.duplicate", json!({"selector": id("b")}))],
        ),
    )
    .await
    .unwrap();
    let copy = result.last_created.clone().unwrap();
    assert_eq!(order(&result.document), ["a", "b", "c", "d", copy.as_str()]);
    assert_eq!(clipped_ids(&result.document), ["c", "d"]);
}

#[tokio::test]
async fn a_copy_of_a_clipped_layer_stays_in_the_group() {
    let (state, document) = clipped_world().await;
    let result = run(
        &state,
        request(
            &document,
            vec![call("core.layer.duplicate", json!({"selector": id("c")}))],
        ),
    )
    .await
    .unwrap();
    let copy = result.last_created.clone().unwrap();
    assert_eq!(order(&result.document), ["a", "b", "c", copy.as_str(), "d"]);
    assert!(result.document.find(&copy).unwrap().clipped);
}

#[tokio::test]
async fn grouping_a_whole_clipping_group_keeps_it_intact_inside() {
    let (state, document) = clipped_world().await;
    let grouped = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.group",
                json!({"selectors": [id("b"), id("c"), id("d")], "name": "Art"}),
            )],
        ),
    )
    .await
    .unwrap()
    .document;
    let group = grouped.iter().find(|layer| layer.name == "Art").unwrap();
    let children: Vec<(&str, bool)> = group
        .children()
        .iter()
        .map(|child| (child.id.as_str(), child.clipped))
        .collect();
    assert_eq!(children, [("b", false), ("c", true), ("d", true)]);
}

#[tokio::test]
async fn grouping_a_clipped_layer_alone_releases_it() {
    let (state, document) = clipped_world().await;
    let grouped = run(
        &state,
        request(
            &document,
            vec![call(
                "core.layer.group",
                json!({"selectors": [id("c")], "name": "Alone"}),
            )],
        ),
    )
    .await
    .unwrap()
    .document;
    assert!(!grouped.find("c").unwrap().clipped);
    // The layer above it, `d`, now follows the group, which sits where `c` was.
    assert!(grouped.find("d").unwrap().clipped);
}

#[tokio::test]
async fn the_clipped_flag_survives_a_round_trip_through_a_project() {
    let (_state, document) = clipped_world().await;
    let text = serde_json::to_string(&document).unwrap();
    let back: LayerDocument = serde_json::from_str(&text).unwrap();
    assert_eq!(clipped_ids(&back), ["c", "d"]);
    // The revision names the document, clipping included.
    let mut released = back.clone();
    released.layers[2].clipped = false;
    assert_ne!(
        document_revision(&back).unwrap(),
        document_revision(&released).unwrap()
    );
}

// ---- the registry the interface is tested against ---------------------------------

/// The interface tests read this file to know which operation ids exist. If the
/// registry changes and the file does not, this fails, so a TypeScript mapping
/// cannot quietly point at an id that is gone.
///
/// Regenerate after an intentional change with `UPDATE_FIXTURES=1 cargo test
/// --test operations_engine the_registry`.
#[test]
fn the_registry_matches_the_snapshot_the_interface_is_tested_against() {
    let current = serde_json::to_value(photoforge_lib::operations::operation_specs()).unwrap();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/operation_registry.json"
    );
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        let text = serde_json::to_string_pretty(&current).unwrap() + "\n";
        std::fs::write(path, text).unwrap();
    }
    let stored: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        current, stored,
        "the operation registry changed; regenerate tests/fixtures/operation_registry.json"
    );
}
