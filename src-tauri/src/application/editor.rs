use crate::infrastructure::LoadedImage;
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;

pub struct EditorSession {
    pub source: LoadedImage,
}

#[derive(Default)]
pub struct AppState {
    pub session: Mutex<Option<EditorSession>>,
    pub latest_request: AtomicU64,
}
