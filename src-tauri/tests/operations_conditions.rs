//! Conditions on steps, and planning: the only decision an automation can make, and a
//! way to see what it would do before it does it.
use photoforge_lib::application::AppState;
use photoforge_lib::domain::EditOperation;
use photoforge_lib::error::AppError;
use photoforge_lib::layers::{LayerContent, LayerDocument};
use photoforge_lib::operations::structure::new_layer;
use photoforge_lib::operations::support::FixedClock;
use photoforge_lib::operations::{
    execute_with, plan, OperationCall, Origin, SequenceIds, TransactionRequest, TransactionResult,
};
use serde_json::{json, Value};

fn call(op: &str, params: Value) -> OperationCall {
    OperationCall {
        op: op.to_string(),
        params,
        when: None,
    }
}

fn when(call: OperationCall, condition: Value) -> OperationCall {
    OperationCall {
        when: Some(serde_json::from_value(condition).expect("a valid condition")),
        ..call
    }
}

fn named(name: &str) -> Value {
    json!({ "kind": "layer_exists", "selector": { "type": "name", "name": name } })
}

fn id(layer: &str) -> Value {
    json!({ "type": "id", "id": layer })
}

fn contrast() -> Value {
    serde_json::to_value(EditOperation::Contrast { amount: 0.25 }).unwrap()
}

fn world(layer_ids: &[&str]) -> (AppState, LayerDocument) {
    let state = AppState::default();
    let mut document = LayerDocument::new(8, 8);
    {
        let mut store = state.layers.lock().unwrap();
        store.reset(8, 8).unwrap();
        for layer in layer_ids {
            let pixel_id = store
                .register(image::RgbaImage::from_pixel(
                    8,
                    8,
                    image::Rgba([200, 100, 50, 255]),
                ))
                .unwrap();
            document.layers.push(new_layer(
                layer,
                layer,
                LayerContent::Pixel {
                    pixel_id,
                    width: 8,
                    height: 8,
                },
                "t0",
            ));
        }
    }
    document.active_layer_id = layer_ids.last().map(|layer| layer.to_string());
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
    execute_with(
        state,
        request,
        &mut ids,
        &FixedClock("2026-10-06T00:00:00.000Z".into()),
    )
    .await
}

#[tokio::test]
async fn a_condition_is_asked_of_the_document_as_it_is_when_the_step_is_reached() {
    let (state, document) = world(&["a", "b"]);
    let result = run(
        &state,
        request(
            &document,
            vec![
                call("core.layer.delete", json!({"selector": id("a")})),
                // `a` is gone by now, so this does not run. A condition asked once, at
                // the start, would have said it did.
                when(
                    call(
                        "core.layer.set_opacity",
                        json!({"selector": id("b"), "opacity": 0.1}),
                    ),
                    named("a"),
                ),
                when(
                    call(
                        "core.layer.set_opacity",
                        json!({"selector": id("b"), "opacity": 0.6}),
                    ),
                    named("b"),
                ),
                when(
                    call(
                        "core.layer.rename",
                        json!({"selector": id("b"), "name": "Never"}),
                    ),
                    json!({ "kind": "not", "condition": named("b") }),
                ),
            ],
        ),
    )
    .await
    .unwrap();
    let skipped: Vec<bool> = result.steps.iter().map(|step| step.skipped).collect();
    assert_eq!(skipped, [false, true, false, true]);
    assert_eq!(result.document.find("b").unwrap().opacity, 0.6);
    assert_eq!(result.document.find("b").unwrap().name, "b");
    assert!(result.document.find("a").is_none());
}

#[tokio::test]
async fn every_kind_of_condition_decides_a_step() {
    let (state, mut document) = world(&["a", "b", "c"]);
    document.active_layer_id = Some("b".into());
    let step = |condition: Value| {
        when(
            call("core.layer.add_group", json!({"name": "Ran"})),
            condition,
        )
    };
    let cases: Vec<(Value, bool)> = vec![
        (json!({"kind": "layer_count_at_least", "count": 3}), true),
        (json!({"kind": "layer_count_at_least", "count": 4}), false),
        (
            json!({"kind": "active_layer_kind", "layerKind": "pixel"}),
            true,
        ),
        (
            json!({"kind": "active_layer_kind", "layerKind": "group"}),
            false,
        ),
        (
            json!({"kind": "precision", "precision": "legacy_srgb8"}),
            true,
        ),
        (
            json!({"kind": "precision", "precision": "linear_srgb_f32"}),
            false,
        ),
        (named("c"), true),
        (named("zzz"), false),
        (json!({"kind": "not", "condition": named("zzz")}), true),
    ];
    for (condition, ran) in cases {
        let result = run(&state, request(&document, vec![step(condition.clone())]))
            .await
            .unwrap();
        assert_eq!(!result.steps[0].skipped, ran, "{condition}");
        assert_eq!(
            result.document.layers.len(),
            if ran { 4 } else { 3 },
            "{condition}"
        );
    }
}

#[tokio::test]
async fn last_created_can_be_a_condition_once_something_has_been() {
    let (state, document) = world(&["a"]);
    let made = run(
        &state,
        request(
            &document,
            vec![
                call("core.layer.add_group", json!({"name": "G"})),
                when(
                    call(
                        "core.layer.rename",
                        json!({"selector": {"type": "last_created"}, "name": "Made"}),
                    ),
                    json!({"kind": "layer_exists", "selector": {"type": "last_created"}}),
                ),
            ],
        ),
    )
    .await
    .unwrap();
    assert!(!made.steps[1].skipped);
    assert!(made.document.iter().any(|layer| layer.name == "Made"));
    // Before anything has been created it can only name a layer the caller imagined.
    let error = run(
        &state,
        request(
            &document,
            vec![when(
                call("core.layer.add_group", json!({})),
                json!({"kind": "layer_exists", "selector": {"type": "last_created"}}),
            )],
        ),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("condition refers"), "{error}");
}

#[tokio::test]
async fn a_bad_condition_is_refused_before_anything_runs() {
    let (state, document) = world(&["a"]);
    let before = buffers(&state);
    let deep = json!({"kind": "not", "condition": {"kind": "not", "condition": {"kind": "not", "condition": named("a")}}});
    for condition in [
        json!({"kind": "layer_count_at_least", "count": 0}),
        json!({"kind": "active_layer_kind", "layerKind": "photograph"}),
        deep,
    ] {
        let error = run(
            &state,
            request(
                &document,
                vec![
                    call(
                        "core.layer.apply_edit",
                        json!({"selector": id("a"), "operations": [contrast()]}),
                    ),
                    when(call("core.layer.add_group", json!({})), condition.clone()),
                ],
            ),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, AppError::Transaction { step: 2, .. }),
            "{condition}: {error:?}"
        );
        assert_eq!(buffers(&state), before, "{condition}: the edit ran");
    }
}

#[tokio::test]
async fn a_planner_cannot_name_a_layer_inside_a_condition() {
    let (state, document) = world(&["a"]);
    let mut planner = request(
        &document,
        vec![when(
            call(
                "core.layer.add_adjustment",
                json!({"operation": contrast()}),
            ),
            named("a"),
        )],
    );
    planner.origin = Origin::Planner;
    let error = run(&state, planner).await.unwrap_err();
    assert!(error.to_string().contains("selected layer"), "{error}");
    // The relative selectors are allowed, as they are in a step.
    let mut allowed = request(
        &document,
        vec![when(
            call(
                "core.layer.add_adjustment",
                json!({"operation": contrast()}),
            ),
            json!({"kind": "layer_exists", "selector": {"type": "active"}}),
        )],
    );
    allowed.origin = Origin::Planner;
    assert!(run(&state, allowed).await.is_ok());
}

#[tokio::test]
async fn a_failure_after_skipped_steps_still_leaves_nothing_behind() {
    let (state, document) = world(&["a", "b"]);
    let before = buffers(&state);
    let error = run(
        &state,
        request(
            &document,
            vec![
                call("core.layer.add_pixel", json!({})),
                when(
                    call("core.layer.add_pixel", json!({})),
                    named("nothing called this"),
                ),
                call("core.layer.delete", json!({"selector": id("gone")})),
            ],
        ),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, AppError::Transaction { step: 3, .. }),
        "{error:?}"
    );
    assert_eq!(buffers(&state), before);
}

#[tokio::test]
async fn planning_says_what_would_happen_and_produces_nothing() {
    let (state, document) = world(&["a", "b"]);
    let before = buffers(&state);
    let steps = || {
        vec![
            call(
                "core.layer.apply_edit",
                json!({"selector": id("a"), "operations": [contrast()]}),
            ),
            when(call("core.layer.add_group", json!({})), named("nothing")),
            call("core.layer.add_pixel", json!({})),
        ]
    };
    let report = plan(&state, &request(&document, steps())).await.unwrap();
    let skipped = |report: &[photoforge_lib::operations::StepReport]| {
        report.iter().map(|step| step.skipped).collect::<Vec<_>>()
    };
    assert_eq!(skipped(&report), [false, true, false]);
    assert_eq!(report[0].op, "core.layer.apply_edit");
    assert_eq!(buffers(&state), before, "planning registered pixels");
    // The same request, run for real, takes exactly the steps the plan said.
    let real = run(&state, request(&document, steps())).await.unwrap();
    assert_eq!(skipped(&report), skipped(&real.steps));
    // A request that cannot work is refused by planning, with the reason running gives.
    let bad = || vec![call("core.layer.delete", json!({"selector": id("nope")}))];
    let planned = plan(&state, &request(&document, bad())).await.unwrap_err();
    let executed = run(&state, request(&document, bad())).await.unwrap_err();
    assert_eq!(planned.to_string(), executed.to_string());
}
