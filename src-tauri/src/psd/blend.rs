//! Photoshop blend mode keys, and what PhotoForge can do with each.
use crate::layers::BlendMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendKey {
    /// A mode PhotoForge has, with the same definition.
    Supported(BlendMode),
    /// `pass`: a group that passes its backdrop through (only meaningful on a group).
    PassThrough,
    /// A mode PhotoForge does not have. The import falls back to Normal and says so.
    Unsupported(&'static str),
    /// A key this table does not know.
    Unknown,
}

pub fn blend_from_key(key: &[u8; 4]) -> BlendKey {
    use BlendKey::{PassThrough, Supported, Unsupported};
    match key {
        b"norm" => Supported(BlendMode::Normal),
        b"pass" => PassThrough,
        b"dark" => Supported(BlendMode::Darken),
        b"mul " => Supported(BlendMode::Multiply),
        b"idiv" => Supported(BlendMode::ColorBurn),
        b"lite" => Supported(BlendMode::Lighten),
        b"scrn" => Supported(BlendMode::Screen),
        b"div " => Supported(BlendMode::ColorDodge),
        b"over" => Supported(BlendMode::Overlay),
        b"sLit" => Supported(BlendMode::SoftLight),
        b"hLit" => Supported(BlendMode::HardLight),
        b"diff" => Supported(BlendMode::Difference),
        b"smud" => Supported(BlendMode::Exclusion),
        b"hue " => Supported(BlendMode::Hue),
        b"sat " => Supported(BlendMode::Saturation),
        b"colr" => Supported(BlendMode::Color),
        b"lum " => Supported(BlendMode::Luminosity),
        b"lbrn" => Supported(BlendMode::LinearBurn),
        b"lddg" => Supported(BlendMode::LinearDodge),
        b"dkCl" => Supported(BlendMode::DarkerColor),
        b"lgCl" => Supported(BlendMode::LighterColor),
        b"vLit" => Supported(BlendMode::VividLight),
        b"lLit" => Supported(BlendMode::LinearLight),
        b"pLit" => Supported(BlendMode::PinLight),
        b"hMix" => Supported(BlendMode::HardMix),
        b"fsub" => Supported(BlendMode::Subtract),
        b"fdiv" => Supported(BlendMode::Divide),
        // A random dither whose pattern cannot be reproduced.
        b"diss" => Unsupported("Dissolve"),
        _ => BlendKey::Unknown,
    }
}

/// A human name for an additional-information key, for the import report. Keys this table
/// does not list are reported by their four characters.
pub fn describe_block_key(key: &[u8; 4]) -> Option<&'static str> {
    Some(match key {
        b"TySh" | b"tySh" => "editable text",
        b"SoLd" | b"SoLE" | b"PlLd" | b"plLd" => "smart object",
        b"lfx2" | b"lrFX" | b"lmfx" => "layer effects (styles)",
        b"vmsk" | b"vsms" => "vector mask",
        b"vscg" | b"SoCo" | b"GdFl" | b"PtFl" => "fill layer",
        b"brit" | b"levl" | b"curv" | b"hue2" | b"hue " | b"blnc" | b"selc" | b"mixr" | b"grdm"
        | b"phfl" | b"posz" | b"thrs" | b"nvrt" | b"CgEd" | b"CGEd" => "adjustment layer",
        b"clbl" | b"infx" | b"knko" => "blending options",
        b"artb" | b"artd" | b"abdd" => "artboard",
        b"lnkD" | b"lnk2" | b"lnk3" | b"lnkE" => "linked file",
        b"vogk" | b"vstk" => "vector stroke / fill",
        b"PxSD" => "pixel source data",
        _ => return None,
    })
}
