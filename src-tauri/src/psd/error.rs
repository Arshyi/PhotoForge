//! What can go wrong reading a Photoshop file, in terms a person can act on.
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PsdError {
    /// The file ended before something it declared.
    Truncated { what: &'static str },
    /// The first four bytes are not `8BPS`.
    NotPhotoshop,
    /// A version other than 1 (PSD) or 2 (PSB).
    UnsupportedVersion(u16),
    /// A valid file that uses something this reader does not read.
    Unsupported { what: String },
    /// A declared size is beyond what PhotoForge will allocate for it. Raised before anything
    /// is allocated or read on the strength of the declaration.
    Limit {
        what: &'static str,
        value: u64,
        limit: u64,
    },
    /// The structure contradicts itself: a length that does not fit, a group that never closes.
    Malformed { what: &'static str },
    /// The operating system refused a read.
    Io(String),
}

impl fmt::Display for PsdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { what } => {
                write!(f, "The Photoshop file ends early (while reading {what}).")
            }
            Self::NotPhotoshop => write!(f, "This is not a Photoshop (PSD or PSB) file."),
            Self::UnsupportedVersion(version) => {
                write!(f, "This Photoshop file has an unknown version ({version}).")
            }
            Self::Unsupported { what } => {
                write!(
                    f,
                    "This Photoshop file uses something PhotoForge does not read: {what}."
                )
            }
            Self::Limit { what, value, limit } => write!(
                f,
                "This Photoshop file declares {what} of {value}, beyond the limit of {limit}."
            ),
            Self::Malformed { what } => {
                write!(
                    f,
                    "This Photoshop file is damaged or inconsistent ({what})."
                )
            }
            Self::Io(message) => write!(f, "The Photoshop file could not be read: {message}"),
        }
    }
}

impl std::error::Error for PsdError {}

impl From<std::io::Error> for PsdError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            Self::Truncated { what: "the file" }
        } else {
            Self::Io(error.to_string())
        }
    }
}

impl From<PsdError> for crate::error::AppError {
    fn from(error: PsdError) -> Self {
        Self::PsdFile(error.to_string())
    }
}
