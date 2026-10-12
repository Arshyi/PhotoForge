//! Merge Down and Convert to Smart Object against clipping.
//!
//! A clipped layer paints only where the layer it is clipped to has coverage. The
//! two operations replace layers with one, so they must either leave the rendered
//! document as it was or refuse and leave the document alone. Every test here
//! renders the document before and after through the compositor, as the application
//! does, and compares the pictures, so a merge that kept the right layers but lost
//! a clip shows up as a different picture.
use image::{Rgba, RgbaImage};
use photoforge_lib::application::AppState;
use photoforge_lib::commands::convert_layers_to_smart_object;
use photoforge_lib::domain::EditOperation;
use photoforge_lib::error::AppError;
use photoforge_lib::layers::{
    render_document_typed, Layer, LayerContent, LayerDocument, LayerKind, RenderOptions,
};
use photoforge_lib::operations::structure::new_layer;
use photoforge_lib::operations::support::FixedClock;
use photoforge_lib::operations::{
    execute_with, plan, OperationCall, Origin, SequenceIds, TransactionRequest, TransactionResult,
};
use photoforge_lib::pixel::DocumentPrecision;
use serde_json::json;

const WIDTH: u32 = 8;
const HEIGHT: u32 = 4;
const PRECISIONS: [DocumentPrecision; 2] = [
    DocumentPrecision::LegacySrgb8,
    DocumentPrecision::LinearSrgbF32,
];

/// What a layer holds, so that a test can name the shape it needs.
#[derive(Clone, Copy)]
enum Art {
    /// Opaque on the left four columns, half transparent on the next two, nothing
    /// on the last two: a base whose coverage ends before the canvas does.
    Shape,
    /// The whole canvas, partly transparent.
    Wash(u8),
    Solid([u8; 4]),
}

fn art(kind: Art) -> RgbaImage {
    RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| match kind {
        Art::Shape => match x {
            0..=3 => Rgba([210, 40, 30, 255]),
            4 | 5 => Rgba([210, 40, 30, 128]),
            _ => Rgba([0, 0, 0, 0]),
        },
        // Varies with the position, so a layer merged in the wrong order or placed
        // wrongly would not match by luck.
        Art::Wash(alpha) => Rgba([20 + 25 * x as u8, 60, 220 - 20 * y as u8, alpha]),
        Art::Solid(colour) => Rgba(colour),
    })
}

struct Spec {
    id: &'static str,
    art: Art,
    clipped: bool,
    opacity: f32,
    adjustment: bool,
}

fn layer(id: &'static str, art: Art) -> Spec {
    Spec {
        id,
        art,
        clipped: false,
        opacity: 1.0,
        adjustment: false,
    }
}

/// A layer that adjusts what is beneath it and has no pixels or shape of its own.
fn adjustment(id: &'static str) -> Spec {
    Spec {
        adjustment: true,
        ..layer(id, Art::Solid([0, 0, 0, 0]))
    }
}

impl Spec {
    fn clipped(mut self) -> Self {
        self.clipped = true;
        self
    }
    fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }
}

fn world(precision: DocumentPrecision, specs: Vec<Spec>) -> (AppState, LayerDocument) {
    let state = AppState::default();
    let mut document = LayerDocument::new(WIDTH, HEIGHT);
    document.precision = precision;
    {
        let mut store = state.layers.lock().unwrap();
        store.reset(WIDTH, HEIGHT).unwrap();
        for spec in specs {
            let content = if spec.adjustment {
                LayerContent::Adjustment {
                    operation: Box::new(EditOperation::Grayscale),
                }
            } else {
                LayerContent::Pixel {
                    pixel_id: store.register(art(spec.art)).unwrap(),
                    width: WIDTH,
                    height: HEIGHT,
                }
            };
            let mut layer = new_layer(spec.id, spec.id, content, "t0");
            layer.clipped = spec.clipped;
            layer.opacity = spec.opacity;
            document.layers.push(layer);
        }
    }
    document.active_layer_id = document.layers.last().map(|layer| layer.id.clone());
    (state, document)
}

/// The document as the compositor draws it.
fn picture(state: &AppState, document: &LayerDocument) -> RgbaImage {
    let resolved = state
        .layers
        .lock()
        .unwrap()
        .resolve(&document.referenced_pixel_ids(), false)
        .unwrap();
    let rendered = render_document_typed(document, &resolved, RenderOptions::default()).unwrap();
    rendered.encoded8().as_ref().clone()
}

fn largest_difference(a: &RgbaImage, b: &RgbaImage) -> i32 {
    a.pixels()
        .zip(b.pixels())
        .flat_map(|(a, b)| (0..4).map(move |i| (i32::from(a.0[i]) - i32::from(b.0[i])).abs()))
        .max()
        .unwrap_or(0)
}

/// The tests above would pass if the clipping made no difference to these pictures.
/// This is how they show that it does: with the clip lost, the picture is another one.
fn assert_picture_differs(state: &AppState, before: &LayerDocument, after: &LayerDocument) {
    let difference = largest_difference(&picture(state, before), &picture(state, after));
    assert!(
        difference > 8,
        "losing the clip changed nothing ({difference})"
    );
}

fn assert_same_picture(state: &AppState, before: &LayerDocument, after: &LayerDocument) {
    // The merge draws through the same renderer, so what differs is one more 8-bit
    // quantisation of the merged result, as in the project's other merge tests.
    let difference = largest_difference(&picture(state, before), &picture(state, after));
    assert!(difference <= 2, "the picture changed by {difference}");
}

/// Moves every layer from `from` up into one new isolated group.
fn into_group(document: &mut LayerDocument, from: usize, id: &str) {
    let children = document.layers.split_off(from);
    document.layers.push(new_layer(
        id,
        id,
        LayerContent::Group {
            children,
            isolated: true,
        },
        "t0",
    ));
    document.validate().unwrap();
}

fn buffers(state: &AppState) -> usize {
    state.layers.lock().unwrap().buffer_count()
}

/// A transaction of one step: Merge Down on the layer `id`.
fn merge_down(document: &LayerDocument, id: &str) -> TransactionRequest {
    TransactionRequest {
        document: document.clone(),
        label: "Merge down".into(),
        steps: vec![OperationCall {
            op: "core.layer.merge_down".into(),
            params: json!({"selector": {"type": "id", "id": id}}),
            when: None,
        }],
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

/// `[(name, clipped)]` from bottom to top.
fn shape(document: &LayerDocument) -> Vec<(String, bool)> {
    document
        .layers
        .iter()
        .map(|layer| (layer.name.clone(), layer.clipped))
        .collect()
}

fn shape_of(expected: &[(&str, bool)]) -> Vec<(String, bool)> {
    expected
        .iter()
        .map(|(name, clipped)| ((*name).to_string(), *clipped))
        .collect()
}

/// Asserts that Merge Down on `id` is refused for its clipping, by the engine's own
/// check and not by anything the caller did first, and that nothing was kept.
async fn assert_refused(state: &AppState, document: &LayerDocument, id: &str) {
    let before = buffers(state);
    let request = merge_down(document, id);
    let error = run(state, request.clone()).await.unwrap_err();
    match &error {
        AppError::Transaction { reason, .. } => assert!(
            reason.contains("clipp"),
            "refused, but not for clipping: {reason}"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(buffers(state), before, "a refused merge left pixels behind");
    // The plan is refused too, so a preview never promises what the run will not do.
    assert!(plan(state, &request).await.is_err());
}

// ---- merging a clipped layer into its base -------------------------------------------

#[tokio::test]
async fn a_clipped_layer_merges_into_its_base_without_changing_the_picture() {
    for precision in PRECISIONS {
        let (state, document) = world(
            precision,
            vec![
                layer("ground", Art::Solid([20, 140, 60, 255])),
                layer("base", Art::Shape),
                layer("ink", Art::Wash(200)).clipped(),
            ],
        );
        let merged = run(&state, merge_down(&document, "ink")).await.unwrap();
        assert_eq!(
            shape(&merged.document),
            shape_of(&[("ground", false), ("base", false)])
        );
        assert_same_picture(&state, &document, &merged.document);
    }
}

// ---- the first trigger: merging a layer into a clipped one ---------------------------

#[tokio::test]
async fn merging_a_base_into_a_layer_clipped_to_the_one_below_is_refused() {
    // [a, b (clipped to a), c], Merge Down on c. The merge would draw b as a base
    // of its own and give back a layer clipped to nothing.
    for precision in PRECISIONS {
        let (state, document) = world(
            precision,
            vec![
                layer("a", Art::Shape),
                layer("b", Art::Wash(220)).clipped(),
                layer("c", Art::Solid([240, 240, 20, 90])),
            ],
        );
        assert_refused(&state, &document, "c").await;
    }
}

// ---- the second trigger: layers clipped to a merged layer ----------------------------

#[tokio::test]
async fn merging_a_base_that_has_layers_clipped_to_it_is_refused() {
    // [a, b, c, x (clipped to c)], Merge Down on c into b. x would be released and
    // paint over the whole canvas.
    for precision in PRECISIONS {
        let (state, document) = world(
            precision,
            vec![
                layer("a", Art::Solid([20, 140, 60, 255])),
                layer("b", Art::Wash(255)),
                layer("c", Art::Shape),
                layer("x", Art::Solid([0, 0, 255, 255])).clipped(),
            ],
        );
        assert_refused(&state, &document, "c").await;
    }
}

#[tokio::test]
async fn merging_a_base_into_the_ground_with_a_clipped_layer_above_it_is_refused() {
    // [ground, base, ink (clipped)], Merge Down on base.
    for precision in PRECISIONS {
        let (state, document) = world(
            precision,
            vec![
                layer("ground", Art::Solid([20, 140, 60, 255])),
                layer("base", Art::Shape),
                layer("ink", Art::Wash(255)).clipped(),
            ],
        );
        assert_refused(&state, &document, "base").await;
    }
}

#[tokio::test]
async fn merging_the_middle_of_a_group_keeps_the_layers_above_it_clipped() {
    // [ground, base, ink (c), tint (c)], Merge Down on ink. tint stays clipped, now
    // to the merged layer, which has the base's shape.
    for precision in PRECISIONS {
        for base_opacity in [1.0, 0.5] {
            let (state, document) = world(
                precision,
                vec![
                    layer("ground", Art::Solid([20, 140, 60, 255])),
                    layer("base", Art::Shape).opacity(base_opacity),
                    layer("ink", Art::Wash(200)).clipped(),
                    layer("tint", Art::Solid([250, 250, 0, 100])).clipped(),
                ],
            );
            let merged = run(&state, merge_down(&document, "ink")).await.unwrap();
            assert_eq!(
                shape(&merged.document),
                shape_of(&[("ground", false), ("base", false), ("tint", true)])
            );
            assert_same_picture(&state, &document, &merged.document);
            let mut released = merged.document.clone();
            released.layer_mut("tint").unwrap().clipped = false;
            assert_picture_differs(&state, &document, &released);
        }
    }
}

#[tokio::test]
async fn merging_clipped_layers_into_each_other_stays_clipped_to_the_base() {
    // [ground, base, c1 (c), d (c)], Merge Down on d. The merged layer is c1's
    // place in the group: still clipped to base, not a base of its own.
    for precision in PRECISIONS {
        let (state, document) = world(
            precision,
            vec![
                layer("ground", Art::Solid([20, 140, 60, 255])),
                layer("base", Art::Shape),
                layer("c1", Art::Wash(230)).clipped().opacity(0.8),
                layer("d", Art::Solid([250, 250, 0, 120])).clipped(),
            ],
        );
        let merged = run(&state, merge_down(&document, "d")).await.unwrap();
        assert_eq!(
            shape(&merged.document),
            shape_of(&[("ground", false), ("base", false), ("c1", true)])
        );
        assert_same_picture(&state, &document, &merged.document);
        let mut released = merged.document.clone();
        released.layers[2].clipped = false;
        assert_picture_differs(&state, &document, &released);
    }
}

#[tokio::test]
async fn clipped_layers_above_a_merge_of_clipped_layers_stay_in_the_group() {
    // [ground, base, c1 (c), c2 (c), c3 (c)], Merge Down on c2.
    for precision in PRECISIONS {
        let (state, document) = world(
            precision,
            vec![
                layer("ground", Art::Solid([20, 140, 60, 255])),
                layer("base", Art::Shape),
                layer("c1", Art::Wash(230)).clipped(),
                layer("c2", Art::Solid([250, 250, 0, 120])).clipped(),
                layer("c3", Art::Solid([0, 0, 0, 80])).clipped(),
            ],
        );
        let merged = run(&state, merge_down(&document, "c2")).await.unwrap();
        assert_eq!(
            shape(&merged.document),
            shape_of(&[
                ("ground", false),
                ("base", false),
                ("c1", true),
                ("c3", true)
            ])
        );
        assert_same_picture(&state, &document, &merged.document);
    }
}

#[tokio::test]
async fn a_merge_inside_a_group_keeps_its_clipping() {
    // The same stack one level down.
    for precision in PRECISIONS {
        let (state, mut document) = world(
            precision,
            vec![
                layer("ground", Art::Solid([20, 140, 60, 255])),
                layer("base", Art::Shape),
                layer("ink", Art::Wash(200)).clipped(),
                layer("tint", Art::Solid([250, 250, 0, 100])).clipped(),
            ],
        );
        into_group(&mut document, 1, "group");
        document.active_layer_id = Some("tint".into());

        let merged = run(&state, merge_down(&document, "ink")).await.unwrap();
        let inner: Vec<(String, bool)> = shape(&LayerDocument {
            layers: merged.document.find("group").unwrap().children().to_vec(),
            ..merged.document.clone()
        });
        assert_eq!(inner, shape_of(&[("base", false), ("tint", true)]));
        assert_same_picture(&state, &document, &merged.document);
    }
}

#[tokio::test]
async fn layers_clipped_to_an_adjustment_cannot_be_clipped_to_its_replacement() {
    // An adjustment has no shape, so what is clipped to it paints as ordinary
    // layers. Merged into a pixel layer they would be clipped to it instead.
    let (state, document) = world(
        DocumentPrecision::LegacySrgb8,
        vec![
            adjustment("adjust"),
            layer("x", Art::Shape).clipped(),
            layer("y", Art::Wash(255)).clipped(),
        ],
    );
    // x merges into the adjustment beneath it, with y still clipped above: refused.
    assert_refused(&state, &document, "x").await;

    // Merge y into x first, with nothing clipped above the pair, and it is fine.
    let merged = run(&state, merge_down(&document, "y")).await.unwrap();
    assert_eq!(
        shape(&merged.document),
        shape_of(&[("adjust", false), ("x", true)])
    );
    assert_same_picture(&state, &document, &merged.document);
}

#[tokio::test]
async fn a_refused_merge_takes_the_whole_transaction_with_it() {
    // [ground, base, c (clipped), d (clipped), e]. Merging c into base is allowed;
    // merging e into d is not. The first step's merged pixels must not survive.
    let (state, document) = world(
        DocumentPrecision::LegacySrgb8,
        vec![
            layer("ground", Art::Solid([20, 140, 60, 255])),
            layer("base", Art::Shape),
            layer("c", Art::Wash(255)).clipped(),
            layer("d", Art::Wash(180)).clipped(),
            layer("e", Art::Solid([250, 250, 0, 100])),
        ],
    );
    let before = buffers(&state);
    let mut request = merge_down(&document, "c");
    request.steps.push(OperationCall {
        op: "core.layer.merge_down".into(),
        params: json!({"selector": {"type": "id", "id": "e"}}),
        when: None,
    });
    let error = run(&state, request).await.unwrap_err();
    assert!(
        matches!(&error, AppError::Transaction { step: 2, .. }),
        "{error:?}"
    );
    assert_eq!(buffers(&state), before, "the first merge's pixels leaked");
}

// ---- Convert to Smart Object ---------------------------------------------------------

fn layer_named<'a>(document: &'a LayerDocument, name: &str) -> &'a Layer {
    document
        .iter_all()
        .find(|layer| layer.name == name)
        .unwrap_or_else(|| panic!("no layer named {name}"))
}

#[tokio::test]
async fn converting_a_clipped_layer_keeps_it_clipped() {
    let (state, document) = world(
        DocumentPrecision::LinearSrgbF32,
        vec![
            layer("ground", Art::Solid([20, 140, 60, 255])),
            layer("base", Art::Shape),
            layer("tint", Art::Wash(200)).clipped().opacity(0.7),
            layer("top", Art::Solid([250, 250, 0, 100])).clipped(),
        ],
    );
    let converted = convert_layers_to_smart_object(document.clone(), vec!["tint".into()])
        .await
        .unwrap();
    let instance = layer_named(&converted, "tint");
    assert_eq!(instance.kind(), LayerKind::SmartObject);
    assert!(instance.clipped, "the instance lost its clip");
    // The source is a stack of its own, whose first layer is a base.
    let source = converted.smart_sources.values().next().unwrap();
    assert!(source.layers.iter().all(|layer| !layer.clipped));
    // And the layer above it is still clipped to the same group.
    assert!(layer_named(&converted, "top").clipped);
    assert_same_picture(&state, &document, &converted);
    let mut released = converted.clone();
    released.layer_mut(&instance.id.clone()).unwrap().clipped = false;
    assert_picture_differs(&state, &document, &released);
}

#[tokio::test]
async fn converting_a_base_leaves_what_is_clipped_to_it_clipped() {
    let (state, document) = world(
        DocumentPrecision::LinearSrgbF32,
        vec![
            layer("ground", Art::Solid([20, 140, 60, 255])),
            layer("base", Art::Shape),
            layer("ink", Art::Wash(200)).clipped(),
        ],
    );
    let converted = convert_layers_to_smart_object(document.clone(), vec!["base".into()])
        .await
        .unwrap();
    assert!(!layer_named(&converted, "base").clipped);
    assert!(layer_named(&converted, "ink").clipped);
    assert_same_picture(&state, &document, &converted);
}

#[tokio::test]
async fn converting_clipped_layers_together_makes_one_clipped_instance() {
    let (state, document) = world(
        DocumentPrecision::LinearSrgbF32,
        vec![
            layer("ground", Art::Solid([20, 140, 60, 255])),
            layer("base", Art::Shape),
            layer("c1", Art::Wash(230)).clipped().opacity(0.8),
            layer("d", Art::Solid([250, 250, 0, 120])).clipped(),
            layer("e", Art::Solid([0, 0, 0, 80])).clipped(),
        ],
    );
    let converted = convert_layers_to_smart_object(document.clone(), vec!["c1".into(), "d".into()])
        .await
        .unwrap();
    let instance = layer_named(&converted, "Smart Object");
    assert!(instance.clipped);
    let source = converted.smart_sources.values().next().unwrap();
    // Rendered in the source as ordinary layers, one over the other.
    assert!(source.layers.iter().all(|layer| !layer.clipped));
    assert!(layer_named(&converted, "e").clipped);
    assert_same_picture(&state, &document, &converted);
}

#[tokio::test]
async fn converting_a_base_with_part_of_its_group_keeps_the_rest_clipped() {
    let (state, document) = world(
        DocumentPrecision::LinearSrgbF32,
        vec![
            layer("ground", Art::Solid([20, 140, 60, 255])),
            layer("base", Art::Shape).opacity(0.9),
            layer("ink", Art::Wash(200)).clipped(),
            layer("tint", Art::Solid([250, 250, 0, 100])).clipped(),
        ],
    );
    let converted =
        convert_layers_to_smart_object(document.clone(), vec!["base".into(), "ink".into()])
            .await
            .unwrap();
    let instance = layer_named(&converted, "Smart Object");
    assert!(!instance.clipped);
    assert!(layer_named(&converted, "tint").clipped);
    let source = converted.smart_sources.values().next().unwrap();
    assert_eq!(
        source
            .layers
            .iter()
            .map(|layer| (layer.name.as_str(), layer.clipped))
            .collect::<Vec<_>>(),
        [("base", false), ("ink", true)]
    );
    assert_same_picture(&state, &document, &converted);
}

#[tokio::test]
async fn converting_a_selection_that_splits_a_clipping_group_is_refused() {
    let (_, document) = world(
        DocumentPrecision::LinearSrgbF32,
        vec![
            layer("a", Art::Shape),
            layer("b", Art::Wash(220)).clipped(),
            layer("c", Art::Solid([240, 240, 20, 90])),
            layer("x", Art::Solid([0, 0, 255, 255])).clipped(),
        ],
    );
    let before = serde_json::to_value(&document).unwrap();
    // Starts inside a group and reaches another base.
    assert!(
        convert_layers_to_smart_object(document.clone(), vec!["b".into(), "c".into()])
            .await
            .is_err()
    );
    // Ends on a base that has layers clipped above it, with another base inside.
    assert!(convert_layers_to_smart_object(
        document.clone(),
        vec!["a".into(), "b".into(), "c".into()]
    )
    .await
    .is_err());
    assert_eq!(before, serde_json::to_value(&document).unwrap());
    // The whole group, with its last layer, converts.
    convert_layers_to_smart_object(document, vec!["c".into(), "x".into()])
        .await
        .unwrap();
}
