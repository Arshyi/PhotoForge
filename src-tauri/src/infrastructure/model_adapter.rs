use crate::domain::EditOperation;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageContext {
    pub width: u32,
    pub height: u32,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditPlan {
    pub explanation: String,
    pub operations: Vec<EditOperation>,
}

#[derive(Debug, thiserror::Error)]
pub enum EditPlanError {
    #[error("The requested edit cannot be expressed with supported operations.")]
    UnsupportedRequest,
    #[error("The proposed edit plan is invalid.")]
    InvalidPlan,
}

pub trait EditPlanProvider: Send + Sync {
    fn create_plan(
        &self,
        request: &str,
        image_context: &ImageContext,
    ) -> Result<EditPlan, EditPlanError>;
}
