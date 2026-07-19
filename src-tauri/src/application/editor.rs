use crate::domain::ImageQualityAnalysis;
use crate::infrastructure::LoadedImage;
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;

pub struct EditorSession {
    pub source: LoadedImage,
    pub document_id: u64,
    pub analysis: Option<ImageQualityAnalysis>,
}

pub struct AppState {
    pub session: Mutex<Option<EditorSession>>,
    pub components: Mutex<ComponentRegistry>,
    pub latest_open_request: AtomicU64,
    pub pending_open_request: AtomicU64,
    pub latest_preview_request: AtomicU64,
    pub latest_analysis_request: AtomicU64,
    pub latest_plan_request: AtomicU64,
    pub preview_gate: tokio::sync::Mutex<()>,
    pub analysis_gate: tokio::sync::Mutex<()>,
    pub plan_gate: tokio::sync::Mutex<()>,
    pub export_gate: tokio::sync::Mutex<()>,
}

impl Default for AppState {
    fn default() -> Self {
        let mut components = ComponentRegistry::default();
        components.load_persisted_configuration();
        Self {
            session: Mutex::new(None),
            components: Mutex::new(components),
            latest_open_request: AtomicU64::new(0),
            pending_open_request: AtomicU64::new(0),
            latest_preview_request: AtomicU64::new(0),
            latest_analysis_request: AtomicU64::new(0),
            latest_plan_request: AtomicU64::new(0),
            preview_gate: tokio::sync::Mutex::new(()),
            analysis_gate: tokio::sync::Mutex::new(()),
            plan_gate: tokio::sync::Mutex::new(()),
            export_gate: tokio::sync::Mutex::new(()),
        }
    }
}
use crate::components::ComponentRegistry;
