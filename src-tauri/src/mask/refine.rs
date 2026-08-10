use super::progress::MaskWorkContext;
use super::MaskBitmap;
use crate::error::AppError;
use image::RgbaImage;
use std::sync::atomic::AtomicBool;

pub fn align_to_image_edges(
    mask: &MaskBitmap,
    image: &RgbaImage,
    strength: f32,
    cancelled: Option<&AtomicBool>,
) -> Result<MaskBitmap, AppError> {
    align_to_image_edges_with_progress(
        mask,
        image,
        strength,
        MaskWorkContext::cancellation_only(cancelled),
    )
}

pub(crate) fn align_to_image_edges_with_progress(
    mask: &MaskBitmap,
    image: &RgbaImage,
    strength: f32,
    context: MaskWorkContext<'_>,
) -> Result<MaskBitmap, AppError> {
    if image.dimensions() != (mask.width(), mask.height()) {
        return Err(AppError::MaskDimensionMismatch {
            mask_width: mask.width(),
            mask_height: mask.height(),
            image_width: image.width(),
            image_height: image.height(),
        });
    }
    if !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
        return Err(AppError::InvalidMask(
            "edge-refinement strength must be between 0 and 1".into(),
        ));
    }
    if strength == 0.0 || mask.width() < 3 || mask.height() < 3 {
        let pixels = u64::from(mask.width()) * u64::from(mask.height());
        context.report("edge_alignment_pixels", pixels, pixels)?;
        return Ok(mask.clone());
    }

    let mut output = mask.clone();
    let interior_width = u64::from(mask.width() - 2);
    let pixels = interior_width * u64::from(mask.height() - 2);
    for y in 1..mask.height() - 1 {
        if y % 16 == 1 {
            context.report(
                "edge_alignment_pixels",
                u64::from(y - 1) * interior_width,
                pixels,
            )?;
        }
        for x in 1..mask.width() - 1 {
            if x % 4_096 == 1 {
                context.check_cancelled()?;
            }
            let value = mask.get(x, y);
            if !is_boundary(mask, x, y, value) {
                continue;
            }
            let gradient = sobel(image, x, y);
            let factor = 1.0 + strength * gradient * 3.0;
            output.set(
                x,
                y,
                ((value as f32 - 127.5) * factor + 127.5)
                    .round()
                    .clamp(0.0, 255.0) as u8,
            );
        }
    }
    context.report("edge_alignment_pixels", pixels, pixels)?;
    Ok(output)
}

fn is_boundary(mask: &MaskBitmap, x: u32, y: u32, value: u8) -> bool {
    if (1..=254).contains(&value) {
        return true;
    }
    let selected = value >= 128;
    [
        mask.get(x - 1, y),
        mask.get(x + 1, y),
        mask.get(x, y - 1),
        mask.get(x, y + 1),
    ]
    .iter()
    .any(|neighbor| (*neighbor >= 128) != selected)
}

fn sobel(image: &RgbaImage, x: u32, y: u32) -> f32 {
    let luminance = |sample_x: u32, sample_y: u32| {
        let pixel = image.get_pixel(sample_x, sample_y);
        if pixel[3] == 0 {
            0.0
        } else {
            (0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32)
                * pixel[3] as f32
                / 255.0
        }
    };
    let gx = -luminance(x - 1, y - 1) + luminance(x + 1, y - 1) - 2.0 * luminance(x - 1, y)
        + 2.0 * luminance(x + 1, y)
        - luminance(x - 1, y + 1)
        + luminance(x + 1, y + 1);
    let gy = -luminance(x - 1, y - 1) - 2.0 * luminance(x, y - 1) - luminance(x + 1, y - 1)
        + luminance(x - 1, y + 1)
        + 2.0 * luminance(x, y + 1)
        + luminance(x + 1, y + 1);
    gx.hypot(gy).min(1_020.0) / 1_020.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn aligned_high_contrast_edge_increases_mask_contrast() {
        let mut image = RgbaImage::from_pixel(5, 5, Rgba([0, 0, 0, 255]));
        for y in 0..5 {
            for x in 3..5 {
                image.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let mut mask = MaskBitmap::empty(5, 5).unwrap();
        for y in 0..5 {
            mask.set(1, y, 32);
            mask.set(2, y, 96);
            mask.set(3, y, 192);
            mask.set(4, y, 255);
        }
        let refined = align_to_image_edges(&mask, &image, 1.0, None).unwrap();
        assert!(refined.get(2, 2) < mask.get(2, 2));
        assert!(refined.get(3, 2) > mask.get(3, 2));
    }

    #[test]
    fn repeated_refinement_is_deterministic() {
        let image = RgbaImage::from_pixel(4, 4, Rgba([100, 120, 140, 255]));
        let mask = MaskBitmap::from_coverage(4, 4, [0, 64, 192, 255].repeat(4)).unwrap();
        assert_eq!(
            align_to_image_edges(&mask, &image, 0.7, None).unwrap(),
            align_to_image_edges(&mask, &image, 0.7, None).unwrap()
        );
    }

    #[test]
    fn edge_alignment_progress_uses_interior_pixel_units() {
        use std::sync::Mutex;

        let image = RgbaImage::from_pixel(10, 40, Rgba([100, 120, 140, 255]));
        let mask = MaskBitmap::full(10, 40).unwrap();
        let reports = Mutex::new(Vec::new());
        let callback = |phase: &str, completed: u64, total: u64| {
            reports
                .lock()
                .unwrap()
                .push((phase.to_owned(), completed, total));
            Ok(())
        };
        align_to_image_edges_with_progress(
            &mask,
            &image,
            1.0,
            MaskWorkContext::new(None, Some(&callback)),
        )
        .unwrap();
        let reports = reports.into_inner().unwrap();
        let expected = 8 * 38;
        assert!(reports.iter().all(|(_, _, total)| *total == expected));
        assert!(reports
            .iter()
            .any(|(_, completed, total)| *completed > 0 && *completed < *total));
        assert_eq!(reports.last().unwrap().1, expected);
    }
}
