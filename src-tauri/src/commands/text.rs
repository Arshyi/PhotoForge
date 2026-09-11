//! Commands for text layers: what fonts exist, which a project is missing, and
//! turning semantic content into pixels when the user asks for that.
//!
//! Nothing here downloads a font or reaches the network. The list is whatever
//! is already installed on the machine.
use serde::Serialize;
use tauri::State;

use crate::application::AppState;
use crate::error::AppError;
use crate::layers::{LayerDocument, LayerKind};
use crate::text;

use super::layers::{find_layer, render_subset_into_buffer, LayerPixelsResult};

/// The fonts this machine has.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontListResult {
    pub families: Vec<String>,
}

/// One font a document asks for, and whether this machine has it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontRequirement {
    /// The family exactly as the project stores it.
    pub family: String,
    pub available: bool,
    /// Which layers ask for it, so the user can find them.
    pub layer_ids: Vec<String>,
}

/// What a document needs in order to render its text as intended.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontRequirementsResult {
    pub requirements: Vec<FontRequirement>,
    /// True when at least one requested family is not installed. The document
    /// still renders — in a substitute face — and still asks for the original.
    pub has_missing: bool,
}

/// Lists the font families installed on this machine.
///
/// Discovery is done once and shared, so repeated calls are cheap; the first
/// one pays for enumerating the system font directory.
#[tauri::command]
pub async fn list_system_fonts() -> Result<FontListResult, AppError> {
    let families = tauri::async_runtime::spawn_blocking(text::available_families)
        .await
        .map_err(|_| AppError::ProcessingFailure("font discovery stopped".into()))?;
    Ok(FontListResult { families })
}

/// Reports which fonts a document asks for and which of them are missing.
///
/// Called when a project is opened. A missing font is surfaced rather than
/// repaired: the project keeps asking for the face it was authored in, so
/// opening it on a machine that has that face restores the intended setting.
#[tauri::command]
pub async fn inspect_document_fonts(
    document: LayerDocument,
) -> Result<FontRequirementsResult, AppError> {
    document.validate()?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut requirements: Vec<FontRequirement> = Vec::new();
        collect(&document.layers, &mut requirements);
        for source in document.smart_sources.values() {
            collect(&source.layers, &mut requirements);
        }
        for requirement in &mut requirements {
            requirement.available =
                requirement.family.is_empty() || text::family_is_available(&requirement.family);
        }
        requirements.sort_by(|a, b| a.family.cmp(&b.family));
        let has_missing = requirements.iter().any(|r| !r.available);
        FontRequirementsResult {
            requirements,
            has_missing,
        }
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("font inspection stopped".into()))?;
    Ok(result)
}

fn collect(layers: &[crate::layers::Layer], out: &mut Vec<FontRequirement>) {
    use crate::layers::LayerContent;
    for layer in layers {
        match &layer.content {
            LayerContent::Text { text } => {
                match out.iter_mut().find(|r| r.family == text.font_family) {
                    Some(existing) => existing.layer_ids.push(layer.id.clone()),
                    None => out.push(FontRequirement {
                        family: text.font_family.clone(),
                        available: false,
                        layer_ids: vec![layer.id.clone()],
                    }),
                }
            }
            LayerContent::Group { children, .. } => collect(children, out),
            LayerContent::Pixel { .. }
            | LayerContent::Shape { .. }
            | LayerContent::SmartObject { .. }
            | LayerContent::Adjustment { .. } => {}
        }
    }
}

/// Renders one text, shape or smart layer into a pixel buffer, so the caller can
/// replace it with an ordinary pixel layer.
///
/// This is the only way semantic content becomes pixels, and it happens because
/// the user asked. Nothing in the render path rasterises a text or shape layer
/// behind their back to make some other operation convenient: every operation
/// that touches these layers either works on the geometry or does not apply.
///
/// Placement is baked, because the resulting buffer is canvas sized and returns
/// with an identity transform. Opacity, blend mode and visibility are not, so
/// they stay editable on the replacement layer rather than being applied twice.
#[tauri::command]
pub async fn rasterize_semantic_layer(
    document: LayerDocument,
    layer_id: String,
    state: State<'_, AppState>,
) -> Result<LayerPixelsResult, AppError> {
    document.validate()?;
    let mut layer = find_layer(&document, &layer_id)?;
    if layer.locked {
        return Err(AppError::LayerLocked(layer.id));
    }
    match layer.kind() {
        LayerKind::Text | LayerKind::Shape | LayerKind::SmartObject => {}
        other => {
            return Err(AppError::InvalidLayerDocument(format!(
                "a {} layer has no semantic content to rasterize",
                other.id()
            )))
        }
    }
    layer.visible = true;
    layer.opacity = 1.0;
    layer.blend_mode = crate::layers::BlendMode::Normal;
    render_subset_into_buffer(document, vec![layer], &state).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::shape::ShapeColor;
    use crate::layers::text::TextContent;
    use crate::layers::{Layer, LayerContent};

    fn text_layer(id: &str, family: &str) -> Layer {
        let mut content = TextContent::new("Hello", 10.0, 40.0, 24.0);
        content.font_family = family.to_string();
        content.fill = ShapeColor::new(0.0, 0.0, 0.0, 1.0);
        Layer {
            content: LayerContent::Text {
                text: Box::new(content),
            },
            ..crate::layers::test_pixel_layer(id, "unused", 1, 1)
        }
    }

    fn document(layers: Vec<Layer>) -> LayerDocument {
        LayerDocument {
            layers,
            ..LayerDocument::new(200, 120)
        }
    }

    #[test]
    fn requirements_group_layers_by_family_and_find_the_missing_one() {
        let installed = text::available_families();
        let present = installed.first().cloned().unwrap_or_default();
        let document = document(vec![
            text_layer("a", &present),
            text_layer("b", "No Such Font Exists Here 12345"),
            text_layer("c", "No Such Font Exists Here 12345"),
            Layer {
                content: LayerContent::Group {
                    children: vec![text_layer("d", "No Such Font Exists Here 12345")],
                    isolated: true,
                },
                ..crate::layers::test_pixel_layer("g", "unused", 1, 1)
            },
        ]);

        let mut requirements: Vec<FontRequirement> = Vec::new();
        collect(&document.layers, &mut requirements);
        for requirement in &mut requirements {
            requirement.available =
                requirement.family.is_empty() || text::family_is_available(&requirement.family);
        }

        let missing = requirements
            .iter()
            .find(|r| r.family == "No Such Font Exists Here 12345")
            .expect("the missing family was not reported");
        assert!(!missing.available);
        // Including the one nested inside a group: a font requirement does not
        // stop being one because the layer sits in a folder.
        assert_eq!(missing.layer_ids, vec!["b", "c", "d"]);

        if !present.is_empty() {
            let found = requirements
                .iter()
                .find(|r| r.family == present)
                .expect("the installed family was not reported");
            assert!(found.available, "{present} was reported missing");
        }
    }

    #[test]
    fn a_document_with_no_text_needs_no_fonts() {
        let mut requirements = Vec::new();
        collect(&document(vec![]).layers, &mut requirements);
        assert!(requirements.is_empty());
    }
}
