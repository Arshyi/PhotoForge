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
