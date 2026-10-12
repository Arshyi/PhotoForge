//! The Rust engine against the TypeScript tree functions.
//!
//! `src/lib/operations/parity.test.ts` builds a document, an operation, and the
//! document the TypeScript function the Layers panel uses produces, and writes them
//! to `tests/fixtures/operations_parity.json`. This applies each operation through
//! the real engine and requires the same document.
//!
//! The two are compared after the same normalisation: identifiers an operation
//! creates become `#1`, `#2`… in the order they appear, timestamps become `T`, and
//! absent and null are the same. Nothing else is forgiven.
use photoforge_lib::application::AppState;
use photoforge_lib::layers::LayerDocument;
use photoforge_lib::operations::support::FixedClock;
use photoforge_lib::operations::{
    execute_with, OperationCall, Origin, SequenceIds, TransactionRequest,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/operations_parity.json"
);
const MERGE_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/operations_merge_parity.json"
);

fn collect_ids(layers: &[Value], out: &mut Vec<String>) {
    for layer in layers {
        out.push(layer["id"].as_str().unwrap().to_string());
        if layer["content"]["type"] == "group" {
            if let Some(children) = layer["content"]["children"].as_array() {
                collect_ids(children, out);
            }
        }
    }
}

fn strip(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(strip).collect()),
        Value::Object(map) => {
            let sorted: BTreeMap<&String, &Value> = map.iter().collect();
            let mut out = Map::new();
            for (key, inner) in sorted {
                if !inner.is_null() {
                    out.insert(key.clone(), strip(inner));
                }
            }
            Value::Object(out)
        }
        // The engine holds opacity and transforms as `f32` and TypeScript as a double,
        // so 0.35 arrives as 0.3499999940395355 and 1 as 1.0. Both are compared as
        // the `f32` they are, which is the precision the document actually has.
        Value::Number(number) => match number.as_f64() {
            Some(value) => serde_json::Number::from_f64(f64::from(value as f32))
                .map_or_else(|| Value::Number(number.clone()), Value::Number),
            None => Value::Number(number.clone()),
        },
        other => other.clone(),
    }
}

fn walk(layer: &Value, fresh: &BTreeMap<String, String>) -> Value {
    let mut layer = layer.clone();
    let id = layer["id"].as_str().unwrap().to_string();
    layer["id"] = json!(fresh.get(&id).cloned().unwrap_or(id));
    layer["metadata"] = json!({"createdAt": "T", "modifiedAt": "T", "custom": {}});
    if layer["content"]["type"] == "group" {
        let children: Vec<Value> = layer["content"]["children"]
            .as_array()
            .map(|children| children.iter().map(|child| walk(child, fresh)).collect())
            .unwrap_or_default();
        layer["content"]["children"] = Value::Array(children);
    }
    layer
}

fn normalise(document: &Value, before: &HashSet<String>) -> Value {
    let mut ids = Vec::new();
    collect_ids(document["layers"].as_array().unwrap(), &mut ids);
    let mut fresh = BTreeMap::new();
    for id in ids {
        if !before.contains(&id) && !fresh.contains_key(&id) {
            let name = format!("#{}", fresh.len() + 1);
            fresh.insert(id, name);
        }
    }
    let mut out = document.clone();
    out["layers"] = Value::Array(
        document["layers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|layer| walk(layer, &fresh))
            .collect(),
    );
    out["activeLayerId"] = match document["activeLayerId"].as_str() {
        Some(id) => json!(fresh.get(id).cloned().unwrap_or_else(|| id.to_string())),
        None => Value::Null,
    };
    strip(&out)
}

#[tokio::test]
async fn every_vector_the_typescript_functions_produce_is_produced_by_the_engine() {
    let text =
        std::fs::read_to_string(FIXTURE).expect("run the parity vitest with UPDATE_FIXTURES=1");
    let vectors: Vec<Value> = serde_json::from_str(&text).unwrap();
    assert!(vectors.len() >= 25, "only {} vectors", vectors.len());

    let mut failures = Vec::new();
    for vector in &vectors {
        let name = vector["name"].as_str().unwrap();
        let document: LayerDocument = serde_json::from_value(vector["document"].clone())
            .unwrap_or_else(|error| panic!("{name}: the fixture document does not load: {error}"));
        let mut before = Vec::new();
        collect_ids(
            vector["document"]["layers"].as_array().unwrap(),
            &mut before,
        );
        let before: HashSet<String> = before.into_iter().collect();

        let state = AppState::default();
        let request = TransactionRequest {
            document,
            label: "parity".into(),
            steps: vec![OperationCall {
                op: vector["call"]["op"].as_str().unwrap().to_string(),
                params: vector["call"]["params"].clone(),
                when: None,
            }],
            expected_revision: None,
            selection: None,
            // The Layers panel is a person at the keyboard.
            origin: Origin::User,
            plugin: None,
        };
        let mut ids = SequenceIds::default();
        let result = execute_with(&state, request, &mut ids, &FixedClock("T".into())).await;
        match result {
            Ok(result) => {
                let actual = normalise(&serde_json::to_value(&result.document).unwrap(), &before);
                let expected = strip(&vector["expected"]);
                if actual != expected {
                    failures.push(format!(
                        "{name}\n  engine:     {actual}\n  typescript: {expected}"
                    ));
                }
            }
            Err(error) => failures.push(format!("{name}: the engine refused it: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} vectors differ:\n{}",
        failures.len(),
        vectors.len(),
        failures.join("\n")
    );
}

/// Merge Down is checked by the panel first and by the engine when it runs, and the
/// two must give the same answer, clipping included. The vectors in
/// `operations_merge_parity.json` say, for a document and a layer, whether the
/// TypeScript check refuses the merge, and what stack a merge that goes ahead leaves.
/// The engine is given the same document, with a small buffer behind every pixel
/// layer so that a merge it allows can actually run.
#[tokio::test]
async fn the_engine_refuses_and_allows_the_merges_the_typescript_check_does() {
    let text = std::fs::read_to_string(MERGE_FIXTURE)
        .expect("run the parity vitest with UPDATE_FIXTURES=1");
    let vectors: Vec<Value> = serde_json::from_str(&text).unwrap();
    assert!(vectors.len() >= 8, "only {} vectors", vectors.len());

    let mut failures = Vec::new();
    for vector in &vectors {
        let name = vector["name"].as_str().unwrap();
        let document: LayerDocument = serde_json::from_value(vector["document"].clone())
            .unwrap_or_else(|error| panic!("{name}: the fixture document does not load: {error}"));
        let state = AppState::default();
        {
            let mut store = state.layers.lock().unwrap();
            store.reset(8, 8).unwrap();
            for pixel_id in document.referenced_pixel_ids() {
                store
                    .register_with_id(
                        &pixel_id,
                        image::RgbaImage::from_pixel(8, 8, image::Rgba([90, 120, 200, 255])),
                    )
                    .unwrap();
            }
        }
        let request = TransactionRequest {
            document,
            label: "parity".into(),
            steps: vec![OperationCall {
                op: "core.layer.merge_down".into(),
                params: json!({"selector": {"type": "id", "id": vector["id"]}}),
                when: None,
            }],
            expected_revision: None,
            selection: None,
            origin: Origin::User,
            plugin: None,
        };
        let mut ids = SequenceIds::default();
        let result = execute_with(&state, request, &mut ids, &FixedClock("T".into())).await;
        let refused = vector["refused"].as_bool().unwrap();
        match (result, refused) {
            (Ok(_), true) => failures.push(format!("{name}: the engine allowed a refused merge")),
            (Err(error), false) => failures.push(format!("{name}: the engine refused it: {error}")),
            (Err(_), true) => {}
            (Ok(result), false) => {
                let actual: Vec<Value> = result
                    .document
                    .layers
                    .iter()
                    .map(|layer| json!([layer.name, layer.clipped]))
                    .collect();
                let expected = vector["after"].as_array().cloned().unwrap_or_default();
                if actual != expected {
                    failures.push(format!(
                        "{name}\n  engine:     {}\n  expected:   {}",
                        Value::Array(actual),
                        Value::Array(expected)
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} vectors differ:\n{}",
        failures.len(),
        vectors.len(),
        failures.join("\n")
    );
}
