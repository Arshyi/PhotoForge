//! Writing the shape layers of a document as SVG, and saying what did not go into the file.
//!
//! SVG is the one interchange format here that is *vector*, so what it can hold is a subset of a
//! document: the shape layers and the groups that hold them. Everything else (pixels, text, smart
//! objects, adjustments, masks) has no honest SVG form and is **left out and named**, never
//! rasterised into the file behind the person's back. The rule is the Photoshop exporter's:
//! nothing is dropped silently, and the account is part of the result.
//!
//! The drawing is the renderer's, not a nearby one. A shape layer's transform is taken about the
//! centre of the *canvas* (that is how the compositor maps shapes), the fill and the stroke are
//! painted in that order, fills apply only to closed geometry, and colours are the document's
//! linear values encoded to sRGB, which is what SVG's `#rrggbb` is.
use crate::color::srgb_encode;
use crate::error::AppError;
use crate::layers::{BlendMode, Layer, LayerContent, LayerDocument, LayerTransform, ShapeContent};
use crate::psd::{ReportEntry, ReportKind};
use crate::vector::{FillRule, LineCap, LineJoin, PathCommand, ShapeGeometry};
use serde::Serialize;
use std::fmt::Write as _;

/// What an export did, in the shape the Photoshop export report has, so one dialog serves both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SvgReport {
    /// Always "SVG".
    pub format: String,
    pub width: u32,
    pub height: u32,
    /// Shape layers written (groups are counted apart).
    pub layers_written: usize,
    pub groups_written: usize,
    pub bytes: u64,
    pub entries: Vec<ReportEntry>,
}

impl SvgReport {
    fn push(&mut self, kind: ReportKind, layer: Option<&str>, detail: impl Into<String>) {
        self.entries.push(ReportEntry {
            kind,
            layer: layer.map(str::to_string),
            detail: detail.into(),
        });
    }

    pub fn count(&self, kind: ReportKind) -> usize {
        self.entries.iter().filter(|e| e.kind == kind).count()
    }
}

/// A finished SVG and the account of it.
#[derive(Debug, Clone)]
pub struct SvgExport {
    pub text: String,
    pub report: SvgReport,
}

/// What an export is given besides the document.
#[derive(Debug, Clone, Default)]
pub struct SvgOptions {
    /// Edits that sit over the whole document, which an SVG cannot hold; they are reported.
    pub operations: Vec<crate::domain::EditOperation>,
}

/// The mix-blend-mode CSS value for a blend mode, where the W3C compositing set has one.
fn css_blend(mode: BlendMode) -> Option<&'static str> {
    Some(match mode {
        BlendMode::Normal => return None,
        BlendMode::Multiply => "multiply",
        BlendMode::Screen => "screen",
        BlendMode::Overlay => "overlay",
        BlendMode::Darken => "darken",
        BlendMode::Lighten => "lighten",
        BlendMode::ColorDodge => "color-dodge",
        BlendMode::ColorBurn => "color-burn",
        BlendMode::SoftLight => "soft-light",
        BlendMode::HardLight => "hard-light",
        BlendMode::Difference => "difference",
        BlendMode::Exclusion => "exclusion",
        BlendMode::Hue => "hue",
        BlendMode::Saturation => "saturation",
        BlendMode::Color => "color",
        BlendMode::Luminosity => "luminosity",
        _ => return None,
    })
}

/// A number as short as it can be written without changing it at the precision of a picture.
pub(crate) fn num(value: f32) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let mut text = format!("{value:.4}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        text = "0".to_string();
    }
    text
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// `#rrggbb`, from a linear working-space colour, and whether a channel had to be clipped.
fn hex(red: f32, green: f32, blue: f32) -> (String, bool) {
    let mut clipped = false;
    let mut byte = |linear: f32| -> u8 {
        if !linear.is_finite() || !(-1e-3..=1.0 + 1e-3).contains(&linear) {
            clipped = true;
        }
        (srgb_encode(linear.clamp(0.0, 1.0)) * 255.0).round() as u8
    };
    let (r, g, b) = (byte(red), byte(green), byte(blue));
    (format!("#{r:02x}{g:02x}{b:02x}"), clipped)
}

struct Writer<'a> {
    document: &'a LayerDocument,
    out: String,
    report: SvgReport,
    /// Something is partly transparent or blended, which an SVG composites in sRGB and a
    /// high-precision document in linear light.
    translucent: bool,
}

impl Writer<'_> {
    fn indent(&mut self, depth: usize) {
        for _ in 0..depth {
            self.out.push_str("  ");
        }
    }

    fn write_layer(&mut self, layer: &Layer, depth: usize) {
        match &layer.content {
            LayerContent::Shape { shape } => self.shape(layer, shape, depth),
            LayerContent::Group { children, isolated } => {
                self.group(layer, children, *isolated, depth)
            }
            LayerContent::Pixel { .. } => self.report.push(
                ReportKind::NotExported,
                Some(&layer.name),
                "A pixel layer is not in the SVG: it holds pixels, and this file holds shapes.",
            ),
            LayerContent::Text { .. } => self.report.push(
                ReportKind::NotExported,
                Some(&layer.name),
                "A text layer is not in the SVG.",
            ),
            LayerContent::SmartObject { .. } => self.report.push(
                ReportKind::NotExported,
                Some(&layer.name),
                "A smart object is not in the SVG.",
            ),
            LayerContent::Adjustment { .. } => self.report.push(
                ReportKind::NotExported,
                Some(&layer.name),
                "An adjustment layer has no SVG form; the shapes beneath it are drawn without it.",
            ),
        }
    }

    /// The attributes every layer element carries, and the entries for what could not be kept.
    fn layer_attributes(&mut self, layer: &Layer) -> String {
        let mut attributes = String::new();
        if !layer.visible {
            attributes.push_str(" display=\"none\"");
        }
        if layer.opacity < 1.0 {
            self.translucent = true;
            let _ = write!(attributes, " opacity=\"{}\"", num(layer.opacity));
        }
        if let Some(css) = css_blend(layer.blend_mode) {
            self.translucent = true;
            let _ = write!(attributes, " style=\"mix-blend-mode:{css}\"");
        } else if layer.blend_mode != BlendMode::Normal {
            self.report.push(
                ReportKind::Approximated,
                Some(&layer.name),
                format!(
                    "The {:?} blend mode has no SVG equivalent; the layer is written as Normal.",
                    layer.blend_mode
                ),
            );
        }
        if !layer.transform.is_identity() {
            let [a, b, c, d, e, f] = matrix(&layer.transform, self.document);
            let _ = write!(
                attributes,
                " transform=\"matrix({} {} {} {} {} {})\"",
                num(a),
                num(b),
                num(c),
                num(d),
                num(e),
                num(f)
            );
        }
        if layer.mask.is_some() {
            self.report.push(
                ReportKind::NotExported,
                Some(&layer.name),
                "The layer's mask is not in the SVG; the layer is drawn unmasked.",
            );
        }
        if layer.clipped {
            self.report.push(
                ReportKind::Approximated,
                Some(&layer.name),
                "The layer is clipped to the one beneath it; the SVG draws it unclipped.",
            );
        }
        attributes
    }

    fn title(&mut self, layer: &Layer, depth: usize) {
        if layer.name.is_empty() {
            return;
        }
        self.indent(depth);
        let _ = writeln!(self.out, "<title>{}</title>", escape(&layer.name));
    }

    fn group(&mut self, layer: &Layer, children: &[Layer], isolated: bool, depth: usize) {
        let attributes = self.layer_attributes(layer);
        if !isolated && (layer.opacity < 1.0 || layer.blend_mode != BlendMode::Normal) {
            self.report.push(
                ReportKind::Approximated,
                Some(&layer.name),
                "A pass-through group with an opacity or blend mode is written as an isolated group.",
            );
        }
        self.indent(depth);
        let _ = writeln!(self.out, "<g{attributes}>");
        self.title(layer, depth + 1);
        for child in children {
            self.write_layer(child, depth + 1);
        }
        self.indent(depth);
        self.out.push_str("</g>\n");
        self.report.groups_written += 1;
    }

    fn shape(&mut self, layer: &Layer, shape: &ShapeContent, depth: usize) {
        let attributes = self.layer_attributes(layer);
        let mut clipped_colour = false;
        let mut paint = String::new();

        self.translucent |= shape.fill.is_some_and(|c| c.alpha > 0.0 && c.alpha < 1.0)
            || shape.stroke.is_some_and(|c| c.alpha > 0.0 && c.alpha < 1.0);
        let fills = shape
            .fill
            .filter(|c| c.alpha > 0.0 && shape.geometry.is_closed());
        match fills {
            Some(fill) => {
                let (colour, clipped) = hex(fill.red, fill.green, fill.blue);
                clipped_colour |= clipped;
                let _ = write!(paint, " fill=\"{colour}\"");
                if fill.alpha < 1.0 {
                    let _ = write!(paint, " fill-opacity=\"{}\"", num(fill.alpha));
                }
                if shape.fill_rule == FillRule::EvenOdd {
                    paint.push_str(" fill-rule=\"evenodd\"");
                }
            }
            None => paint.push_str(" fill=\"none\""),
        }
        if let (Some(stroke), Some(style)) = (shape.stroke, shape.stroke_style) {
            if stroke.alpha > 0.0 && style.is_valid() {
                let (colour, clipped) = hex(stroke.red, stroke.green, stroke.blue);
                clipped_colour |= clipped;
                let _ = write!(
                    paint,
                    " stroke=\"{colour}\" stroke-width=\"{}\"",
                    num(style.width)
                );
                if stroke.alpha < 1.0 {
                    let _ = write!(paint, " stroke-opacity=\"{}\"", num(stroke.alpha));
                }
                match style.cap {
                    LineCap::Butt => {}
                    LineCap::Round => paint.push_str(" stroke-linecap=\"round\""),
                    LineCap::Square => paint.push_str(" stroke-linecap=\"square\""),
                }
                // SVG's own default is a mitre; PhotoForge's is round, so it is always said.
                match style.join {
                    LineJoin::Round => paint.push_str(" stroke-linejoin=\"round\""),
                    LineJoin::Bevel => paint.push_str(" stroke-linejoin=\"bevel\""),
                    LineJoin::Miter => {
                        paint.push_str(" stroke-linejoin=\"miter\"");
                        if (style.miter_limit - 4.0).abs() > 1e-6 {
                            let _ =
                                write!(paint, " stroke-miterlimit=\"{}\"", num(style.miter_limit));
                        }
                    }
                }
                let (sx, sy) = (layer.transform.scale_x.abs(), layer.transform.scale_y.abs());
                if (sx - sy).abs() > 1e-4 {
                    self.report.push(
                        ReportKind::Approximated,
                        Some(&layer.name),
                        "The layer is scaled differently across and down: PhotoForge draws the stroke at the average of the two, an SVG scales it with each.",
                    );
                }
            }
        }
        if clipped_colour {
            self.report.push(
                ReportKind::Approximated,
                Some(&layer.name),
                "A colour outside what sRGB can show was clipped to sRGB.",
            );
        }

        let element = match &shape.geometry {
            ShapeGeometry::Rectangle {
                x,
                y,
                width,
                height,
                corner_radius,
            } => {
                let radius = corner_radius.min(width * 0.5).min(height * 0.5).max(0.0);
                let mut text = format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                    num(*x),
                    num(*y),
                    num(*width),
                    num(*height)
                );
                if radius > 0.0 {
                    let _ = write!(text, " rx=\"{0}\" ry=\"{0}\"", num(radius));
                }
                text
            }
            ShapeGeometry::Ellipse { cx, cy, rx, ry } => format!(
                "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"",
                num(*cx),
                num(*cy),
                num(*rx),
                num(*ry)
            ),
            ShapeGeometry::Line { x1, y1, x2, y2 } => format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"",
                num(*x1),
                num(*y1),
                num(*x2),
                num(*y2)
            ),
            geometry => format!("<path d=\"{}\"", path_data(geometry)),
        };

        self.indent(depth);
        if layer.name.is_empty() {
            let _ = writeln!(self.out, "{element}{attributes}{paint}/>");
        } else {
            let _ = writeln!(self.out, "{element}{attributes}{paint}>");
            self.title(layer, depth + 1);
            self.indent(depth);
            self.out.push_str(match &shape.geometry {
                ShapeGeometry::Rectangle { .. } => "</rect>\n",
                ShapeGeometry::Ellipse { .. } => "</ellipse>\n",
                ShapeGeometry::Line { .. } => "</line>\n",
                _ => "</path>\n",
            });
        }
        self.report.layers_written += 1;
    }
}

/// The affine matrix of a layer transform, as SVG's `matrix(a b c d e f)`.
///
/// The compositor takes a shape's transform about the centre of the canvas: scale and flip, then
/// rotate, about the centre, then translate. This is that, written out.
fn matrix(transform: &LayerTransform, document: &LayerDocument) -> [f32; 6] {
    let (cx, cy) = (
        document.canvas_width as f32 / 2.0,
        document.canvas_height as f32 / 2.0,
    );
    let sx = if transform.flip_horizontal {
        -transform.scale_x
    } else {
        transform.scale_x
    };
    let sy = if transform.flip_vertical {
        -transform.scale_y
    } else {
        transform.scale_y
    };
    let (sin, cos) = transform.rotation_degrees.to_radians().sin_cos();
    let (a, b, c, d) = (cos * sx, sin * sx, -sin * sy, cos * sy);
    let e = cx + transform.translate_x - (a * cx + c * cy);
    let f = cy + transform.translate_y - (b * cx + d * cy);
    [a, b, c, d, e, f]
}

/// Path data for a geometry, from the same commands the renderer flattens.
fn path_data(geometry: &ShapeGeometry) -> String {
    let mut data = String::new();
    for command in geometry.to_path().commands {
        if !data.is_empty() {
            data.push(' ');
        }
        match command {
            PathCommand::MoveTo { x, y } => {
                let _ = write!(data, "M{} {}", num(x), num(y));
            }
            PathCommand::LineTo { x, y } => {
                let _ = write!(data, "L{} {}", num(x), num(y));
            }
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                let _ = write!(
                    data,
                    "C{} {} {} {} {} {}",
                    num(c1x),
                    num(c1y),
                    num(c2x),
                    num(c2y),
                    num(x),
                    num(y)
                );
            }
            PathCommand::Close => data.push('Z'),
        }
    }
    data
}

/// Writes the shape layers of `document` as SVG text.
pub fn export_svg(document: &LayerDocument, options: &SvgOptions) -> Result<SvgExport, AppError> {
    document.validate()?;
    let mut writer = Writer {
        document,
        out: String::new(),
        report: SvgReport {
            format: "SVG".to_string(),
            width: document.canvas_width,
            height: document.canvas_height,
            ..SvgReport::default()
        },
        translucent: false,
    };
    let _ = writeln!(writer.out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(
        writer.out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">",
        w = document.canvas_width,
        h = document.canvas_height
    );
    for layer in &document.layers {
        writer.write_layer(layer, 1);
    }
    writer.out.push_str("</svg>\n");
    if !options.operations.is_empty() {
        let kinds: Vec<String> = options
            .operations
            .iter()
            .map(|operation| operation.kind().replace('_', " "))
            .collect();
        writer.report.push(
            ReportKind::NotExported,
            None,
            format!(
                "Edits over the whole document ({}) are not in the SVG.",
                kinds.join(", ")
            ),
        );
    }
    if writer.translucent && document.precision == crate::pixel::DocumentPrecision::LinearSrgbF32 {
        writer.report.push(
            ReportKind::Approximated,
            None,
            "Where partly transparent or blended shapes overlap, an SVG blends in sRGB and this document in linear light, so those overlaps look different (darker, usually) in an SVG viewer.",
        );
    }
    if writer.report.layers_written == 0 {
        writer.report.push(
            ReportKind::Note,
            None,
            "The document has no shape layers, so the SVG is an empty canvas.",
        );
    }
    writer.report.bytes = writer.out.len() as u64;
    Ok(SvgExport {
        text: writer.out,
        report: writer.report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::{LayerContent, ShapeColor};
    use crate::operations::structure::new_layer;
    use crate::vector::StrokeStyle;

    fn shape(name: &str, geometry: ShapeGeometry) -> Layer {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(1);
        new_layer(
            &format!("s{}", NEXT.fetch_add(1, Ordering::Relaxed)),
            name,
            LayerContent::Shape {
                shape: Box::new(ShapeContent {
                    geometry,
                    fill: Some(ShapeColor::new(1.0, 0.0, 0.0, 1.0)),
                    stroke: None,
                    stroke_style: None,
                    fill_rule: FillRule::NonZero,
                }),
            },
            "t",
        )
    }

    fn document(layers: Vec<Layer>) -> LayerDocument {
        let mut document = LayerDocument::new(200, 100);
        document.layers = layers;
        document
    }

    fn rect() -> ShapeGeometry {
        ShapeGeometry::Rectangle {
            x: 10.0,
            y: 20.0,
            width: 80.0,
            height: 40.0,
            corner_radius: 0.0,
        }
    }

    #[test]
    fn numbers_are_short_and_never_negative_zero() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(-0.00001), "0");
        assert_eq!(num(12.34567), "12.3457");
        assert_eq!(num(f32::NAN), "0");
    }

    #[test]
    fn a_rectangle_is_a_rect_with_its_colour_encoded_for_srgb() {
        let export = export_svg(
            &document(vec![shape("Box", rect())]),
            &SvgOptions::default(),
        )
        .unwrap();
        assert!(export.text.contains("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"100\" viewBox=\"0 0 200 100\">"));
        assert!(export
            .text
            .contains("<rect x=\"10\" y=\"20\" width=\"80\" height=\"40\" fill=\"#ff0000\">"));
        assert!(export.text.contains("<title>Box</title>"));
        assert_eq!(export.report.layers_written, 1);
        assert!(
            export.report.entries.is_empty(),
            "{:?}",
            export.report.entries
        );
        // Linear 0.5 is sRGB 188 (0xbc), not 128.
        let mut grey = shape("Grey", rect());
        if let LayerContent::Shape { shape } = &mut grey.content {
            shape.fill = Some(ShapeColor::new(0.5, 0.5, 0.5, 0.5));
        }
        let export = export_svg(&document(vec![grey]), &SvgOptions::default()).unwrap();
        assert!(export
            .text
            .contains("fill=\"#bcbcbc\" fill-opacity=\"0.5\""));
    }

    #[test]
    fn stroke_cap_join_and_fill_rule_are_written() {
        let mut layer = shape("Outlined", rect());
        if let LayerContent::Shape { shape } = &mut layer.content {
            shape.fill = None;
            shape.stroke = Some(ShapeColor::new(0.0, 0.0, 1.0, 0.75));
            shape.stroke_style = Some(StrokeStyle {
                width: 6.0,
                cap: LineCap::Square,
                join: LineJoin::Miter,
                miter_limit: 7.0,
            });
            shape.fill_rule = FillRule::EvenOdd;
        }
        let export = export_svg(&document(vec![layer]), &SvgOptions::default()).unwrap();
        for wanted in [
            "fill=\"none\"",
            "stroke=\"#0000ff\"",
            "stroke-width=\"6\"",
            "stroke-opacity=\"0.75\"",
            "stroke-linecap=\"square\"",
            "stroke-linejoin=\"miter\"",
            "stroke-miterlimit=\"7\"",
        ] {
            assert!(export.text.contains(wanted), "{wanted}\n{}", export.text);
        }
    }

    #[test]
    fn each_join_and_cap_is_written_as_itself_and_the_default_is_said() {
        let stroked = |cap, join| {
            let mut layer = shape("Joined", rect());
            if let LayerContent::Shape { shape } = &mut layer.content {
                shape.fill = None;
                shape.stroke = Some(ShapeColor::new(0.0, 0.0, 0.0, 1.0));
                shape.stroke_style = Some(StrokeStyle {
                    width: 2.0,
                    cap,
                    join,
                    miter_limit: 4.0,
                });
            }
            export_svg(&document(vec![layer]), &SvgOptions::default())
                .unwrap()
                .text
        };
        let round = stroked(LineCap::Round, LineJoin::Round);
        assert!(
            round.contains("stroke-linejoin=\"round\"")
                && round.contains("stroke-linecap=\"round\"")
        );
        let bevel = stroked(LineCap::Butt, LineJoin::Bevel);
        assert!(bevel.contains("stroke-linejoin=\"bevel\""));
        assert!(!bevel.contains("stroke-linecap"), "butt is SVG's default");
        // A miter at the default limit does not repeat the limit.
        let miter = stroked(LineCap::Butt, LineJoin::Miter);
        assert!(miter.contains("stroke-linejoin=\"miter\"") && !miter.contains("miterlimit"));
    }

    #[test]
    fn an_open_path_is_not_filled_as_the_renderer_does_not_fill_it() {
        let line = shape(
            "Line",
            ShapeGeometry::Line {
                x1: 0.0,
                y1: 0.0,
                x2: 50.0,
                y2: 50.0,
            },
        );
        let export = export_svg(&document(vec![line]), &SvgOptions::default()).unwrap();
        assert!(export
            .text
            .contains("<line x1=\"0\" y1=\"0\" x2=\"50\" y2=\"50\" fill=\"none\""));
    }

    #[test]
    fn polygons_and_stars_are_path_data_from_the_same_commands_the_renderer_flattens() {
        let polygon = shape(
            "Hex",
            ShapeGeometry::Polygon {
                cx: 50.0,
                cy: 50.0,
                radius: 20.0,
                sides: 6,
                rotation_degrees: 0.0,
            },
        );
        let export = export_svg(&document(vec![polygon]), &SvgOptions::default()).unwrap();
        assert!(export.text.contains("<path d=\"M"));
        assert!(export.text.contains("Z\""));
        assert_eq!(export.text.matches('L').count(), 5);
    }

    #[test]
    fn the_transform_is_about_the_canvas_centre_as_the_compositor_has_it() {
        let mut layer = shape("Moved", rect());
        layer.transform = LayerTransform {
            translate_x: 30.0,
            translate_y: -10.0,
            scale_x: 2.0,
            scale_y: 2.0,
            rotation_degrees: 90.0,
            ..LayerTransform::default()
        };
        let doc = document(vec![layer.clone()]);
        let [a, b, c, d, e, f] = matrix(&layer.transform, &doc);
        // The compositor's own forward map, for a point and for the centre.
        for point in [
            (10.0_f32, 20.0_f32),
            (100.0, 50.0),
            (0.0, 0.0),
            (200.0, 100.0),
        ] {
            let expected = layer.transform.forward(point, 200, 100);
            let got = (a * point.0 + c * point.1 + e, b * point.0 + d * point.1 + f);
            assert!(
                (expected.0 - got.0).abs() < 1e-3 && (expected.1 - got.1).abs() < 1e-3,
                "{point:?}: {expected:?} vs {got:?}"
            );
        }
        let export = export_svg(&doc, &SvgOptions::default()).unwrap();
        assert!(export.text.contains("transform=\"matrix("));
    }

    #[test]
    fn flips_are_in_the_matrix_too() {
        let transform = LayerTransform {
            flip_horizontal: true,
            rotation_degrees: 30.0,
            scale_x: 1.5,
            scale_y: 0.75,
            translate_x: 4.0,
            ..LayerTransform::default()
        };
        let doc = document(vec![]);
        let [a, b, c, d, e, f] = matrix(&transform, &doc);
        for point in [(3.0_f32, 7.0_f32), (150.0, 90.0)] {
            let expected = transform.forward(point, 200, 100);
            let got = (a * point.0 + c * point.1 + e, b * point.0 + d * point.1 + f);
            assert!((expected.0 - got.0).abs() < 1e-3 && (expected.1 - got.1).abs() < 1e-3);
        }
    }

    #[test]
    fn groups_opacity_blend_and_visibility_are_kept() {
        let mut inner = shape("Inner", rect());
        inner.blend_mode = BlendMode::Multiply;
        inner.visible = false;
        let mut group = new_layer(
            "g1",
            "Folder",
            LayerContent::Group {
                children: vec![inner],
                isolated: true,
            },
            "t",
        );
        group.opacity = 0.5;
        let export = export_svg(&document(vec![group]), &SvgOptions::default()).unwrap();
        assert!(export.text.contains("<g opacity=\"0.5\">"));
        assert!(export.text.contains("display=\"none\""));
        assert!(export.text.contains("mix-blend-mode:multiply"));
        assert_eq!(export.report.groups_written, 1);
        assert_eq!(export.report.layers_written, 1);
    }

    #[test]
    fn a_pass_through_group_that_is_not_plain_is_said_to_be_isolated() {
        let group = |opacity: f32, isolated: bool| {
            let mut layer = new_layer(
                "g",
                "Folder",
                LayerContent::Group {
                    children: vec![shape("Inner", rect())],
                    isolated,
                },
                "t",
            );
            layer.opacity = opacity;
            layer
        };
        let said = |layer: Layer| {
            export_svg(&document(vec![layer]), &SvgOptions::default())
                .unwrap()
                .report
                .entries
                .iter()
                .any(|e| e.kind == ReportKind::Approximated && e.detail.contains("pass-through"))
        };
        assert!(said(group(0.5, false)));
        // A plain pass-through group is the same thing in SVG, and isolated ones are exact.
        assert!(!said(group(1.0, false)));
        assert!(!said(group(0.5, true)));
    }

    #[test]
    fn what_an_svg_cannot_hold_is_left_out_and_named() {
        let pixels = new_layer(
            "p",
            "Photo",
            LayerContent::Pixel {
                pixel_id: "px".into(),
                width: 10,
                height: 10,
            },
            "t",
        );
        let mut odd = shape("Odd", rect());
        odd.blend_mode = BlendMode::VividLight;
        odd.clipped = true;
        let export = export_svg(
            &document(vec![pixels, odd]),
            &SvgOptions {
                operations: vec![crate::domain::EditOperation::Grayscale],
            },
        )
        .unwrap();
        let by_kind = |kind| {
            export
                .report
                .entries
                .iter()
                .filter(|e| e.kind == kind)
                .count()
        };
        // The pixel layer and the whole-document edit are left out; the blend mode and the
        // clipping are approximated.
        assert_eq!(by_kind(ReportKind::NotExported), 2);
        assert_eq!(by_kind(ReportKind::Approximated), 2);
        assert!(!export.text.contains("Photo"));
        assert!(export.text.contains("Odd"));
    }

    #[test]
    fn names_are_escaped_and_cannot_break_out_of_the_element() {
        let layer = shape("</title><script>alert(1)</script> & \"q\"", rect());
        let export = export_svg(&document(vec![layer]), &SvgOptions::default()).unwrap();
        assert!(!export.text.contains("<script>"));
        assert!(export.text.contains("&lt;/title&gt;&lt;script&gt;"));
        assert!(export.text.contains("&amp;"));
        assert!(export.text.contains("&quot;q&quot;"));
    }

    #[test]
    fn translucent_overlaps_are_reported_for_a_linear_document_only() {
        let mut half = shape("Half", rect());
        half.opacity = 0.5;
        let mut linear = document(vec![half.clone()]);
        linear.precision = crate::pixel::DocumentPrecision::LinearSrgbF32;
        let said =
            |export: &SvgExport| {
                export.report.entries.iter().any(|e| {
                    e.kind == ReportKind::Approximated && e.detail.contains("blends in sRGB")
                })
            };
        assert!(said(&export_svg(&linear, &SvgOptions::default()).unwrap()));
        // A legacy document composites in encoded sRGB, as an SVG does.
        let mut legacy = document(vec![half]);
        legacy.precision = crate::pixel::DocumentPrecision::LegacySrgb8;
        assert!(!said(&export_svg(&legacy, &SvgOptions::default()).unwrap()));
        // And a document of opaque shapes has nothing to say either way.
        let mut opaque = document(vec![shape("Box", rect())]);
        opaque.precision = crate::pixel::DocumentPrecision::LinearSrgbF32;
        assert!(!said(&export_svg(&opaque, &SvgOptions::default()).unwrap()));
    }

    #[test]
    fn a_document_with_no_shapes_is_an_empty_canvas_and_says_so() {
        let export = export_svg(&document(vec![]), &SvgOptions::default()).unwrap();
        assert_eq!(export.report.layers_written, 0);
        assert!(export
            .report
            .entries
            .iter()
            .any(|e| e.kind == ReportKind::Note && e.detail.contains("no shape layers")));
    }
}
