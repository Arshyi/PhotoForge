//! The wire types of a transaction: what the caller sends and what comes back.
use crate::layers::LayerDocument;
use crate::mask::MaskSnapshot;
use serde::{Deserialize, Serialize};

/// The most steps one transaction may carry. The same ceiling the layer workflow
/// has always had, so a workflow can become one transaction without being cut.
pub const MAX_TRANSACTION_STEPS: usize = 100;

pub const MAX_LABEL_CHARS: usize = 120;

/// One call: the operation's registered id, and its parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationCall {
    pub op: String,
    #[serde(default)]
    pub params: serde_json::Value,
    /// The step is skipped, in the dry run and the real one alike, unless this holds
    /// for the document as it is when the step is reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<super::condition::Condition>,
}

/// Who is asking.
///
/// It decides what is allowed, not how it is done: every origin runs the same
/// operation through the same engine. A person may act on a layer they locked; an
/// automation may not, because a lock is exactly the thing that protects a layer
/// from an unattended replay. A planner may use only the relative selectors and
/// the operations that are suggestions rather than decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    #[default]
    User,
    Automation,
    Planner,
    Plugin,
    Batch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransactionRequest {
    pub document: LayerDocument,
    /// The name of the one undo entry a successful transaction becomes.
    pub label: String,
    pub steps: Vec<OperationCall>,
    /// The revision of the document this was planned against. If it is given and
    /// is not the revision of `document`, nothing runs.
    #[serde(default)]
    pub expected_revision: Option<String>,
    /// The canvas selection, for the steps that need one.
    #[serde(default)]
    pub selection: Option<MaskSnapshot>,
    #[serde(default)]
    pub origin: Origin,
    /// The plugin on whose behalf a `plugin` transaction runs. It is what stops a
    /// plugin's command from spending the authority of another plugin: a plugin
    /// transaction may apply that plugin's filters and no one else's.
    #[serde(default)]
    pub plugin: Option<String>,
}

/// What one step did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepReport {
    pub index: usize,
    pub op: String,
    /// The step's condition did not hold, so it was not run.
    pub skipped: bool,
    /// Layers this step created.
    pub created_layers: Vec<String>,
    /// Pixel buffers this step registered.
    pub created_pixels: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionResult {
    /// The document after every step. Commit this as one history entry.
    pub document: LayerDocument,
    /// Its revision, to name as `expected_revision` in a follow-up plan.
    pub revision: String,
    pub label: String,
    pub steps: Vec<StepReport>,
    /// Every pixel buffer the transaction registered: the ones the new document
    /// may refer to, and which the caller keeps alive for as long as history does.
    pub created_pixel_ids: Vec<String>,
    /// The last layer a step created, if any.
    pub last_created: Option<String>,
}
