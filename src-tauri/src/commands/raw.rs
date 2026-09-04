use crate::application::AppState;
use crate::color::DevelopmentParameters;
use crate::error::AppError;
use crate::raw::develop::{develop_sensor, DevelopedRaw, RenderScale};
use crate::raw::{
    decoder_capabilities, dng, inspect_raw_path, read_source_bytes, source_reference_for,
    verify_source, RawCaptureMetadata, RawError, RawInspection, RawLayerSource, RawSourceMode,
    RawSourceReference, RawSourceStatus, DECODER_ID, DECODER_VERSION,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawInspectionResult {
    pub inspection: RawInspection,
    pub capabilities: crate::raw::RawDecoderCapabilities,
}

/// What opening or re-developing a RAW layer produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDevelopResult {
    /// The buffer the developed image was registered under.
    pub pixel_id: String,
    pub width: u32,
    pub height: u32,
    /// The sensor's own dimensions, which a preview does not match.
    pub source_width: u32,
    pub source_height: u32,
    pub source: RawLayerSource,
    pub metadata: RawCaptureMetadata,
    /// The white-balance multipliers actually applied.
    pub multipliers: [f32; 3],
    /// Whether a camera colour matrix was available and used.
    pub color_managed: bool,
    pub demosaic: String,
    pub cfa_pattern: String,
    pub bits_per_sample: u8,
    pub black_level: [f32; 4],
    pub white_level: f32,
    pub processing_time_ms: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDevelopRequest {
    pub source: RawLayerSource,
    /// Overrides the parameters stored in the source, for a live edit.
    #[serde(default)]
    pub parameters: Option<DevelopmentParameters>,
    #[serde(default)]
    pub full_resolution: bool,
    /// Guards against a stale worker result replacing a newer edit.
    pub document_id: u64,
    pub request_id: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawSourceStatusResult {
    pub status: RawSourceStatus,
    pub expected_sha256: String,
}

fn map(error: RawError) -> AppError {
    AppError::RawInspection(error.to_string())
}

/// Inspects a user-selected RAW source without decoding or modifying it.
#[tauri::command]
pub fn inspect_raw(path: String) -> Result<RawInspectionResult, AppError> {
    let inspection = inspect_raw_path(&PathBuf::from(path)).map_err(map)?;
    Ok(RawInspectionResult {
        inspection,
        capabilities: decoder_capabilities(),
    })
}

/// Registers a developed image and describes what produced it.
fn register(
    state: &AppState,
    developed: DevelopedRaw,
    sensor: &dng::SensorImage,
    source: RawLayerSource,
    started: std::time::Instant,
    expected: Option<(u64, u64)>,
) -> Result<RawDevelopResult, AppError> {
    // Match the session -> store lock order used by Open. Keep session identity
    // stable until the immutable result has been registered.
    let session = state
        .session
        .lock()
        .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
    if let Some((document_id, request_id)) = expected {
        use std::sync::atomic::Ordering;
        if state.pending_open_request.load(Ordering::Acquire) != 0
            || !session
                .as_ref()
                .is_some_and(|s| s.document_id == document_id)
            || state.latest_raw_request.load(Ordering::Acquire) != request_id
        {
            return Err(AppError::RenderCancelled);
        }
    }
    let image = developed.image;
    let (width, height) = (image.width(), image.height());
    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let pixel_id = store.register_float(image)?;
    Ok(RawDevelopResult {
        pixel_id,
        width,
        height,
        source_width: developed.source_dimensions.0,
        source_height: developed.source_dimensions.1,
        source,
        metadata: sensor.metadata.clone(),
        multipliers: developed.multipliers,
        color_managed: developed.color_managed,
        demosaic: developed.demosaic.name().to_string(),
        cfa_pattern: sensor.cfa.name().to_string(),
        bits_per_sample: sensor.bits_per_sample,
        black_level: sensor.black_level,
        white_level: sensor.white_level,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
    })
}

/// Decodes a RAW file and develops it into a layer buffer.
///
/// The original file is only ever read. What comes back is a developed raster
/// plus the record needed to develop it again, which is what keeps the edit
/// non-destructive rather than baking a decision into pixels.
#[tauri::command]
pub async fn open_raw_layer(
    path: String,
    parameters: Option<DevelopmentParameters>,
    full_resolution: bool,
    state: State<'_, AppState>,
) -> Result<RawDevelopResult, AppError> {
    let started = std::time::Instant::now();
    let source_path = PathBuf::from(&path);
    let bytes = read_source_bytes(&source_path).map_err(map)?;
    let parameters = parameters.unwrap_or_default();
    parameters
        .validate()
        .map_err(|error| AppError::RawInspection(error.to_string()))?;

    let scale = if full_resolution {
        RenderScale::Full
    } else {
        RenderScale::Preview
    };
    let requested_parameters = parameters.clone();
    // Decoding and demosaicing a large sensor is CPU-bound work and must not
    // run on the interface thread.
    let decoded = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)
            .map_err(|e| RawError::InvalidMetadata(e.to_string()))?;
        let sensor = dng::decode(&bytes)?;
        let developed = develop_sensor(&sensor, &parameters, scale)?;
        Ok::<_, RawError>((sensor, developed))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("the RAW worker stopped".into()))?
    .map_err(map)?;
    let (sensor, developed) = decoded;

    let reference = source_reference_for(&source_path, sensor.width, sensor.height).map_err(map)?;
    let source = RawLayerSource {
        reference,
        mode: RawSourceMode::Linked {
            path: canonical_string(&source_path)?,
        },
        parameters: developed_parameters(&developed, requested_parameters),
        decoder: DECODER_ID.to_string(),
        decoder_version: DECODER_VERSION.to_string(),
        capture: sensor.metadata.clone(),
    };
    source.validate().map_err(map)?;
    register(&state, developed, &sensor, source, started, None)
}

/// The parameters that produced a development, with the resolved white balance
/// written back so reopening a project reproduces the same picture even if the
/// automatic estimate would now differ.
fn developed_parameters(
    developed: &DevelopedRaw,
    parameters: DevelopmentParameters,
) -> DevelopmentParameters {
    DevelopmentParameters {
        white_balance: crate::color::WhiteBalance::Custom {
            multipliers: developed.multipliers,
        },
        ..parameters
    }
}

fn canonical_string(path: &Path) -> Result<String, AppError> {
    crate::raw::canonical_source_path(path)
        .map_err(map)
        .map(|canonical| crate::raw::presentable_path(&canonical))
}

/// Re-develops a RAW layer from its original file with new parameters.
#[tauri::command]
pub async fn develop_raw_layer(
    request: RawDevelopRequest,
    state: State<'_, AppState>,
) -> Result<RawDevelopResult, AppError> {
    let started = std::time::Instant::now();
    if !super::sampling::is_current(&state, request.document_id)? {
        return Err(AppError::NoImageOpen);
    }
    state
        .latest_raw_request
        .store(request.request_id, std::sync::atomic::Ordering::Release);
    let _permit = state.raw_gate.lock().await;
    if state
        .latest_raw_request
        .load(std::sync::atomic::Ordering::Acquire)
        != request.request_id
    {
        return Err(AppError::RenderCancelled);
    }
    request.source.validate().map_err(map)?;
    let path = request
        .source
        .linked_path()
        .ok_or_else(|| {
            AppError::RawInspection(
                "this build can only re-develop a linked RAW source".to_string(),
            )
        })?
        .to_string();
    let source_path = PathBuf::from(&path);

    // The file must still be the photograph the project was built from. Binding
    // to a different one would silently put someone else's picture under these
    // edits.
    match verify_source(&request.source.reference, &source_path) {
        RawSourceStatus::Available => {}
        RawSourceStatus::Missing => {
            return Err(AppError::RawInspection(format!(
                "the RAW file for this layer is no longer at {path}"
            )))
        }
        RawSourceStatus::Changed { .. } => {
            return Err(AppError::RawInspection(format!(
                "the file at {path} is not the photograph this layer was developed from"
            )))
        }
    }

    let bytes = read_source_bytes(&source_path).map_err(map)?;
    let parameters = request
        .parameters
        .clone()
        .unwrap_or_else(|| request.source.parameters.clone());
    parameters
        .validate()
        .map_err(|error| AppError::RawInspection(error.to_string()))?;
    let scale = if request.full_resolution {
        RenderScale::Full
    } else {
        RenderScale::Preview
    };

    let for_worker = parameters.clone();
    let decoded = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)
            .map_err(|e| RawError::InvalidMetadata(e.to_string()))?;
        let sensor = dng::decode(&bytes)?;
        let developed = develop_sensor(&sensor, &for_worker, scale)?;
        Ok::<_, RawError>((sensor, developed))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("the RAW worker stopped".into()))?
    .map_err(map)?;
    let (sensor, developed) = decoded;

    let mut source = request.source.clone();
    source.decoder_version = DECODER_VERSION.to_string();
    source.parameters = developed_parameters(&developed, parameters);
    source.capture = sensor.metadata.clone();
    source.validate().map_err(map)?;
    if !super::sampling::is_current(&state, request.document_id)?
        || state
            .latest_raw_request
            .load(std::sync::atomic::Ordering::Acquire)
            != request.request_id
    {
        return Err(AppError::RenderCancelled);
    }
    register(
        &state,
        developed,
        &sensor,
        source,
        started,
        Some((request.document_id, request.request_id)),
    )
}

/// Reports whether a linked RAW source is present and unchanged.
#[tauri::command]
pub fn verify_raw_source(
    reference: RawSourceReference,
    path: String,
) -> Result<RawSourceStatusResult, AppError> {
    reference.validate().map_err(map)?;
    Ok(RawSourceStatusResult {
        status: verify_source(&reference, &PathBuf::from(path)),
        expected_sha256: reference.sha256.clone(),
    })
}

/// Points a RAW layer at a file the user chose after the original went missing.
///
/// The hash must match. A file that merely has the right name is refused, so a
/// relink can never quietly bind a project to a different photograph.
#[tauri::command]
pub fn relink_raw_source(source: RawLayerSource, path: String) -> Result<RawLayerSource, AppError> {
    source.validate().map_err(map)?;
    let candidate = PathBuf::from(&path);
    match verify_source(&source.reference, &candidate) {
        RawSourceStatus::Available => {}
        RawSourceStatus::Missing => {
            return Err(AppError::RawInspection(
                "there is no readable file at that location".into(),
            ))
        }
        RawSourceStatus::Changed { found_sha256 } => {
            return Err(AppError::RawInspection(format!(
                "that file is a different photograph: it hashes to {} but this layer was developed from {}",
                &found_sha256[..16.min(found_sha256.len())],
                &source.reference.sha256[..16.min(source.reference.sha256.len())]
            )))
        }
    }
    let mut relinked = source;
    relinked.mode = RawSourceMode::Linked {
        path: canonical_string(&candidate)?,
    };
    relinked.validate().map_err(map)?;
    Ok(relinked)
}

/// Develops a RAW layer at full sensor resolution and writes a true 16-bit PNG.
///
/// The preview the user was editing is never what gets exported: this decodes
/// the original again, demosaics with the high-quality algorithm, and quantises
/// only at the file boundary, so the extra precision the linear pipeline
/// carried is actually present in the result.
#[tauri::command]
pub async fn export_raw_layer_png16(
    output_path: String,
    source: RawLayerSource,
    parameters: Option<DevelopmentParameters>,
    state: State<'_, AppState>,
) -> Result<crate::domain::ExportResult, AppError> {
    source.validate().map_err(map)?;
    let path = source
        .linked_path()
        .ok_or_else(|| {
            AppError::RawInspection("this build can only export a linked RAW source".to_string())
        })?
        .to_string();
    let source_path = PathBuf::from(&path);
    if verify_source(&source.reference, &source_path) != RawSourceStatus::Available {
        return Err(AppError::RawInspection(format!(
            "the RAW file for this layer is missing or changed at {path}"
        )));
    }
    let parameters = parameters.unwrap_or_else(|| source.parameters.clone());
    parameters
        .validate()
        .map_err(|error| AppError::RawInspection(error.to_string()))?;

    let bytes = read_source_bytes(&source_path).map_err(map)?;
    let output = PathBuf::from(output_path);
    let _permit = state.export_gate.lock().await;
    let started = std::time::Instant::now();

    let (saved, width, height) = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        let sensor = dng::decode(&bytes).map_err(map)?;
        // Always the full sensor and the better algorithm, whatever the
        // interface was previewing.
        let developed = develop_sensor(&sensor, &parameters, RenderScale::Full).map_err(map)?;
        let dimensions = (developed.image.width(), developed.image.height());
        let saved =
            crate::infrastructure::save_float_png16(&developed.image, &source_path, &output)?;
        Ok::<_, AppError>((saved, dimensions.0, dimensions.1))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("the RAW export worker stopped".into()))??;

    Ok(crate::domain::ExportResult {
        output_path: saved.to_string_lossy().into_owned(),
        width,
        height,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
    })
}

/// Registers every RAW command on a Tauri builder, so integration tests can
/// drive them through the real IPC boundary.
#[doc(hidden)]
pub fn register_raw_commands<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        inspect_raw,
        open_raw_layer,
        develop_raw_layer,
        verify_raw_source,
        relink_raw_source,
        export_raw_layer_png16
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::dng::fixtures::DngBuilder;

    fn write_dng(directory: &Path, name: &str, level: u16) -> PathBuf {
        let path = directory.join(name);
        let data = DngBuilder::new(8, 8, vec![level; 64])
            .levels(vec![0], (1, 1), 4095)
            .build();
        std::fs::write(&path, data).expect("writes");
        path
    }

    fn reference_for(path: &Path) -> RawSourceReference {
        source_reference_for(path, 8, 8).expect("reference")
    }

    #[test]
    fn a_present_unchanged_source_is_available() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_dng(directory.path(), "shot.dng", 1000);
        let reference = reference_for(&path);
        assert_eq!(verify_source(&reference, &path), RawSourceStatus::Available);
    }

    #[test]
    fn a_deleted_source_reports_missing_rather_than_erroring() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_dng(directory.path(), "shot.dng", 1000);
        let reference = reference_for(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(verify_source(&reference, &path), RawSourceStatus::Missing);
    }

    /// The case that matters most: a file with the right name but different
    /// contents must never be accepted silently.
    #[test]
    fn a_different_photograph_at_the_same_path_reports_changed() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_dng(directory.path(), "shot.dng", 1000);
        let reference = reference_for(&path);
        // Overwrite with a different exposure of the same shape.
        write_dng(directory.path(), "shot.dng", 2000);
        match verify_source(&reference, &path) {
            RawSourceStatus::Changed { found_sha256 } => {
                assert_ne!(found_sha256, reference.sha256);
                assert_eq!(found_sha256.len(), 64);
            }
            other => panic!("expected a changed source, got {other:?}"),
        }
    }

    #[test]
    fn relinking_accepts_the_same_photograph_moved_elsewhere() {
        let directory = tempfile::tempdir().unwrap();
        let original = write_dng(directory.path(), "shot.dng", 1234);
        let reference = reference_for(&original);
        let source = RawLayerSource {
            reference,
            mode: RawSourceMode::Linked {
                path: original.to_string_lossy().into_owned(),
            },
            parameters: DevelopmentParameters::default(),
            decoder: DECODER_ID.into(),
            decoder_version: DECODER_VERSION.into(),
            capture: RawCaptureMetadata::default(),
        };
        // The same bytes, at a new location.
        let moved = directory.path().join("archive.dng");
        std::fs::copy(&original, &moved).unwrap();
        let relinked =
            relink_raw_source(source.clone(), moved.to_string_lossy().into_owned()).unwrap();
        assert!(relinked.linked_path().unwrap().ends_with("archive.dng"));
        assert_eq!(relinked.reference.sha256, source.reference.sha256);
    }

    #[test]
    fn relinking_refuses_a_different_photograph_and_says_why() {
        let directory = tempfile::tempdir().unwrap();
        let original = write_dng(directory.path(), "shot.dng", 1234);
        let other = write_dng(directory.path(), "other.dng", 4000);
        let source = RawLayerSource {
            reference: reference_for(&original),
            mode: RawSourceMode::Linked {
                path: original.to_string_lossy().into_owned(),
            },
            parameters: DevelopmentParameters::default(),
            decoder: DECODER_ID.into(),
            decoder_version: DECODER_VERSION.into(),
            capture: RawCaptureMetadata::default(),
        };
        let error = relink_raw_source(source, other.to_string_lossy().into_owned())
            .expect_err("a different photograph must be refused");
        let message = error.to_string();
        assert!(
            message.contains("different photograph"),
            "the refusal did not explain itself: {message}"
        );
    }

    #[test]
    fn relinking_refuses_a_path_with_nothing_at_it() {
        let directory = tempfile::tempdir().unwrap();
        let original = write_dng(directory.path(), "shot.dng", 1234);
        let source = RawLayerSource {
            reference: reference_for(&original),
            mode: RawSourceMode::Linked {
                path: original.to_string_lossy().into_owned(),
            },
            parameters: DevelopmentParameters::default(),
            decoder: DECODER_ID.into(),
            decoder_version: DECODER_VERSION.into(),
            capture: RawCaptureMetadata::default(),
        };
        let absent = directory.path().join("gone.dng");
        assert!(relink_raw_source(source, absent.to_string_lossy().into_owned()).is_err());
    }

    #[test]
    fn a_raw_layer_source_round_trips_through_json() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_dng(directory.path(), "shot.dng", 900);
        let source = RawLayerSource {
            reference: reference_for(&path),
            mode: RawSourceMode::Linked {
                path: path.to_string_lossy().into_owned(),
            },
            parameters: DevelopmentParameters {
                exposure_ev: 0.75,
                ..DevelopmentParameters::default()
            },
            decoder: DECODER_ID.into(),
            decoder_version: DECODER_VERSION.into(),
            capture: RawCaptureMetadata::default(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: RawLayerSource = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, source);
        restored.validate().unwrap();
        // The mode is an explicit tagged value, never inferred.
        assert!(json.contains("\"mode\":\"linked\""));
    }

    #[test]
    fn an_embedded_source_is_accepted_by_the_schema_for_forward_compatibility() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_dng(directory.path(), "shot.dng", 900);
        let source = RawLayerSource {
            reference: reference_for(&path),
            mode: RawSourceMode::Embedded,
            parameters: DevelopmentParameters::default(),
            decoder: DECODER_ID.into(),
            decoder_version: DECODER_VERSION.into(),
            capture: RawCaptureMetadata::default(),
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: RawLayerSource = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.mode, RawSourceMode::Embedded);
        assert!(restored.linked_path().is_none());
    }

    #[test]
    fn an_invalid_source_record_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_dng(directory.path(), "shot.dng", 900);
        let mut source = RawLayerSource {
            reference: reference_for(&path),
            mode: RawSourceMode::Linked {
                path: path.to_string_lossy().into_owned(),
            },
            parameters: DevelopmentParameters::default(),
            decoder: DECODER_ID.into(),
            decoder_version: DECODER_VERSION.into(),
            capture: RawCaptureMetadata::default(),
        };
        source.decoder = String::new();
        assert!(source.validate().is_err());

        source.decoder = DECODER_ID.into();
        source.parameters.exposure_ev = f32::NAN;
        assert!(source.validate().is_err());
    }

    #[test]
    fn inspection_reports_a_dng_as_decodable_and_names_the_decoder() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_dng(directory.path(), "shot.dng", 900);
        let result = inspect_raw(path.to_string_lossy().into_owned()).unwrap();
        assert_eq!(result.inspection.support, crate::raw::RawSupport::Decodable);
        assert_eq!(result.inspection.decoder.as_deref(), Some(DECODER_ID));
        assert!(result.capabilities.backend_available);
        assert!(!result.capabilities.native_dependencies);
        assert_eq!(
            result.capabilities.formats,
            vec![crate::raw::RawFormat::Dng]
        );
    }

    /// A format this build recognises but cannot decode must say exactly that,
    /// rather than being reported as supported.
    #[test]
    fn inspection_does_not_claim_a_format_it_cannot_decode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shot.cr2");
        std::fs::write(&path, [b'I', b'I', 42, 0, 8, 0, 0, 0, 0, 0]).unwrap();
        let result = inspect_raw(path.to_string_lossy().into_owned()).unwrap();
        assert_eq!(
            result.inspection.support,
            crate::raw::RawSupport::RecognizedDecoderUnavailable
        );
        assert_eq!(result.inspection.decoder, None);
        assert!(!result
            .capabilities
            .formats
            .contains(&crate::raw::RawFormat::Cr2));
    }
}
