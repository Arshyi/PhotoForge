use crate::domain::{
    BatchFailureRecord, BatchOptions, BatchPreview, BatchState, BatchStatus, ExportProfile,
    Workflow, MAX_BATCH_FILES,
};
use crate::error::AppError;
use crate::image_processing::high_precision::{check_cancel, pipeline_typed};
use crate::infrastructure::{load_image, save_image_with_profile};
use crate::pixel::PixelBuffer;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub fn preview_batch(
    options: &BatchOptions,
    workflow: &Workflow,
) -> Result<BatchPreview, AppError> {
    options.validate()?;
    validate_batch_workflow(workflow)?;
    let input = canonical_folder(&options.input_folder)?;
    let output = canonical_folder(&options.output_folder)?;
    validate_folders(&input, &output)?;
    let files = discover_images(&input, options.recursive)?;
    let mut sample_outputs = Vec::new();
    let mut skipped_existing = 0;
    for (index, path) in files.iter().enumerate() {
        let target = output_path(
            path,
            &output,
            index,
            workflow,
            &options.filename_template,
            options.export_profile,
        )?;
        skipped_existing += usize::from(target.exists() && !options.overwrite);
        if sample_outputs.len() < 12 {
            sample_outputs.push(target.to_string_lossy().into_owned());
        }
    }
    Ok(BatchPreview {
        discovered: files.len(),
        sample_outputs,
        estimated_time_ms: files.len() as u64 * 250,
        skipped_existing,
    })
}

pub fn run_batch(
    batch_id: u64,
    options: BatchOptions,
    workflow: Workflow,
    status: Arc<Mutex<BatchStatus>>,
    cancelled: Arc<AtomicBool>,
) -> Result<BatchStatus, AppError> {
    options.validate()?;
    validate_batch_workflow(&workflow)?;
    let started = Instant::now();
    set_status(&status, |value| {
        *value = BatchStatus {
            batch_id,
            state: BatchState::Discovering,
            ..Default::default()
        };
    });
    let input = canonical_folder(&options.input_folder)?;
    let output = canonical_folder(&options.output_folder)?;
    validate_folders(&input, &output)?;
    let files = Arc::new(discover_images(&input, options.recursive)?);
    set_status(&status, |value| {
        value.state = BatchState::Running;
        value.discovered = files.len();
    });

    let next = Arc::new(AtomicUsize::new(0));
    let claims = Arc::new(Mutex::new(HashSet::<PathBuf>::new()));
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    // One decoded image at a time: a float source can approach 1 GiB. The
    // requested worker count remains an upper bound, never a memory promise.
    let worker_count = 1;
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let files = Arc::clone(&files);
            let next = Arc::clone(&next);
            let claims = Arc::clone(&claims);
            let log = Arc::clone(&log);
            let status = Arc::clone(&status);
            let cancelled = Arc::clone(&cancelled);
            let output = output.clone();
            let options = &options;
            let workflow = &workflow;
            scope.spawn(move || loop {
                if cancelled.load(Ordering::Acquire) {
                    break;
                }
                let index = next.fetch_add(1, Ordering::AcqRel);
                let Some(path) = files.get(index) else {
                    break;
                };
                set_status(&status, |value| {
                    value.current_file = Some(path.to_string_lossy().into_owned());
                });
                let target = match output_path(
                    path,
                    &output,
                    index,
                    workflow,
                    &options.filename_template,
                    options.export_profile,
                ) {
                    Ok(target) => target,
                    Err(error) => {
                        record_failure(&status, &log, path, &error);
                        continue;
                    }
                };
                let claimed = claims
                    .lock()
                    .map(|mut values| values.insert(target.clone()))
                    .unwrap_or(false);
                if !claimed || (target.exists() && !options.overwrite) {
                    set_status(&status, |value| value.skipped += 1);
                    push_log(&log, format!("SKIPPED\t{}", path.display()));
                    continue;
                }
                if options.dry_run {
                    set_status(&status, |value| value.completed += 1);
                    push_log(
                        &log,
                        format!("PREVIEW\t{}\t{}", path.display(), target.display()),
                    );
                    update_estimate(&status, started);
                    continue;
                }
                let result = (|| {
                    let _job = crate::resources::acquire_job(Some(&cancelled))?;
                    check_cancel(Some(&cancelled))?;
                    let (source, source_path) = if is_project(path) {
                        load_project_composite_typed(path)?
                    } else if path
                        .extension()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| s.eq_ignore_ascii_case("dng"))
                    {
                        // Canonical Windows discovery uses a verbatim disk prefix;
                        // the public RAW loader deliberately rejects device paths.
                        let local_path = PathBuf::from(crate::raw::presentable_path(path));
                        let bytes = crate::raw::read_source_bytes(&local_path)
                            .map_err(|e| AppError::RawInspection(e.to_string()))?;
                        check_cancel(Some(&cancelled))?;
                        let developed = crate::raw::develop::develop_bytes(
                            &bytes,
                            &crate::color::DevelopmentParameters::default(),
                            crate::raw::develop::RenderScale::Full,
                        )
                        .map_err(|e| AppError::RawInspection(e.to_string()))?;
                        (developed.image.into(), path.clone())
                    } else {
                        let loaded = load_image(path)?;
                        let pixels = match loaded.working {
                            Some(image) => PixelBuffer::LinearRgbaF32(image),
                            None => loaded.original.to_rgba8().into(),
                        };
                        (pixels, loaded.path)
                    };
                    check_cancel(Some(&cancelled))?;
                    // A plugin filter is a function of linear float pixels, and the legacy
                    // 8-bit renderer refuses to run one rather than quantise on every call.
                    // An 8-bit source is promoted here, once, deliberately, as "Convert to
                    // linear float" does for a document, so a batch can use a filter on a
                    // folder of JPEGs. Nothing else is promoted: a workflow without a plugin
                    // filter keeps the precision it always had.
                    let source = if matches!(source, PixelBuffer::EncodedSrgba8(_))
                        && workflow.operations.iter().any(|operation| {
                            matches!(operation, crate::domain::EditOperation::PluginFilter { .. })
                        }) {
                        PixelBuffer::LinearRgbaF32(source.linear()?)
                    } else {
                        source
                    };
                    let processed = pipeline_typed(source, &workflow.operations, Some(&cancelled))?;
                    if options.color.is_some() || matches!(processed, PixelBuffer::LinearRgbaF32(_))
                    {
                        let color =
                            options
                                .color
                                .unwrap_or(crate::color_management::ColorExportOptions {
                                    bit_depth: if matches!(
                                        options.export_profile,
                                        ExportProfile::Archive | ExportProfile::Lossless
                                    ) {
                                        16
                                    } else {
                                        8
                                    },
                                    ..Default::default()
                                });
                        crate::infrastructure::save_color_image(
                            processed.linear()?.as_ref(),
                            &source_path,
                            &target,
                            options.export_profile,
                            color,
                            Some(&cancelled),
                        )?;
                    } else {
                        check_cancel(Some(&cancelled))?;
                        save_image_with_profile(
                            &image::DynamicImage::ImageRgba8((*processed.encoded8()).clone()),
                            &source_path,
                            &target,
                            options.export_profile,
                        )?;
                    }
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        set_status(&status, |value| value.completed += 1);
                        push_log(
                            &log,
                            format!("OK\t{}\t{}", path.display(), target.display()),
                        );
                    }
                    Err(AppError::RenderCancelled) => break,
                    Err(error) => record_failure(&status, &log, path, &error),
                }
                update_estimate(&status, started);
            });
        }
    });

    let log_path = output.join(format!("photoforge-batch-{batch_id}.log"));
    let log_text = log
        .lock()
        .map(|lines| lines.join("\n"))
        .unwrap_or_else(|_| "PhotoForge batch log unavailable".into());
    if !options.dry_run {
        fs::write(&log_path, log_text).map_err(|error| {
            AppError::BatchFailure(format!("could not write batch log: {error}"))
        })?;
    }
    set_status(&status, |value| {
        value.state = if cancelled.load(Ordering::Acquire) {
            BatchState::Cancelled
        } else {
            BatchState::Completed
        };
        value.current_file = None;
        value.estimated_remaining_ms = Some(0);
        value.elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        value.log_path = (!options.dry_run).then(|| log_path.to_string_lossy().into_owned());
    });
    status
        .lock()
        .map(|value| value.clone())
        .map_err(|_| AppError::BatchFailure("batch status is unavailable".into()))
}

/// Batch currently applies a workflow's document-level image operations to
/// each rendered input. Layer steps need a live layer document, selection, and
/// ordered pixel-store mutations, which this worker does not provide yet.
/// Reject them at the boundary instead of accepting the workflow and silently
/// exporting an image that omitted part (or all) of the requested work.
fn validate_batch_workflow(workflow: &Workflow) -> Result<(), AppError> {
    workflow.validate()?;
    if !workflow.layer_steps.is_empty() {
        return Err(AppError::BatchFailure(
            "layer-aware workflow steps are not supported by batch processing".into(),
        ));
    }
    Ok(())
}

pub fn discover_images(folder: &Path, recursive: bool) -> Result<Vec<PathBuf>, AppError> {
    let mut files = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(current) = folders.pop() {
        let entries = fs::read_dir(&current).map_err(|error| {
            AppError::BatchFailure(format!("cannot read {}: {error}", current.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| AppError::BatchFailure(error.to_string()))?;
            let file_type = entry
                .file_type()
                .map_err(|error| AppError::BatchFailure(error.to_string()))?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() && recursive {
                folders.push(entry.path());
            } else if file_type.is_file() && supported_image(&entry.path()) {
                files.push(entry.path());
                if files.len() > MAX_BATCH_FILES {
                    return Err(AppError::BatchFailure(format!(
                        "batch exceeds the {MAX_BATCH_FILES} file safety limit"
                    )));
                }
            }
        }
    }
    files.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    Ok(files)
}

fn canonical_folder(path: &str) -> Result<PathBuf, AppError> {
    let path = fs::canonicalize(path)
        .map_err(|error| AppError::BatchFailure(format!("folder is unavailable: {error}")))?;
    if !path.is_dir() {
        return Err(AppError::BatchFailure("batch path is not a folder".into()));
    }
    Ok(path)
}

fn supported_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "dng")
    ) || is_project(path)
}

/// A saved PhotoForge project is accepted as a batch input. Batch renders its
/// visible composite and exports that; it never edits the layer tree, and the
/// project file itself is only ever read.
fn is_project(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
        == Some(crate::layers::PROJECT_EXTENSION)
}

/// Renders a project's composite so the ordinary batch export path can write it.
fn load_project_composite_typed(path: &Path) -> Result<(PixelBuffer, PathBuf), AppError> {
    let project = crate::layers::load_project(path)?;
    let mut store = crate::layers::LayerPixelStore::default();
    store.reset(
        project.document.canvas_width,
        project.document.canvas_height,
    )?;
    for (pixel_id, image) in project.pixels {
        store.register_with_id(&pixel_id, image)?;
    }
    for (pixel_id, image) in project.linear_pixels {
        store.register_typed_with_id(&pixel_id, image.into())?;
    }
    let resolved = store.resolve(&project.document.referenced_pixel_ids(), false)?;
    let composite = crate::layers::render_document_typed(
        &project.document,
        &resolved,
        crate::layers::RenderOptions::default(),
    )?;
    // The project's own document pipeline runs first, then the workflow's.
    let rendered = pipeline_typed(composite, &project.document_operations, None)?;
    Ok((rendered, path.to_path_buf()))
}

fn output_path(
    input: &Path,
    output_folder: &Path,
    index: usize,
    workflow: &Workflow,
    template: &str,
    profile: ExportProfile,
) -> Result<PathBuf, AppError> {
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = profile_extension(profile);
    let name = template
        .replace("{name}", stem)
        .replace("{index}", &format!("{:04}", index + 1))
        .replace("{ext}", extension)
        .replace("{workflow}", &workflow.name);
    let name = sanitize_filename(&name);
    if name.is_empty() {
        return Err(AppError::BatchFailure(
            "filename template produced an empty name".into(),
        ));
    }
    let mut target = output_folder.join(name);
    target.set_extension(extension);
    Ok(target)
}

fn validate_folders(input: &Path, output: &Path) -> Result<(), AppError> {
    // Keep previously exported files out of recursive input discovery. This also
    // prevents overwrite=true from destroying another member of the input set.
    if output.ancestors().any(|parent| paths_equal(input, parent)) {
        return Err(AppError::BatchFailure(
            "the output folder must be outside the input folder".into(),
        ));
    }
    Ok(())
}

fn profile_extension(profile: ExportProfile) -> &'static str {
    match profile {
        ExportProfile::Web | ExportProfile::Print | ExportProfile::HighJpeg => "jpg",
        ExportProfile::MaximumCompression => "webp",
        ExportProfile::Archive | ExportProfile::Lossless => "png",
    }
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_' | '.' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches([' ', '.'])
        .chars()
        .take(180)
        .collect()
}

fn record_failure(
    status: &Arc<Mutex<BatchStatus>>,
    log: &Arc<Mutex<Vec<String>>>,
    path: &Path,
    error: &AppError,
) {
    set_status(status, |value| {
        value.failed += 1;
        if value.failures.len() < 100 {
            value.failures.push(BatchFailureRecord {
                input_path: path.to_string_lossy().into_owned(),
                error: error.to_string(),
            });
        }
    });
    push_log(log, format!("FAILED\t{}\t{}", path.display(), error));
}

fn update_estimate(status: &Arc<Mutex<BatchStatus>>, started: Instant) {
    set_status(status, |value| {
        value.elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let processed = value.completed + value.failed + value.skipped;
        if processed > 0 {
            let remaining = value.discovered.saturating_sub(processed);
            value.estimated_remaining_ms =
                Some(value.elapsed_ms / processed as u64 * remaining as u64);
        }
    });
}

fn set_status(status: &Arc<Mutex<BatchStatus>>, update: impl FnOnce(&mut BatchStatus)) {
    if let Ok(mut value) = status.lock() {
        update(&mut value);
    }
}

fn push_log(log: &Arc<Mutex<Vec<String>>>, line: String) {
    if let Ok(mut lines) = log.lock() {
        lines.push(line);
    }
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::EditOperation;
    use image::{Rgba, RgbaImage};

    fn workflow() -> Workflow {
        Workflow {
            id: "batch".into(),
            name: "Batch Enhance".into(),
            description: String::new(),
            folder: String::new(),
            favorite: false,
            operations: vec![EditOperation::Brightness { amount: 0.1 }],
            layer_steps: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn options(input: &Path, output: &Path) -> BatchOptions {
        BatchOptions {
            input_folder: input.to_string_lossy().into_owned(),
            output_folder: output.to_string_lossy().into_owned(),
            filename_template: "{name}-{index}".into(),
            recursive: false,
            overwrite: false,
            workers: 2,
            export_profile: ExportProfile::Lossless,
            color: None,
            dry_run: false,
        }
    }

    #[test]
    fn discovery_filters_and_sorts_supported_images() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["z.webp", "a.PNG", "b.jpg", "notes.txt"] {
            fs::write(directory.path().join(name), []).unwrap();
        }
        let files = discover_images(directory.path(), false).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].file_name().unwrap(), "a.PNG");
    }

    #[test]
    fn discovery_honors_recursive_option() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("image.png"), []).unwrap();
        assert!(discover_images(directory.path(), false).unwrap().is_empty());
        assert_eq!(discover_images(directory.path(), true).unwrap().len(), 1);
    }

    #[test]
    fn filename_templates_expand_and_sanitize() {
        let target = output_path(
            Path::new("my photo.png"),
            Path::new("output"),
            6,
            &workflow(),
            "{workflow}-{name}-{index}-{ext}",
            ExportProfile::Web,
        )
        .unwrap();
        assert_eq!(
            target,
            Path::new("output").join("Batch Enhance-my photo-0007-jpg.jpg")
        );
    }

    #[test]
    fn project_files_are_accepted_as_batch_inputs() {
        assert!(supported_image(Path::new("a.photoforge")));
        assert!(is_project(Path::new("a.PHOTOFORGE")));
        assert!(!is_project(Path::new("a.png")));
        assert!(!supported_image(Path::new("notes.txt")));
        assert!(!supported_image(Path::new("a.photoforge-recovery")));
    }

    #[test]
    fn a_batch_exports_a_project_composite_without_modifying_the_project() {
        use crate::layers::{LayerDocument, LayerPixelStore};
        use image::{Rgba, RgbaImage};

        let directory = tempfile::tempdir().unwrap();
        let project_path = directory.path().join("doc.photoforge");

        // Two stacked layers so the export proves compositing actually ran.
        let mut store = LayerPixelStore::default();
        store.reset(4, 4).unwrap();
        let bottom = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 255, 255]));
        let mut top = RgbaImage::from_pixel(4, 4, Rgba([255, 0, 0, 255]));
        top.put_pixel(0, 0, Rgba([0, 0, 0, 0]));

        let mut document = LayerDocument::new(4, 4);
        document.layers = vec![
            crate::layers::test_pixel_layer("bottom", "pxb", 4, 4),
            crate::layers::test_pixel_layer("top", "pxt", 4, 4),
        ];
        crate::layers::save_project(
            &project_path,
            &document,
            &[],
            &[("pxb".to_string(), &bottom), ("pxt".to_string(), &top)],
            "0.8.0",
            "",
            "",
        )
        .unwrap();
        let original = fs::read(&project_path).unwrap();

        let (composite, source) = load_project_composite_typed(&project_path).unwrap();
        let rgba = composite.encoded8();
        // The top layer covers the bottom except where it is transparent.
        assert_eq!(rgba.get_pixel(1, 1).0, [255, 0, 0, 255]);
        assert_eq!(rgba.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(source, project_path);
        // Reading a project for batch never writes to it.
        assert_eq!(fs::read(&project_path).unwrap(), original);
    }

    #[test]
    fn a_corrupt_project_fails_that_batch_item_rather_than_the_run() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("broken.photoforge");
        fs::write(&path, b"not a project").unwrap();
        assert!(load_project_composite_typed(&path).is_err());
    }

    #[test]
    fn profiles_select_expected_extensions() {
        for (profile, extension) in [
            (ExportProfile::Web, "jpg"),
            (ExportProfile::Print, "jpg"),
            (ExportProfile::Archive, "png"),
            (ExportProfile::Lossless, "png"),
            (ExportProfile::HighJpeg, "jpg"),
            (ExportProfile::MaximumCompression, "webp"),
        ] {
            assert_eq!(profile_extension(profile), extension);
        }
    }

    #[test]
    fn batch_dng_is_full_precision_tagged_deterministic_and_source_safe() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("in");
        let output = root.path().join("out");
        fs::create_dir(&input).unwrap();
        fs::create_dir(&output).unwrap();
        let bytes = crate::raw::dng::fixtures::DngBuilder::new(4, 4, vec![1001; 16])
            .levels(vec![0], (1, 1), 4095)
            .build();
        fs::write(input.join("a.dng"), &bytes).unwrap();
        fs::write(input.join("b.dng"), &bytes).unwrap();
        fs::write(input.join("bad.dng"), b"broken").unwrap();
        let mut options = options(&input, &output);
        options.color = Some(crate::color_management::ColorExportOptions {
            color_space: crate::color_management::RgbColorSpace::DisplayP3,
            ..Default::default()
        });
        let result = run_batch(
            991,
            options,
            workflow(),
            Arc::new(Mutex::new(BatchStatus::default())),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!((result.completed, result.failed), (2, 1), "{result:?}");
        let a = fs::read(output.join("a-0001.png")).unwrap();
        let b = fs::read(output.join("b-0002.png")).unwrap();
        assert_eq!(a, b);
        let image = image::load_from_memory(&a).unwrap();
        assert_eq!(image.color(), image::ColorType::Rgba16);
        assert!(image.to_rgba16().pixels().any(|p| p[0] % 257 != 0));
        assert_eq!(fs::read(input.join("a.dng")).unwrap(), bytes);
    }

    #[test]
    fn nested_output_is_rejected_even_when_overwrite_is_enabled() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("output");
        fs::create_dir(&output).unwrap();
        let mut options = options(root.path(), &output);
        options.overwrite = true;
        options.recursive = true;
        assert!(preview_batch(&options, &workflow()).is_err());
    }

    #[test]
    fn batch_preview_lists_outputs_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input");
        let output = directory.path().join("output");
        fs::create_dir(&input).unwrap();
        fs::create_dir(&output).unwrap();
        RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 255]))
            .save(input.join("one.png"))
            .unwrap();
        let preview = preview_batch(&options(&input, &output), &workflow()).unwrap();
        assert_eq!(preview.discovered, 1);
        assert_eq!(preview.sample_outputs.len(), 1);
        assert!(fs::read_dir(output).unwrap().next().is_none());
    }

    #[test]
    fn batch_preview_rejects_layer_steps_instead_of_silently_ignoring_them() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input");
        let output = directory.path().join("output");
        fs::create_dir(&input).unwrap();
        fs::create_dir(&output).unwrap();
        let mut workflow = workflow();
        workflow.layer_steps = vec![crate::layers::LayerWorkflowStep::SetVisibility {
            selector: crate::layers::LayerSelector::Active,
            visible: false,
        }];

        let error = preview_batch(&options(&input, &output), &workflow).unwrap_err();
        assert!(matches!(error, AppError::BatchFailure(_)));
        assert!(error.to_string().contains("layer-aware workflow steps"));
    }

    #[test]
    fn batch_run_rejects_layer_steps_before_writing_outputs_or_a_log() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input");
        let output = directory.path().join("output");
        fs::create_dir(&input).unwrap();
        fs::create_dir(&output).unwrap();
        RgbaImage::new(2, 2).save(input.join("one.png")).unwrap();
        let mut workflow = workflow();
        workflow.layer_steps = vec![crate::layers::LayerWorkflowStep::Flatten];

        let error = run_batch(
            9,
            options(&input, &output),
            workflow,
            Arc::new(Mutex::new(BatchStatus::default())),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
        assert!(matches!(error, AppError::BatchFailure(_)));
        assert!(fs::read_dir(output).unwrap().next().is_none());
    }

    #[test]
    fn dry_run_updates_progress_without_outputs() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input");
        let output = directory.path().join("output");
        fs::create_dir(&input).unwrap();
        fs::create_dir(&output).unwrap();
        RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 255]))
            .save(input.join("one.png"))
            .unwrap();
        let mut options = options(&input, &output);
        options.dry_run = true;
        let status = run_batch(
            4,
            options,
            workflow(),
            Arc::new(Mutex::new(BatchStatus::default())),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(status.state, BatchState::Completed);
        assert_eq!(status.completed, 1);
        assert!(fs::read_dir(output).unwrap().next().is_none());
    }

    #[test]
    fn batch_exports_images_and_log() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input");
        let output = directory.path().join("output");
        fs::create_dir(&input).unwrap();
        fs::create_dir(&output).unwrap();
        RgbaImage::from_pixel(3, 4, Rgba([5, 6, 7, 255]))
            .save(input.join("one.png"))
            .unwrap();
        let status = run_batch(
            5,
            options(&input, &output),
            workflow(),
            Arc::new(Mutex::new(BatchStatus::default())),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(status.completed, 1);
        assert_eq!(
            image::image_dimensions(output.join("one-0001.png")).unwrap(),
            (3, 4)
        );
        assert!(Path::new(status.log_path.as_ref().unwrap()).is_file());
    }

    #[test]
    fn batch_skips_existing_outputs() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input");
        let output = directory.path().join("output");
        fs::create_dir(&input).unwrap();
        fs::create_dir(&output).unwrap();
        RgbaImage::new(2, 2).save(input.join("one.png")).unwrap();
        fs::write(output.join("one-0001.png"), b"existing").unwrap();
        let status = run_batch(
            6,
            options(&input, &output),
            workflow(),
            Arc::new(Mutex::new(BatchStatus::default())),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(status.skipped, 1);
        assert_eq!(fs::read(output.join("one-0001.png")).unwrap(), b"existing");
    }

    #[test]
    fn pre_cancelled_batch_stops_before_processing() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input");
        let output = directory.path().join("output");
        fs::create_dir(&input).unwrap();
        fs::create_dir(&output).unwrap();
        RgbaImage::new(2, 2).save(input.join("one.png")).unwrap();
        let cancelled = Arc::new(AtomicBool::new(true));
        let status = run_batch(
            7,
            options(&input, &output),
            workflow(),
            Arc::new(Mutex::new(BatchStatus::default())),
            cancelled,
        )
        .unwrap();
        assert_eq!(status.state, BatchState::Cancelled);
        assert_eq!(status.completed, 0);
    }

    #[test]
    fn same_input_and_output_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let result = run_batch(
            8,
            options(directory.path(), directory.path()),
            workflow(),
            Arc::new(Mutex::new(BatchStatus::default())),
            Arc::new(AtomicBool::new(false)),
        );
        assert!(matches!(result, Err(AppError::BatchFailure(_))));
    }
}
