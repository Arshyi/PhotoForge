//! The one path by which a document is edited by anything other than a person's
//! direct manipulation.
//!
//! A plugin, a recorded macro, a layer workflow, an AI planner, a batch run and a
//! future command-line tool all need to change a layered document. If each had its
//! own way of doing it there would be five editing engines to keep consistent, and
//! they would not be: one would forget to validate, another would leave a buffer
//! behind when it failed halfway. So there is one.
//!
//! ```text
//!   typed operation  ->  validation  ->  resource admission  ->  transaction
//!        ->  document invariants  ->  one Undo entry  ->  renderer / cache
//! ```
//!
//! * [`registry`] names every operation, versions it, says what it needs and what
//!   it may touch, and turns a `{ op, params }` call into a typed value or refuses
//!   it. An id that is not in the registry is not an operation.
//! * [`engine`] runs a list of calls as **one transaction**: on a private copy of
//!   the document, validated after every step, with a journal of every pixel
//!   buffer the steps created. If anything fails, the copy is dropped and the
//!   journal is replayed in reverse; the caller's document and the pixel store are
//!   as they were. If everything succeeds the caller gets one new document, which
//!   becomes exactly one entry in the undo history.
//! * [`structure`] is the tree editing itself: insert, move, group, duplicate.
//! * [`support`] holds the sources of identifiers and time, so a replay can be made
//!   deterministic, and the revision a plan is checked against.
//!
//! # What this does not do
//!
//! It does not touch the screen. It does not hold the document: the document is a
//! value passed in and a value passed out, which is what makes undo a pointer swap
//! and a failed transaction a no-op. And it is not yet the only code that edits a
//! document: the Layers panel still edits through the TypeScript tree functions
//! for the rapid-fire cases (a dragged opacity slider, a rename), and the two are
//! held to the same behaviour by the vectors in
//! `src-tauri/tests/fixtures/operations_parity.json`. `docs/automation.md` lists
//! which interface actions go through here.
pub mod engine;
pub mod model;
pub mod registry;
pub mod structure;
pub mod support;

pub use engine::{execute, execute_with};
pub use model::{
    OperationCall, Origin, StepReport, TransactionRequest, TransactionResult, MAX_TRANSACTION_STEPS,
};
pub use registry::{operation_specs, OperationKind, OperationSpec};
pub use support::{document_revision, Clock, IdSource, RandomIds, SequenceIds, SystemClock};
