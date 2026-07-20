use crate::domain::{BatchStatus, ImageQualityAnalysis, OllamaDiagnostics};
use crate::infrastructure::LoadedImage;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

pub struct EditorSession {
    pub source: LoadedImage,
    pub document_id: u64,
    pub analysis: Option<ImageQualityAnalysis>,
}

pub struct AppState {
    pub session: Mutex<Option<EditorSession>>,
    pub components: Mutex<ComponentRegistry>,
    pub ollama_diagnostics: Mutex<OllamaDiagnostics>,
    pub latest_open_request: AtomicU64,
    pub pending_open_request: AtomicU64,
    pub latest_preview_request: AtomicU64,
    pub latest_analysis_request: AtomicU64,
    pub latest_plan_request: AtomicU64,
    pub latest_histogram_request: AtomicU64,
    pub preview_gate: tokio::sync::Mutex<()>,
    pub analysis_gate: tokio::sync::Mutex<()>,
    pub plan_gate: tokio::sync::Mutex<()>,
    pub export_gate: tokio::sync::Mutex<()>,
    pub histogram_gate: tokio::sync::Mutex<()>,
    pub batch_gate: tokio::sync::Mutex<()>,
    pub batch_status: Arc<Mutex<BatchStatus>>,
    pub batch_cancelled: Arc<AtomicBool>,
}

impl Default for AppState {
    fn default() -> Self {
        let mut components = ComponentRegistry::default();
        components.load_persisted_configuration();
        Self {
            session: Mutex::new(None),
            components: Mutex::new(components),
            ollama_diagnostics: Mutex::new(OllamaDiagnostics::default()),
            latest_open_request: AtomicU64::new(0),
            pending_open_request: AtomicU64::new(0),
            latest_preview_request: AtomicU64::new(0),
            latest_analysis_request: AtomicU64::new(0),
            latest_plan_request: AtomicU64::new(0),
            latest_histogram_request: AtomicU64::new(0),
            preview_gate: tokio::sync::Mutex::new(()),
            analysis_gate: tokio::sync::Mutex::new(()),
            plan_gate: tokio::sync::Mutex::new(()),
            export_gate: tokio::sync::Mutex::new(()),
            histogram_gate: tokio::sync::Mutex::new(()),
            batch_gate: tokio::sync::Mutex::new(()),
            batch_status: Arc::new(Mutex::new(BatchStatus::default())),
            batch_cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}
use crate::components::ComponentRegistry;
