use super::bitmap::MaskBitmap;
use super::geometry::Point;
use crate::error::AppError;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Connectivity {
    #[default]
    Four,
    Eight,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WandOptions {
    pub tolerance: f32,
    pub connectivity: Connectivity,
    pub anti_alias: bool,
    pub contiguous: bool,
}

impl WandOptions {
    fn validate(self) -> Result<(), AppError> {
        if self.tolerance.is_finite() && (0.0..=1.0).contains(&self.tolerance) {
            Ok(())
        } else {
            Err(AppError::InvalidMask(
                "magic-wand tolerance must be between 0 and 1".into(),
            ))
        }
    }
}

pub fn select(
    image: &RgbaImage,
    point: Point,
    options: WandOptions,
    cancelled: Option<&AtomicBool>,
) -> Result<MaskBitmap, AppError> {
    options.validate()?;
    point.validate()?;
    if point.x < 0.0
        || point.y < 0.0
        || point.x >= image.width() as f32
        || point.y >= image.height() as f32
    {
        return Err(AppError::InvalidMask(
            "magic-wand sample is outside the image".into(),
        ));
    }
    let seed_x = point.x.floor() as u32;
    let seed_y = point.y.floor() as u32;
    let sample = *image.get_pixel(seed_x, seed_y);
    let mut mask = MaskBitmap::empty(image.width(), image.height())?;
    let soft_limit = (options.tolerance + if options.anti_alias { 0.08 } else { 0.0 }).min(1.0);

    if !options.contiguous {
        for (index, pixel) in image.pixels().enumerate() {
            if index % 4_096 == 0 {
                check_cancelled(cancelled)?;
            }
            mask.coverage_mut()[index] = coverage(
                color_distance(*pixel, sample),
                options.tolerance,
                soft_limit,
            );
        }
        return Ok(mask);
    }

    let width = image.width() as usize;
    let height = image.height() as usize;
    let mut visited = vec![false; width * height];
    let mut queue = VecDeque::from([(seed_x, seed_y)]);
    visited[seed_y as usize * width + seed_x as usize] = true;
    let mut processed = 0_usize;
    while let Some((x, y)) = queue.pop_front() {
        if processed % 4_096 == 0 {
            check_cancelled(cancelled)?;
        }
        processed += 1;
        let distance = color_distance(*image.get_pixel(x, y), sample);
        if distance > soft_limit {
            continue;
        }
        mask.set(x, y, coverage(distance, options.tolerance, soft_limit));
        if distance > options.tolerance {
            continue;
        }
        for (next_x, next_y) in neighbors(x, y, image.width(), image.height(), options.connectivity)
        {
            let index = next_y as usize * width + next_x as usize;
            if !visited[index] {
                visited[index] = true;
                queue.push_back((next_x, next_y));
            }
        }
    }
    Ok(mask)
}

fn coverage(distance: f32, tolerance: f32, soft_limit: f32) -> u8 {
    if distance <= tolerance || soft_limit <= tolerance {
        255
    } else if distance >= soft_limit {
        0
    } else {
        (255.0 * (1.0 - (distance - tolerance) / (soft_limit - tolerance))).round() as u8
    }
}

fn color_distance(left: image::Rgba<u8>, right: image::Rgba<u8>) -> f32 {
    let alpha = (left[3] as f32 - right[3] as f32).abs() / 255.0;
    if left[3] == 0 || right[3] == 0 {
        return alpha;
    }
    let red = (left[0] as f32 - right[0] as f32).abs() / 255.0;
    let green = (left[1] as f32 - right[1] as f32).abs() / 255.0;
    let blue = (left[2] as f32 - right[2] as f32).abs() / 255.0;
    red.max(green).max(blue).max(alpha)
}

fn neighbors(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    connectivity: Connectivity,
) -> Vec<(u32, u32)> {
    let mut result = Vec::with_capacity(if connectivity == Connectivity::Eight {
        8
    } else {
        4
    });
    for delta_y in -1_i32..=1 {
        for delta_x in -1_i32..=1 {
            if (delta_x == 0 && delta_y == 0)
                || (connectivity == Connectivity::Four && delta_x != 0 && delta_y != 0)
            {
                continue;
            }
            let next_x = x as i64 + i64::from(delta_x);
            let next_y = y as i64 + i64::from(delta_y);
            if next_x >= 0 && next_y >= 0 && next_x < i64::from(width) && next_y < i64::from(height)
            {
                result.push((next_x as u32, next_y as u32));
            }
        }
    }
    result
}

fn check_cancelled(cancelled: Option<&AtomicBool>) -> Result<(), AppError> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(AppError::MaskCancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn options(connectivity: Connectivity) -> WandOptions {
        WandOptions {
            tolerance: 0.0,
            connectivity,
            anti_alias: false,
            contiguous: true,
        }
    }

    #[test]
    fn four_and_eight_connected_regions_differ() {
        let mut image = RgbaImage::from_pixel(3, 3, Rgba([0, 0, 0, 255]));
        image.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        image.put_pixel(1, 1, Rgba([255, 255, 255, 255]));
        let point = Point { x: 0.0, y: 0.0 };
        let four = select(&image, point, options(Connectivity::Four), None).unwrap();
        let eight = select(&image, point, options(Connectivity::Eight), None).unwrap();
        assert_eq!(four.get(1, 1), 0);
        assert_eq!(eight.get(1, 1), 255);
    }

    #[test]
    fn transparent_pixels_compare_by_alpha() {
        let mut image = RgbaImage::from_pixel(2, 1, Rgba([255, 0, 0, 0]));
        image.put_pixel(1, 0, Rgba([255, 0, 0, 255]));
        let mask = select(
            &image,
            Point { x: 0.0, y: 0.0 },
            WandOptions {
                tolerance: 0.1,
                ..options(Connectivity::Four)
            },
            None,
        )
        .unwrap();
        assert_eq!(mask.get(0, 0), 255);
        assert_eq!(mask.get(1, 0), 0);
    }

    #[test]
    fn pathological_single_color_image_is_bounded_and_complete() {
        let image = RgbaImage::from_pixel(100, 100, Rgba([10, 20, 30, 255]));
        let mask = select(
            &image,
            Point { x: 50.0, y: 50.0 },
            options(Connectivity::Eight),
            None,
        )
        .unwrap();
        assert!(mask.coverage().iter().all(|value| *value == 255));
    }
}
