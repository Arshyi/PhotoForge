//! The Layers panel's requests, run through the real engine.
//!
//! Since 0.15.0 the Layers panel makes its edits by asking the transaction engine. This is the
//! other half of `src/lib/layers/panelOps.test.ts`, which writes every plan the panel can send —
//! its label, its calls, and a document each is valid on — to `tests/fixtures/panel_plans.json`.
//! Each is run here as the person at the keyboard (`Origin::User`), exactly as the application
//! sends it. A parameter of the wrong type, a blend mode spelt the wrong way or a selector that
//! resolves to nothing fails here instead of at somebody's first click.
use image::{Rgba, RgbaImage};
use photoforge_lib::application::AppState;
use photoforge_lib::layers::{BlendMode, LayerContent, LayerDocument, LayerMask};
use photoforge_lib::mask::{MaskBitmap, MaskSnapshot};
use photoforge_lib::operations::support::FixedClock;
use photoforge_lib::operations::{
    execute_with, OperationCall, Origin, SequenceIds, TransactionRequest,
};
use serde_json::Value;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/panel_plans.json"
);

/// The plan fixture's mask is a placeholder that stands for "this layer has a mask"; the engine
/// needs one it can decode.
fn real_mask() -> LayerMask {
    LayerMask {
        snapshot: MaskSnapshot::encode(&MaskBitmap::full(8, 8).unwrap()),
        enabled: true,
        inverted: false,
    }
}

async fn run(plan: &Value) -> Result<(LayerDocument, LayerDocument), String> {
    let mut document: LayerDocument = serde_json::from_value(plan["document"].clone())
        .map_err(|error| format!("the fixture document does not load: {error}"))?;
    if let Some(layer) = document.layer_mut("m") {
        layer.mask = Some(real_mask());
    }
    let state = AppState::default();
    {
        let mut store = state.layers.lock().unwrap();
        store.reset(8, 8).unwrap();
        // The buffer a placed image would already have been decoded into.
        store
            .register_with_id(
                "px-placed",
                RgbaImage::from_pixel(4, 3, Rgba([9, 9, 9, 255])),
            )
            .unwrap();
    }
    let steps: Vec<OperationCall> = plan["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| OperationCall {
            op: step["op"].as_str().unwrap().to_string(),
            params: step.get("params").cloned().unwrap_or(Value::Null),
            when: None,
        })
        .collect();
    let request = TransactionRequest {
        document: document.clone(),
        label: plan["label"].as_str().unwrap().to_string(),
        steps,
        expected_revision: None,
        selection: None,
        // The Layers panel is a person at the keyboard.
        origin: Origin::User,
        plugin: None,
    };
    let mut ids = SequenceIds::default();
    execute_with(&state, request, &mut ids, &FixedClock("T".into()))
        .await
        .map(|result| (document, result.document))
        .map_err(|error| error.to_string())
}

#[tokio::test]
async fn every_plan_the_panel_sends_is_accepted_and_changes_the_document() {
    let text = std::fs::read_to_string(FIXTURE)
        .expect("run the panel vitest with UPDATE_FIXTURES=1 to create the fixture");
    let plans: Vec<Value> = serde_json::from_str(&text).unwrap();
    assert!(plans.len() >= 25, "only {} plans", plans.len());

    let mut failures = Vec::new();
    for plan in &plans {
        let name = plan["name"].as_str().unwrap();
        match run(plan).await {
            Ok((before, after)) => {
                if serde_json::to_value(&before).unwrap() == serde_json::to_value(&after).unwrap() {
                    failures.push(format!(
                        "{name}: the engine accepted it and changed nothing"
                    ));
                }
            }
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} plans failed:\n{}",
        failures.len(),
        plans.len(),
        failures.join("\n")
    );
}

/// What a few of the plans must actually do, so "accepted and changed something" is not the
/// whole claim.
#[tokio::test]
async fn the_plans_do_what_their_labels_say() {
    let text = std::fs::read_to_string(FIXTURE).unwrap();
    let plans: Vec<Value> = serde_json::from_str(&text).unwrap();
    let plan = |name: &str| {
        plans
            .iter()
            .find(|plan| plan["name"] == name)
            .unwrap_or_else(|| panic!("no plan called {name}"))
    };

    let (_, after) = run(plan("hide")).await.unwrap();
    assert!(!after.find("a").unwrap().visible);

    let (_, after) = run(plan("lock")).await.unwrap();
    assert!(after.find("a").unwrap().locked);

    let (_, after) = run(plan("opacity")).await.unwrap();
    assert!((after.find("a").unwrap().opacity - 0.35).abs() < 1e-6);

    let (_, after) = run(plan("blend soft light")).await.unwrap();
    assert_eq!(after.find("a").unwrap().blend_mode, BlendMode::SoftLight);

    let (_, after) = run(plan("move into a group")).await.unwrap();
    let group = after.find("g").unwrap();
    assert!(group.children().iter().any(|child| child.id == "a"));

    let (_, after) = run(plan("delete")).await.unwrap();
    assert!(after.find("a").is_none());

    let (_, after) = run(plan("group several")).await.unwrap();
    assert!(after.find("a").is_some() && after.find("g").is_some());
    assert_eq!(after.layers.len(), 3, "two layers became one group");

    let (_, after) = run(plan("pass through")).await.unwrap();
    assert!(matches!(
        after.find("g").unwrap().content,
        LayerContent::Group {
            isolated: false,
            ..
        }
    ));

    let (_, after) = run(plan("hide mask")).await.unwrap();
    assert!(after.find("a").unwrap().mask.is_some());

    let (_, after) = run(plan("disable mask")).await.unwrap();
    assert!(!after.find("m").unwrap().mask.as_ref().unwrap().enabled);

    let (_, after) = run(plan("invert mask")).await.unwrap();
    assert!(after.find("m").unwrap().mask.as_ref().unwrap().inverted);

    let (_, after) = run(plan("delete mask")).await.unwrap();
    assert!(after.find("m").unwrap().mask.is_none());

    let (_, after) = run(plan("place an image")).await.unwrap();
    let placed = after.layers.last().unwrap();
    assert_eq!(placed.name, "photo.png");
    assert_eq!(placed.pixel_id(), Some("px-placed"));
    assert_eq!(placed.pixel_dimensions(), Some((4, 3)));

    let (_, after) = run(plan("new pixel layer")).await.unwrap();
    assert_eq!(after.layers.len(), 5);
}
