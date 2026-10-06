//! Image sources: reading a bounded part of a file that may be too large to open
//! whole, and remembering which part.
//!
//! * [`model`] is the provenance a layer carries: which file, which bytes of it,
//!   which part.
//! * [`pixels`] and [`reduce`] are the shared machinery: decoded rows in the
//!   layouts a full decode would give, and an exact area resampler that works a
//!   row at a time.
//! * [`jpeg`] decodes a region from the whole frame, and a reduced copy at a DCT
//!   scale that never produces the whole frame.
//! * [`webp`] decodes the whole frame, as the format's own decoder can do no
//!   other, and says so.
//! * [`png`] is the one format that supports true row-streaming region decoding.
//!
//! The decoders live beside it and each states, in its own header, exactly what
//! it allocates — a decoder that decodes the whole frame and crops is called
//! that, never "region decoding".
pub mod jpeg;
pub mod model;
pub mod open;
pub mod pixels;
pub mod png;
pub mod probe;
pub mod reduce;
pub mod webp;

pub use model::{Rect, SourceIdentity, SourceOrigin, SourceView};
