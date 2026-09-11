//! Explicit smart-object authoring and local link operations.
//!
//! Loading a project never follows its paths. File reads below happen only at
//! these user-invoked commands; embedded content remains the render authority.
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::application::AppState;
use crate::error::AppError;
use crate::layers::smart::{LinkState, SmartLink, SmartObjectContent, SmartSource};
use crate::layers::{BlendMode, Layer, LayerContent, LayerDocument, LayerKind};
use crate::pixel::PixelBuffer;

/// A single explicit link check/import has a bounded amount of file I/O.
const MAX_LINK_BYTES: u64 = 256 * 1024 * 1024;
static NEXT_SMART_ID: AtomicU64 = AtomicU64::new(1);

fn fresh_id(document: &LayerDocument, prefix: &str) -> String {
    loop {
        let id = format!("{prefix}{}", NEXT_SMART_ID.fetch_add(1, Ordering::Relaxed));
        if !document.iter_all().any(|layer| layer.id == id)
            && !document.smart_sources.contains_key(&id)
        {
            return id;
        }
    }
}

fn instance(id: String, name: String, source_id: String) -> Layer {
    Layer {
        id,
        name,
        visible: true,
        locked: false,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        transform: Default::default(),
        mask: None,
        collapsed: false,
        metadata: Default::default(),
        raw: None,
        content: LayerContent::SmartObject {
            smart: Box::new(SmartObjectContent { source_id }),
        },
    }
}

fn replace_range(
    layers: &mut Vec<Layer>,
    parent: &[usize],
    start: usize,
    count: usize,
    replacement: Layer,
) -> Result<(), AppError> {
    if let Some((&index, rest)) = parent.split_first() {
        let children = layers
            .get_mut(index)
            .and_then(Layer::children_mut)
            .ok_or_else(|| AppError::InvalidLayerDocument("the source stack moved".into()))?;
        return replace_range(children, rest, start, count, replacement);
    }
    layers.splice(start..start + count, [replacement]);
    Ok(())
}

fn convert_document(
    mut document: LayerDocument,
    layer_ids: &[String],
) -> Result<LayerDocument, AppError> {
    document.validate()?;
    if document.precision != crate::pixel::DocumentPrecision::LinearSrgbF32 {
        return Err(AppError::InvalidLayerDocument(
            "smart objects require a linear-float document; convert the document precision before authoring semantic layers".into(),
        ));
    }
    if layer_ids.is_empty() {
        return Err(AppError::InvalidLayerDocument(
            "select a layer to convert".into(),
        ));
    }
    let first = super::layers::find_layer(&document, &layer_ids[0])?;
    let mut path = document
        .path_to(&first.id)
        .ok_or_else(|| AppError::LayerNotFound(first.id.clone()))?;
    if first.locked
        || layer_ids
            .iter()
            .any(|id| document.find(id).is_some_and(|l| l.locked))
    {
        return Err(AppError::LayerLocked(first.id));
    }
    let source_id = fresh_id(&document, "smart-source-");
    let instance_id = fresh_id(&document, "smart-layer-");
    let (source, replacement, start) = if layer_ids.len() == 1 {
        if first.kind() == LayerKind::Adjustment || first.is_pass_through() {
            return Err(AppError::InvalidLayerDocument(
                "a backdrop-dependent adjustment or pass-through group cannot become an isolated smart source alone".into(),
            ));
        }
        if first.kind() == LayerKind::SmartObject {
            return Err(AppError::InvalidLayerDocument(
                "this layer is already a smart object".into(),
            ));
        }
        let (width, height) = first
            .pixel_dimensions()
            .unwrap_or((document.canvas_width, document.canvas_height));
        let mut content = first.clone();
        content.transform = Default::default();
        content.mask = None;
        content.opacity = 1.0;
        content.blend_mode = BlendMode::Normal;
        content.visible = true;
        let mut replacement = instance(instance_id, first.name.clone(), source_id.clone());
        replacement.transform = first.transform;
        replacement.mask = first.mask;
        replacement.opacity = first.opacity;
        replacement.blend_mode = first.blend_mode;
        replacement.visible = first.visible;
        replacement.metadata = first.metadata;
        (
            SmartSource::new(width, height, vec![content]),
            replacement,
            *path.last().expect("layer path"),
        )
    } else {
        // A smart source, like a merge, renders on transparency. Reuse the
        // same contiguity and backdrop checks so conversion cannot change the
        // meaning of an adjustment or a blend-dependent selection.
        let ordered = super::layers::merge_targets(&document, layer_ids)?;
        let start = ordered
            .iter()
            .filter_map(|layer| document.path_to(&layer.id))
            .filter_map(|p| p.last().copied())
            .min()
            .expect("nonempty selection");
        (
            SmartSource::new(document.canvas_width, document.canvas_height, ordered),
            instance(instance_id, "Smart Object".into(), source_id.clone()),
            start,
        )
    };
    path.pop();
    document.active_layer_id = Some(replacement.id.clone());
    replace_range(
        &mut document.layers,
        &path,
        start,
        layer_ids.len(),
        replacement,
    )?;
    document.smart_sources.insert(source_id, source);
    document.validate()?;
    Ok(document)
}

/// Moves selected editable layers into a native-size embedded source. No
/// rendering or pixel mutation occurs. The caller records one history entry.
#[tauri::command]
pub async fn convert_layers_to_smart_object(
    document: LayerDocument,
    layer_ids: Vec<String>,
) -> Result<LayerDocument, AppError> {
    convert_document(document, &layer_ids)
}

fn replace_source(
    mut document: LayerDocument,
    source_id: &str,
    mut source: SmartSource,
) -> Result<LayerDocument, AppError> {
    document.validate()?;
    let previous = document.smart_sources.get(source_id).ok_or_else(|| {
        AppError::InvalidLayerDocument(format!("smart source {source_id} is missing"))
    })?;
    // Editing linked contents makes the embedded stack authoritative. A link
    // must never imply that edited content is still a faithful copy of a file.
    source.link = if source.width == previous.width
        && source.height == previous.height
        && source.layers == previous.layers
    {
        previous.link.clone()
    } else {
        None
    };
    document.smart_sources.insert(source_id.into(), source);
    document.validate()?;
    Ok(document)
}

#[tauri::command]
pub async fn update_smart_source(
    document: LayerDocument,
    source_id: String,
    source: SmartSource,
    state: State<'_, AppState>,
) -> Result<LayerDocument, AppError> {
    let updated = replace_source(document, &source_id, source)?;
    let store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    // Validate buffer references as well as the typed graph before accepting
    // the edit; no instance should become an unresolved reference on commit.
    store.resolve(&updated.referenced_pixel_ids(), false)?;
    Ok(updated)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartLinkStatus {
    pub source_id: String,
    pub state: LinkState,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartLinksResult {
    pub links: Vec<SmartLinkStatus>,
}

fn local_path(path: &str) -> Result<PathBuf, AppError> {
    SmartLink {
        path: path.into(),
        digest: "0".repeat(64),
        bytes: 0,
    }
    .validate()?;
    let result = PathBuf::from(path);
    if !result.is_absolute()
        || path.starts_with("\\\\")
        || path.starts_with("//")
        || path.contains("://")
        || path.contains('\0')
        || result.components().any(|p| p == Component::ParentDir)
    {
        return Err(AppError::ProjectIo("smart links require an absolute local path without parent traversal or device/network prefixes".into()));
    }
    // Windows resolves DOS device basenames even under an otherwise ordinary
    // drive path. Do not open those devices (or alternate data streams).
    for part in path.split(['\\', '/']).skip(1) {
        let stem = part
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end_matches([' ', '.'])
            .to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(AppError::ProjectIo(
                "smart links cannot name a device".into(),
            ));
        }
    }
    Ok(result)
}

/// Hash one open handle, stopping even if another process grows the file.
fn hash_reader(
    reader: &mut impl Read,
    mut copy: Option<&mut File>,
    limit: u64,
) -> Result<(String, u64), AppError> {
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|e| AppError::ProjectIo(e.to_string()))?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or(AppError::OutOfMemoryRisk)?;
        if total > limit {
            return Err(AppError::ProjectTooLarge {
                bytes: total,
                limit,
            });
        }
        hasher.update(&buffer[..count]);
        if let Some(file) = copy.as_deref_mut() {
            file.write_all(&buffer[..count])
                .map_err(|e| AppError::ProjectIo(e.to_string()))?;
        }
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

fn open_local(path: &str) -> Result<File, AppError> {
    let path = local_path(path)?;
    // A local-looking symlink/junction can target a network share. Refuse
    // reparse points component by component before following the path.
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        if !matches!(component, Component::Normal(_)) {
            continue;
        }
        let metadata =
            std::fs::symlink_metadata(&prefix).map_err(|e| AppError::ProjectIo(e.to_string()))?;
        let mut reparse = metadata.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            reparse |= metadata.file_attributes() & 0x400 != 0;
        }
        if reparse {
            return Err(AppError::ProjectIo(
                "smart links cannot traverse a symbolic link or reparse point".into(),
            ));
        }
    }
    let file = File::open(&path).map_err(|e| AppError::ProjectIo(e.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|e| AppError::ProjectIo(e.to_string()))?;
    if !metadata.is_file() {
        return Err(AppError::ProjectIo(
            "a smart link must name a regular file".into(),
        ));
    }
    if metadata.len() > MAX_LINK_BYTES {
        return Err(AppError::ProjectTooLarge {
            bytes: metadata.len(),
            limit: MAX_LINK_BYTES,
        });
    }
    Ok(file)
}

fn inspect_links(document: &LayerDocument) -> SmartLinksResult {
    let mut links = Vec::new();
    let mut inspected_bytes = 0_u64;
    let mut source_ids: Vec<_> = document.smart_sources.keys().collect();
    source_ids.sort_unstable();
    for source_id in source_ids {
        let source = &document.smart_sources[source_id];
        let (state, detail) = match &source.link {
            None => (LinkState::Embedded, None),
            Some(link) => {
                let result = (|| {
                    let mut file = open_local(&link.path)?;
                    let size = file
                        .metadata()
                        .map_err(|e| AppError::ProjectIo(e.to_string()))?
                        .len();
                    let remaining = MAX_LINK_BYTES.saturating_sub(inspected_bytes);
                    if size > remaining {
                        return Err(AppError::ProjectIo("link verification byte budget exceeded; embedded contents remain available".into()));
                    }
                    let hashed = hash_reader(&mut file, None, remaining)?;
                    inspected_bytes += hashed.1;
                    Ok(hashed)
                })();
                match result {
                    Ok((digest, bytes)) if digest.eq_ignore_ascii_case(&link.digest) && bytes == link.bytes => (LinkState::Available, None),
                    Ok(_) => (LinkState::Changed, Some("The file differs from the embedded source. Relink and explicitly accept replacement to update it.".into())),
                    Err(error) => (LinkState::Missing, Some(error.to_string())),
                }
            }
        };
        links.push(SmartLinkStatus {
            source_id: source_id.clone(),
            state,
            detail,
        });
    }
    links.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    SmartLinksResult { links }
}

/// Only invoke from an explicit Check Links action. A project open must not
/// cause reads of attacker-provided local paths.
#[tauri::command]
pub async fn inspect_smart_links(document: LayerDocument) -> Result<SmartLinksResult, AppError> {
    document.validate()?;
    tauri::async_runtime::spawn_blocking(move || inspect_links(&document))
        .await
        .map_err(|_| AppError::ProcessingFailure("link inspection worker stopped".into()))
}

fn snapshot_image(path: &str) -> Result<(PixelBuffer, SmartLink, String), AppError> {
    let mut input = open_local(path)?;
    let original = Path::new(path);
    let extension = original
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    if extension.len() > 16 || !extension.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(AppError::UnsupportedImageFormat);
    }
    let mut snapshot = tempfile::Builder::new()
        .prefix("photoforge-smart-")
        .suffix(&format!(".{extension}"))
        .tempfile()
        .map_err(|e| AppError::ProjectIo(e.to_string()))?;
    // Decode exactly the bytes that were hashed, not a second view of a file
    // that may have changed in between. The bounded private snapshot is removed
    // automatically on every return path.
    let (digest, bytes) = hash_reader(&mut input, Some(snapshot.as_file_mut()), MAX_LINK_BYTES)?;
    snapshot
        .flush()
        .map_err(|e| AppError::ProjectIo(e.to_string()))?;
    let loaded = crate::infrastructure::load_image(snapshot.path())?;
    let pixels = match loaded.working {
        Some(working) => PixelBuffer::LinearRgbaF32(working),
        None => loaded.original.to_rgba8().into(),
    };
    let name = original
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Smart Object")
        .chars()
        .take(256)
        .collect();
    Ok((
        pixels,
        SmartLink {
            path: path.into(),
            digest,
            bytes,
        },
        name,
    ))
}

fn pixel_source(document: &LayerDocument, pixels: &PixelBuffer, name: String) -> SmartSource {
    let (width, height) = pixels.dimensions();
    let mut layer = instance(fresh_id(document, "smart-content-"), name, String::new());
    layer.content = LayerContent::Pixel {
        pixel_id: "pending-smart-import".into(),
        width,
        height,
    };
    SmartSource::new(width, height, vec![layer])
}

#[tauri::command]
pub async fn import_smart_object(
    mut document: LayerDocument,
    path: String,
    linked: bool,
    state: State<'_, AppState>,
) -> Result<LayerDocument, AppError> {
    document.validate()?;
    let (pixels, link, name) = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        snapshot_image(&path)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("smart import worker stopped".into()))??;
    let source_id = fresh_id(&document, "smart-source-");
    let mut source = pixel_source(&document, &pixels, name.clone());
    source.link = linked.then_some(link);
    let layer = instance(fresh_id(&document, "smart-layer-"), name, source_id.clone());
    document.active_layer_id = Some(layer.id.clone());
    document.layers.push(layer);
    document.smart_sources.insert(source_id.clone(), source);
    document.validate()?;
    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let pixel_id = store.register_typed(pixels)?;
    if let LayerContent::Pixel {
        pixel_id: target, ..
    } = &mut document
        .smart_sources
        .get_mut(&source_id)
        .expect("inserted source")
        .layers[0]
        .content
    {
        *target = pixel_id;
    }
    Ok(document)
}

/// Relinks a file without rewriting it. Different bytes are rejected unless
/// the user explicitly accepts replacing the shared embedded source.
#[tauri::command]
pub async fn relink_smart_source(
    mut document: LayerDocument,
    source_id: String,
    path: String,
    accept_changed: bool,
    state: State<'_, AppState>,
) -> Result<LayerDocument, AppError> {
    document.validate()?;
    let previous = document
        .smart_sources
        .get(&source_id)
        .ok_or_else(|| AppError::InvalidLayerDocument("smart source is missing".into()))?
        .clone();
    let expected = previous
        .link
        .as_ref()
        .ok_or_else(|| {
            AppError::InvalidLayerDocument("this source is embedded, not linked".into())
        })?
        .clone();
    let (pixels, link, name) = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        snapshot_image(&path)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("smart relink worker stopped".into()))??;
    if link.digest.eq_ignore_ascii_case(&expected.digest) && link.bytes == expected.bytes {
        document
            .smart_sources
            .get_mut(&source_id)
            .expect("checked source")
            .link = Some(link);
        return Ok(document);
    }
    if !accept_changed {
        return Err(AppError::InvalidLayerDocument("the selected file differs from the expected source; explicit acceptance is required to replace its contents".into()));
    }
    let mut replacement = pixel_source(&document, &pixels, name);
    replacement.link = Some(link);
    document
        .smart_sources
        .insert(source_id.clone(), replacement);
    document.validate()?;
    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let pixel_id = store.register_typed(pixels)?;
    if let LayerContent::Pixel {
        pixel_id: target, ..
    } = &mut document
        .smart_sources
        .get_mut(&source_id)
        .expect("replaced source")
        .layers[0]
        .content
    {
        *target = pixel_id;
    }
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::test_pixel_layer;

    fn document() -> LayerDocument {
        let mut document = LayerDocument::new(100, 80);
        document.precision = crate::pixel::DocumentPrecision::LinearSrgbF32;
        document
            .layers
            .push(test_pixel_layer("original", "pixels", 20, 10));
        document
    }

    #[test]
    fn conversion_rejects_legacy_precision_before_mutating_the_tree() {
        let mut legacy = LayerDocument::new(100, 80);
        legacy
            .layers
            .push(test_pixel_layer("original", "pixels", 20, 10));
        let before = legacy.clone();
        let error = convert_document(legacy, &["original".into()]).unwrap_err();
        assert!(error.to_string().contains("linear-float"));
        assert_eq!(before.smart_sources.len(), 0);
        assert_eq!(before.layers[0].kind(), LayerKind::Pixel);
    }

    #[test]
    fn conversion_preserves_native_pixels_placement_and_source_identity() {
        let mut before = document();
        before.layers[0].transform.translate_x = 12.0;
        before.layers[0].opacity = 0.4;
        let after = convert_document(before.clone(), &["original".into()]).unwrap();
        assert_eq!(after.layers.len(), 1);
        assert_eq!(after.layers[0].kind(), LayerKind::SmartObject);
        assert_eq!(after.layers[0].transform, before.layers[0].transform);
        assert_eq!(after.layers[0].opacity, 0.4);
        let source = after.smart_sources.values().next().unwrap();
        assert_eq!((source.width, source.height), (20, 10));
        assert_eq!(source.layers[0].id, "original");
        assert_eq!(source.layers[0].pixel_id(), Some("pixels"));
        assert_eq!(source.layers[0].opacity, 1.0);
        assert_eq!(source.layers[0].transform, Default::default());
        assert_eq!(after.referenced_pixel_ids(), vec!["pixels"]);
    }

    #[test]
    fn conversion_rejects_duplicate_and_backdrop_dependent_ranges() {
        let mut before = document();
        before
            .layers
            .push(test_pixel_layer("second", "px2", 20, 10));
        assert!(convert_document(before.clone(), &["original".into(), "original".into()]).is_err());
        before.layers[1].blend_mode = BlendMode::Multiply;
        let single = convert_document(before.clone(), &["second".into()]).unwrap();
        assert_eq!(single.layers[1].blend_mode, BlendMode::Multiply);
        assert!(convert_document(before, &["missing".into()]).is_err());
    }

    #[test]
    fn edited_source_detaches_link_and_updates_registry_without_replacing_instances() {
        let mut before = convert_document(document(), &["original".into()]).unwrap();
        let id = before.smart_sources.keys().next().unwrap().clone();
        before.smart_sources.get_mut(&id).unwrap().link = Some(SmartLink {
            path: std::env::temp_dir()
                .join("source.png")
                .to_string_lossy()
                .into_owned(),
            digest: "0".repeat(64),
            bytes: 12,
        });
        let mut source = before.smart_sources[&id].clone();
        source.layers[0].opacity = 0.5;
        let after = replace_source(before.clone(), &id, source).unwrap();
        assert!(after.smart_sources[&id].link.is_none());
        assert_eq!(before.layers, after.layers);
        assert_eq!(after.smart_sources[&id].layers[0].opacity, 0.5);
    }

    #[test]
    fn link_reads_reject_network_devices_and_traversal() {
        for path in [
            "relative.png",
            "../source.png",
            "https://example.com/image.png",
            "\\\\server\\share\\image.png",
            "//server/share/file",
            "\\\\?\\C:\\image.png",
            "C:\\images\\..\\secret.png",
        ] {
            assert!(local_path(path).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn snapshot_hashes_exact_import_bytes_and_link_status_changes_without_replacing_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.png");
        image::RgbaImage::from_pixel(2, 3, image::Rgba([128, 0, 0, 255]))
            .save(&path)
            .unwrap();
        let (pixels, link, _) = snapshot_image(path.to_str().unwrap()).unwrap();
        assert_eq!(pixels.dimensions(), (2, 3));
        let expected = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
        assert_eq!(link.digest, expected);
        let mut document = convert_document(document(), &["original".into()]).unwrap();
        document.smart_sources.values_mut().next().unwrap().link = Some(link);
        assert_eq!(
            inspect_links(&document).links[0].state,
            LinkState::Available
        );
        image::RgbaImage::from_pixel(2, 3, image::Rgba([0, 255, 0, 255]))
            .save(&path)
            .unwrap();
        assert_eq!(inspect_links(&document).links[0].state, LinkState::Changed);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(inspect_links(&document).links[0].state, LinkState::Missing);
        assert_eq!(document.referenced_pixel_ids(), vec!["pixels"]);
    }
}
