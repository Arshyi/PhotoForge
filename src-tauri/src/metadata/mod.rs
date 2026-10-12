//! Metadata a photograph carries (EXIF, XMP, IPTC), and what an export does with it.
//!
//! Until 15.3 an exported file carried none of it: the encoders were handed pixels and an ICC
//! profile. Dropping everything is safe for privacy and destructive for a photographer's
//! archive, and doing it without saying so is the worse half of that. So: the three blocks are
//! read when a file is opened, kept with the document, and written back on export under a
//! policy the person chooses, and the export says what it kept, what it removed and what it could
//! not store.
//!
//! * [`MetadataPolicy::KeepAll`] writes what the source had, corrected for the edit: the
//!   orientation is upright because the pixels are, the picture's size is the new size, and the
//!   embedded thumbnail is gone, because it is a small copy of the picture as it was.
//! * [`MetadataPolicy::RemovePrivate`] also removes the place (EXIF GPS, IPTC location fields,
//!   XMP that names one), the vendor maker note, the owner and serial numbers, and any XMP that
//!   carries a preview.
//! * [`MetadataPolicy::RemoveAll`] writes none, except an orientation where the pixels could not
//!   be turned upright, because a sideways picture is a defect and not a privacy choice.
//!
//! Nothing is invented. Fields that are not understood are carried or dropped as blocks, never
//! reconstructed, and an unreadable block is dropped whole rather than guessed at.

pub mod container;
pub mod exif;
pub mod iptc;
pub mod scan;
pub mod tiff;
pub mod xmp;

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Bits of [`SourceMetadata::left_behind`]: parts of a TIFF-structured source's EXIF that were
/// not copied out of the file, because they cannot be moved without breaking them or are more
/// than this code carries.
pub mod left_behind {
    /// The vendor's maker note, or a DNG's private data (where a converter keeps it). Its
    /// offsets are into the file it came from, so it cannot be carried as a block of its own.
    pub const MAKER_NOTE: u8 = 1;
    /// A field larger than one is carried, or one that did not fit the block's budget.
    pub const OVERSIZE: u8 = 2;
}

/// What an export does with the source's metadata.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataPolicy {
    KeepAll,
    /// The default: the place and the identifiers go, the description of the picture stays.
    #[default]
    RemovePrivate,
    RemoveAll,
}

/// The three blocks of a source, as the file held them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMetadata {
    /// A TIFF block, without the "Exif\0\0" prefix.
    pub exif: Option<Arc<Vec<u8>>>,
    pub xmp: Option<Arc<Vec<u8>>>,
    pub iptc: Option<Arc<Vec<u8>>>,
    /// The EXIF orientation of the source, whether or not the pixels were turned: 1 when upright.
    pub source_orientation: u8,
    /// The pixels were turned upright when the file was opened, so the orientation is spent.
    pub orientation_applied: bool,
    /// What the source held that is not in `exif` (see [`left_behind`]); 0 for a source whose
    /// EXIF came whole.
    pub left_behind: u8,
}

impl Default for SourceMetadata {
    fn default() -> Self {
        Self {
            exif: None,
            xmp: None,
            iptc: None,
            source_orientation: 1,
            orientation_applied: false,
            left_behind: 0,
        }
    }
}

impl SourceMetadata {
    pub fn is_empty(&self) -> bool {
        self.exif.is_none() && self.xmp.is_none() && self.iptc.is_none()
    }

    /// Takes what a decoder returned, dropping what is over the sizes this code carries.
    pub fn from_blocks(exif: Option<Vec<u8>>, xmp: Option<Vec<u8>>, iptc: Option<Vec<u8>>) -> Self {
        // The JPEG decoder returns the TIFF block; a "Exif\0\0" prefix is removed if present.
        let exif = exif.map(|bytes| match bytes.strip_prefix(b"Exif\0\0") {
            Some(rest) => rest.to_vec(),
            None => bytes,
        });
        // JPEG hands back the Photoshop resource block; the stream is inside it.
        let iptc = iptc.and_then(|bytes| iptc::extract(&bytes));
        let keep = |bytes: Option<Vec<u8>>, limit: usize| {
            bytes
                .filter(|bytes| !bytes.is_empty() && bytes.len() <= limit)
                .map(Arc::new)
        };
        let exif = keep(exif, exif::MAX_EXIF_BYTES);
        let source_orientation = exif
            .as_ref()
            .and_then(|block| exif::orientation(block))
            .unwrap_or(1);
        Self {
            exif,
            xmp: keep(xmp, xmp::MAX_XMP_BYTES),
            iptc: keep(iptc, iptc::MAX_IPTC_BYTES),
            source_orientation,
            orientation_applied: false,
            left_behind: 0,
        }
    }
}

/// What a source carries, for the interface: what is there, and what an export under the
/// private policy would take out. The place itself is not shown, only that there is one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub has_exif: bool,
    pub has_xmp: bool,
    pub has_iptc: bool,
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub taken: Option<String>,
    pub has_location: bool,
    pub has_maker_note: bool,
    pub has_identifiers: bool,
    pub has_thumbnail: bool,
    /// The EXIF orientation of the file, 1 to 8.
    pub orientation: u8,
    /// The pixels were turned upright when the file was opened.
    pub orientation_applied: bool,
}

impl SourceMetadata {
    pub fn summary(&self) -> SourceSummary {
        let exif = self
            .exif
            .as_ref()
            .and_then(|block| exif::summarize(block))
            .unwrap_or_default();
        let xmp_place = self.xmp.as_ref().is_some_and(|packet| {
            let found = xmp::inspect(packet);
            found.place
        });
        let iptc_place = self
            .iptc
            .as_ref()
            .is_some_and(|stream| iptc::has_location(stream));
        SourceSummary {
            has_exif: self.exif.is_some(),
            has_xmp: self.xmp.is_some(),
            has_iptc: self.iptc.is_some(),
            make: exif.make,
            model: exif.model,
            lens: exif.lens,
            taken: exif.taken,
            has_location: exif.has_location || xmp_place || iptc_place,
            has_maker_note: exif.has_maker_note || self.left_behind & left_behind::MAKER_NOTE != 0,
            has_identifiers: exif.has_identifiers,
            has_thumbnail: exif.has_thumbnail,
            orientation: self.source_orientation,
            orientation_applied: self.orientation_applied,
        }
    }
}

/// One line of an export's account of its metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataNote {
    /// "kept", "removed" or "note".
    pub kind: &'static str,
    pub detail: String,
}

/// What an export did with the metadata, for the person to read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataReport {
    pub policy: MetadataPolicy,
    pub notes: Vec<MetadataNote>,
}

impl MetadataReport {
    fn kept(&mut self, detail: impl Into<String>) {
        self.notes.push(MetadataNote {
            kind: "kept",
            detail: detail.into(),
        });
    }

    pub(crate) fn removed(&mut self, detail: impl Into<String>) {
        self.notes.push(MetadataNote {
            kind: "removed",
            detail: detail.into(),
        });
    }

    fn note(&mut self, detail: impl Into<String>) {
        self.notes.push(MetadataNote {
            kind: "note",
            detail: detail.into(),
        });
    }
}

/// The blocks to write, after the policy and the edit have been applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prepared {
    pub exif: Option<Vec<u8>>,
    pub xmp: Option<Vec<u8>>,
    pub iptc: Option<Vec<u8>>,
}

impl Prepared {
    pub fn as_embedded(&self) -> container::Embedded<'_> {
        container::Embedded {
            exif: self.exif.as_deref(),
            xmp: self.xmp.as_deref(),
            iptc: self.iptc.as_deref(),
        }
    }
}

/// What the output file is, so the block can say so.
#[derive(Debug, Clone, Copy)]
pub struct OutputFacts {
    pub width: u32,
    pub height: u32,
    /// The file carries a profile other than sRGB.
    pub wide_gamut_profile: bool,
}

/// Applies `policy` to `source` for a picture of the given size.
pub fn prepare(
    source: &SourceMetadata,
    policy: MetadataPolicy,
    output: OutputFacts,
) -> (Prepared, MetadataReport) {
    let mut report = MetadataReport {
        policy,
        notes: Vec::new(),
    };
    let mut prepared = Prepared::default();
    // An orientation that was not spent on the pixels has to travel with them whatever else is
    // removed, or the picture comes out sideways.
    let carry_orientation = !source.orientation_applied && source.source_orientation != 1;
    let new_orientation = if source.orientation_applied {
        Some(1)
    } else {
        None
    };

    if source.is_empty() {
        if carry_orientation {
            prepared.exif = Some(exif::orientation_only(source.source_orientation));
            report.note("The picture could not be turned upright, so its orientation is stored.");
        }
        return (prepared, report);
    }

    if policy == MetadataPolicy::RemoveAll {
        if source.exif.is_some() {
            report.removed("EXIF (camera, exposure, date, place)");
        }
        if source.xmp.is_some() {
            report.removed("XMP (title, keywords, rating, edit history)");
        }
        if source.iptc.is_some() {
            report.removed("IPTC (caption, credit, copyright, place)");
        }
        if carry_orientation {
            prepared.exif = Some(exif::orientation_only(source.source_orientation));
            report.note("The picture could not be turned upright, so its orientation is stored.");
        }
        return (prepared, report);
    }

    let private = policy == MetadataPolicy::RemovePrivate;

    if let Some(block) = &source.exif {
        let spec = exif::CleanSpec {
            private,
            orientation: new_orientation,
            dimensions: Some((output.width, output.height)),
            colour_space_uncalibrated: output.wide_gamut_profile,
        };
        match exif::clean(block, &spec) {
            Some(cleaned) => {
                let summary = exif::summarize(&cleaned.bytes);
                let mut what = vec!["EXIF".to_string()];
                if let Some(summary) = &summary {
                    let camera = [summary.make.as_deref(), summary.model.as_deref()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !camera.is_empty() {
                        what.push(camera);
                    }
                    if let Some(taken) = &summary.taken {
                        what.push(taken.clone());
                    }
                }
                report.kept(what.join(", "));
                if cleaned.removed.location {
                    report.removed("the place the picture was taken (GPS)");
                }
                if cleaned.removed.maker_note {
                    report.removed(
                        "the camera maker's private notes (they can hold serial numbers, a place or a preview)",
                    );
                }
                if cleaned.removed.identifiers {
                    report.removed("the camera owner's name and serial numbers");
                }
                if cleaned.removed.thumbnail {
                    report.removed(
                        "the small embedded preview (it is a copy of the picture before editing)",
                    );
                }
                if !private {
                    if let Some(summary) = &summary {
                        if summary.has_location {
                            report.note("The place the picture was taken is in the file.");
                        }
                    }
                }
                prepared.exif = Some(cleaned.bytes);
            }
            None => {
                report.removed("EXIF, because it could not be read safely enough to clean");
                if carry_orientation {
                    prepared.exif = Some(exif::orientation_only(source.source_orientation));
                }
            }
        }
    } else if carry_orientation {
        prepared.exif = Some(exif::orientation_only(source.source_orientation));
    }

    // What a TIFF-structured source (a DNG) held that could not be moved out of it. This is
    // said under every policy that writes EXIF: the person is told it is not in the file, not
    // left to find that out.
    if source.left_behind & left_behind::MAKER_NOTE != 0 {
        report.removed(
            "the camera maker's private notes (a DNG keeps them where they cannot be moved out of it)",
        );
    }
    if source.left_behind & left_behind::OVERSIZE != 0 {
        report.removed("EXIF fields too large to carry");
    }

    if let Some(packet) = &source.xmp {
        let findings = xmp::inspect(packet);
        if private && findings.any() {
            let mut why = Vec::new();
            if findings.place {
                why.push("names a place");
            }
            if findings.identity {
                why.push("names an owner or a serial number");
            }
            if findings.preview {
                why.push("holds a preview of the picture");
            }
            if findings.unreadable {
                why.push("could not be read");
            }
            report.removed(format!("XMP, because it {}", why.join(" and ")));
        } else {
            report.kept("XMP (title, keywords, rating, edit history)");
            prepared.xmp = Some(packet.as_ref().clone());
        }
    }

    if let Some(stream) = &source.iptc {
        if private && iptc::has_location(stream) {
            match iptc::without_location(stream) {
                Some(cleaned) if !cleaned.is_empty() => {
                    report.kept("IPTC (caption, credit, copyright)");
                    report.removed("the IPTC place fields");
                    prepared.iptc = Some(cleaned);
                }
                _ => report.removed("IPTC, because it named a place"),
            }
        } else if iptc::is_readable(stream) {
            report.kept("IPTC (caption, credit, copyright)");
            prepared.iptc = Some(stream.as_ref().clone());
        } else {
            report.removed("IPTC, because it could not be read");
        }
    }

    (prepared, report)
}

/// The metadata of one output file: what is to be written and the account of it, which the
/// writer completes with what the file could not hold.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub prepared: Prepared,
    pub report: MetadataReport,
}

impl Plan {
    /// Applies `policy` to `source` (if the document has one) for an output of this size.
    pub fn new(
        source: Option<&SourceMetadata>,
        policy: MetadataPolicy,
        output: OutputFacts,
    ) -> Self {
        match source {
            Some(source) => {
                let (prepared, report) = prepare(source, policy, output);
                Self { prepared, report }
            }
            None => Self {
                prepared: Prepared::default(),
                report: MetadataReport {
                    policy,
                    notes: Vec::new(),
                },
            },
        }
    }
}

/// Embeds `prepared` into the encoded file and adds to the report what could not be stored.
pub fn embed_into(
    format: container::Format,
    encoded: Vec<u8>,
    prepared: &Prepared,
    report: &mut MetadataReport,
) -> Result<Vec<u8>, crate::error::AppError> {
    let (bytes, placed) = container::embed(format, encoded, &prepared.as_embedded())
        .map_err(|_| crate::error::AppError::ExportFailure)?;
    report_unplaced(report, prepared, placed);
    Ok(bytes)
}

/// Turns "kept" into "removed" for each block the file turned out to have no place for.
pub fn report_unplaced(
    report: &mut MetadataReport,
    prepared: &Prepared,
    placed: container::Placed,
) {
    let mut unplaced = |present: bool, stored: bool, name: &str, why: &str| {
        if present && !stored {
            report
                .notes
                .retain(|n| !(n.kind == "kept" && n.detail.starts_with(name)));
            report.notes.push(MetadataNote {
                kind: "removed",
                detail: format!("{name}, {why}"),
            });
        }
    };
    unplaced(
        prepared.exif.is_some(),
        placed.exif,
        "EXIF",
        "because it does not fit in this kind of file",
    );
    unplaced(
        prepared.xmp.is_some(),
        placed.xmp,
        "XMP",
        "because it does not fit in this kind of file",
    );
    unplaced(
        prepared.iptc.is_some(),
        placed.iptc,
        "IPTC",
        "which this kind of file has no place for (use JPEG or PSD)",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> OutputFacts {
        OutputFacts {
            width: 100,
            height: 50,
            wide_gamut_profile: false,
        }
    }

    #[test]
    fn an_empty_source_writes_nothing() {
        let (prepared, report) =
            prepare(&SourceMetadata::default(), MetadataPolicy::KeepAll, facts());
        assert_eq!(prepared, Prepared::default());
        assert!(report.notes.is_empty());
    }

    #[test]
    fn an_orientation_that_was_not_applied_survives_remove_all() {
        let source = SourceMetadata {
            source_orientation: 6,
            orientation_applied: false,
            exif: Some(Arc::new(exif::orientation_only(6))),
            ..SourceMetadata::default()
        };
        let (prepared, _) = prepare(&source, MetadataPolicy::RemoveAll, facts());
        assert_eq!(
            exif::orientation(prepared.exif.as_deref().unwrap()),
            Some(6)
        );
    }

    #[test]
    fn an_orientation_that_was_applied_is_written_as_upright() {
        let source = SourceMetadata {
            source_orientation: 6,
            orientation_applied: true,
            exif: Some(Arc::new(exif::orientation_only(6))),
            ..SourceMetadata::default()
        };
        let (prepared, _) = prepare(&source, MetadataPolicy::KeepAll, facts());
        assert_eq!(
            exif::orientation(prepared.exif.as_deref().unwrap()),
            Some(1)
        );
        // And nothing at all under remove-all: there is no orientation left to say.
        let (none, _) = prepare(&source, MetadataPolicy::RemoveAll, facts());
        assert!(none.exif.is_none());
    }

    #[test]
    fn remove_private_drops_an_xmp_packet_that_names_a_place_and_says_why() {
        let source = SourceMetadata::from_blocks(
            None,
            Some(b"<x:xmpmeta><rdf:Description exif:GPSLatitude=\"1\"/></x:xmpmeta>".to_vec()),
            None,
        );
        let (kept, _) = prepare(&source, MetadataPolicy::KeepAll, facts());
        assert!(kept.xmp.is_some());
        let (private, report) = prepare(&source, MetadataPolicy::RemovePrivate, facts());
        assert!(private.xmp.is_none());
        assert!(report
            .notes
            .iter()
            .any(|n| n.kind == "removed" && n.detail.contains("names a place")));
    }

    #[test]
    fn an_unreadable_exif_block_is_dropped_whole_and_reported() {
        let source = SourceMetadata::from_blocks(Some(b"II*\0garbage".to_vec()), None, None);
        let (prepared, report) = prepare(&source, MetadataPolicy::KeepAll, facts());
        assert!(prepared.exif.is_none());
        assert!(report
            .notes
            .iter()
            .any(|n| n.kind == "removed" && n.detail.starts_with("EXIF")));
    }

    /// A JPEG of 40 x 20 pixels the way a phone writes one: the sensor's orientation with a tag
    /// to turn it, a place, a maker note and a thumbnail in EXIF, a packet that names a city,
    /// and captions with a place in IPTC.
    fn photograph(dir: &std::path::Path) -> std::path::PathBuf {
        use image::codecs::jpeg::JpegEncoder;
        use image::{ExtendedColorType, ImageEncoder};
        let rgb: Vec<u8> = (0..40 * 20)
            .flat_map(|i| [(i % 40 * 6) as u8, (i / 40 * 12) as u8, 90])
            .collect();
        let mut encoded = Vec::new();
        JpegEncoder::new_with_quality(&mut encoded, 92)
            .write_image(&rgb, 40, 20, ExtendedColorType::Rgb8)
            .unwrap();
        let exif = exif::fixture::Fixture::build();
        let xmp = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:Description xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\" photoshop:City=\"Vancouver\"/></x:xmpmeta>";
        let mut iim = Vec::new();
        for (number, text) in [(25_u8, "harbour"), (90, "Vancouver"), (116, "(c) A")] {
            iim.extend_from_slice(&[0x1C, 2, number]);
            iim.extend_from_slice(&(text.len() as u16).to_be_bytes());
            iim.extend_from_slice(text.as_bytes());
        }
        let (with, placed) = container::embed(
            container::Format::Jpeg,
            encoded,
            &container::Embedded {
                exif: Some(&exif),
                xmp: Some(xmp),
                iptc: Some(&iim),
            },
        )
        .unwrap();
        assert!(placed.exif && placed.xmp && placed.iptc);
        let path = dir.join("phone.jpg");
        std::fs::write(&path, with).unwrap();
        path
    }

    #[test]
    fn a_photograph_goes_through_open_and_export_under_each_policy() {
        use crate::domain::ExportProfile;
        use crate::infrastructure::{load_image, save_image_with_metadata};
        use image::GenericImageView;
        let dir = tempfile::tempdir().unwrap();
        let path = photograph(dir.path());
        let loaded = load_image(&path).unwrap();
        // Turned upright on open: 40 x 20 with orientation 6 is a portrait.
        assert_eq!(loaded.original.dimensions(), (20, 40));
        let source = loaded.source_metadata.clone().unwrap();
        assert!(source.orientation_applied);
        assert_eq!(source.source_orientation, 6);
        let facts = OutputFacts {
            width: 20,
            height: 40,
            wide_gamut_profile: false,
        };
        let export = |policy: MetadataPolicy| {
            let out = dir.path().join(format!("{policy:?}.jpg"));
            let mut plan = Plan::new(Some(&source), policy, facts);
            save_image_with_metadata(
                &loaded.original,
                &path,
                &out,
                ExportProfile::Lossless,
                Some(&mut plan),
            )
            .unwrap();
            (scan::read(&out).unwrap(), plan.report, out)
        };

        let (kept, report, out) = export(MetadataPolicy::KeepAll);
        let summary = exif::summarize(kept.exif.as_deref().unwrap()).unwrap();
        // The tag is spent, the size is the new size, the thumbnail is gone, the rest is there.
        assert_eq!(summary.orientation, 1);
        assert_eq!(summary.pixel_dimensions, Some((20, 40)));
        assert!(!summary.has_thumbnail);
        assert!(summary.has_location && summary.has_maker_note && summary.has_identifiers);
        assert_eq!(summary.model.as_deref(), Some("Cam-One"));
        assert!(kept.xmp.is_some());
        assert!(iptc::has_location(kept.iptc.as_deref().unwrap()));
        assert!(report.notes.iter().any(|n| n.kind == "kept"));
        // And the file is a picture of 20 x 40 that a stranger's decoder reads.
        assert_eq!(image::open(&out).unwrap().dimensions(), (20, 40));

        let (private, report, _) = export(MetadataPolicy::RemovePrivate);
        let summary = exif::summarize(private.exif.as_deref().unwrap()).unwrap();
        assert!(!summary.has_location && !summary.has_maker_note && !summary.has_identifiers);
        assert!(!summary.has_thumbnail);
        assert_eq!(summary.make.as_deref(), Some("Acme"));
        assert!(private.xmp.is_none(), "the packet names a city");
        let iim = private.iptc.as_deref().unwrap();
        assert!(!iptc::has_location(iim));
        assert!(String::from_utf8_lossy(iim).contains("harbour"));
        assert!(String::from_utf8_lossy(iim).contains("(c) A"));
        let removed: Vec<_> = report
            .notes
            .iter()
            .filter(|n| n.kind == "removed")
            .map(|n| n.detail.as_str())
            .collect();
        for wanted in [
            "the place",
            "maker",
            "serial",
            "preview",
            "XMP",
            "IPTC place",
        ] {
            assert!(
                removed.iter().any(|d| d.contains(wanted)),
                "{wanted}: {removed:?}"
            );
        }
        // Nothing of the place is anywhere in the bytes of the file.
        let bytes = std::fs::read(dir.path().join("RemovePrivate.jpg")).unwrap();
        for secret in [
            &b"Vancouver"[..],
            b"SERIAL-0042",
            b"SECRET-MAKER-NOTE",
            b"THUMBNAIL",
        ] {
            assert!(
                !bytes.windows(secret.len()).any(|w| w == secret),
                "{}",
                String::from_utf8_lossy(secret)
            );
        }

        let (none, report, _) = export(MetadataPolicy::RemoveAll);
        assert!(none.is_empty());
        assert!(report
            .notes
            .iter()
            .all(|n| n.kind != "kept" && n.kind != "note"));
    }

    /// Opening turns the picture upright itself, which is only right while the decoder does
    /// not. If an upgrade of the `image` crate starts applying the tag, this fails, and the
    /// picture would otherwise be turned twice.
    #[test]
    fn the_decoder_leaves_the_orientation_to_the_application() {
        use image::GenericImageView;
        use image::{ImageDecoder, ImageReader};
        let dir = tempfile::tempdir().unwrap();
        let path = photograph(dir.path());
        let mut decoder = ImageReader::open(&path)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();
        assert!(decoder.exif_metadata().unwrap().is_some());
        let decoded = image::DynamicImage::from_decoder(decoder).unwrap();
        // 40 x 20 as stored; the tag says turn it a quarter.
        assert_eq!(decoded.dimensions(), (40, 20));
        assert_eq!(
            crate::infrastructure::load_image(&path)
                .unwrap()
                .original
                .dimensions(),
            (20, 40)
        );
    }

    #[test]
    fn a_png_export_says_iptc_has_no_place_there_and_keeps_the_rest() {
        use crate::domain::ExportProfile;
        use crate::infrastructure::{load_image, save_image_with_metadata};
        let dir = tempfile::tempdir().unwrap();
        let path = photograph(dir.path());
        let loaded = load_image(&path).unwrap();
        let source = loaded.source_metadata.clone().unwrap();
        let out = dir.path().join("kept.png");
        let mut plan = Plan::new(
            Some(&source),
            MetadataPolicy::KeepAll,
            OutputFacts {
                width: 20,
                height: 40,
                wide_gamut_profile: false,
            },
        );
        save_image_with_metadata(
            &loaded.original,
            &path,
            &out,
            ExportProfile::Lossless,
            Some(&mut plan),
        )
        .unwrap();
        let found = scan::read(&out).unwrap();
        assert!(found.exif.is_some() && found.xmp.is_some());
        assert!(found.iptc.is_none());
        let notes = &plan.report.notes;
        assert!(notes
            .iter()
            .any(|n| n.kind == "removed" && n.detail.starts_with("IPTC")));
        assert!(!notes
            .iter()
            .any(|n| n.kind == "kept" && n.detail.starts_with("IPTC")));
        assert!(notes
            .iter()
            .any(|n| n.kind == "kept" && n.detail.starts_with("EXIF")));
    }

    #[test]
    fn a_file_that_could_not_be_turned_upright_keeps_its_orientation_even_when_all_is_removed() {
        use crate::domain::ExportProfile;
        use crate::infrastructure::save_image_with_metadata;
        let dir = tempfile::tempdir().unwrap();
        let path = photograph(dir.path());
        // As a region or reduced open leaves it: the tag read, not applied.
        let mut source = scan::read(&path).unwrap();
        assert!(!source.orientation_applied);
        assert_eq!(source.source_orientation, 6);
        let image = image::DynamicImage::new_rgb8(40, 20);
        let out = dir.path().join("kept-orientation.png");
        let mut plan = Plan::new(
            Some(&source),
            MetadataPolicy::RemoveAll,
            OutputFacts {
                width: 40,
                height: 20,
                wide_gamut_profile: false,
            },
        );
        save_image_with_metadata(
            &image,
            &path,
            &out,
            ExportProfile::Lossless,
            Some(&mut plan),
        )
        .unwrap();
        let found = scan::read(&out).unwrap();
        assert_eq!(found.source_orientation, 6);
        assert!(found.xmp.is_none() && found.iptc.is_none());
        assert!(plan.report.notes.iter().any(|n| n.kind == "note"));
        source.orientation_applied = true;
    }

    #[test]
    fn the_exif_prefix_is_removed_and_oversize_blocks_are_not_carried() {
        let mut prefixed = b"Exif\0\0".to_vec();
        prefixed.extend_from_slice(&exif::orientation_only(3));
        let source = SourceMetadata::from_blocks(Some(prefixed), None, None);
        assert_eq!(source.source_orientation, 3);
        let big = SourceMetadata::from_blocks(Some(vec![0; exif::MAX_EXIF_BYTES + 1]), None, None);
        assert!(big.exif.is_none());
    }
}
