//! The eight EXIF orientations: where a pixel of a picture that is stored sideways goes when the
//! picture is turned upright.
//!
//! One definition for everything that applies the tag, so a TIFF opened whole and a camera RAW
//! developed whole are turned by the same rule, which was checked against Pillow's
//! `exif_transpose` (`docs/tiff.md`).
//!
//! | Value | Stored as | Turned by |
//! | --- | --- | --- |
//! | 1 | upright | nothing |
//! | 2 | mirrored left to right | mirror left to right |
//! | 3 | upside down | half a turn |
//! | 4 | mirrored top to bottom | mirror top to bottom |
//! | 5 | transposed (mirrored along the main diagonal) | transpose |
//! | 6 | a quarter turn clockwise needed | quarter turn clockwise |
//! | 7 | transposed the other way (the anti-diagonal) | transverse |
//! | 8 | a quarter turn counter-clockwise needed | quarter turn counter-clockwise |
//!
//! Values 5 to 8 exchange the picture's width and height.
use std::collections::TryReserveError;

/// Whether `orientation` is one of the eight values.
pub const fn is_valid(orientation: u8) -> bool {
    matches!(orientation, 1..=8)
}

/// Whether turning the picture exchanges its width and height.
pub const fn swaps_axes(orientation: u8) -> bool {
    matches!(orientation, 5..=8)
}

/// The size of a `width` × `height` picture once it is turned upright.
pub const fn oriented_size(orientation: u8, width: u32, height: u32) -> (u32, u32) {
    if swaps_axes(orientation) {
        (height, width)
    } else {
        (width, height)
    }
}

/// Where the pixel at (`x`, `y`) of a `width` × `height` picture lands in the upright picture.
/// `x` is below `width` and `y` below `height`.
pub const fn place(orientation: u8, width: u32, height: u32, x: u32, y: u32) -> (u32, u32) {
    match orientation {
        2 => (width - 1 - x, y),
        3 => (width - 1 - x, height - 1 - y),
        4 => (x, height - 1 - y),
        5 => (y, x),
        6 => (height - 1 - y, x),
        7 => (height - 1 - y, width - 1 - x),
        8 => (y, width - 1 - x),
        _ => (x, y),
    }
}

/// Turns row-major `pixels` of a `width` × `height` picture upright. The mirrors and the half
/// turn are made in place; the four that exchange the axes need a second buffer (the first is
/// dropped as soon as it has been read), so a caller that must stay inside a memory budget
/// counts the picture twice for those.
///
/// An orientation outside 1 to 8, or a pixel count that is not `width × height`, leaves the
/// pixels as they are.
pub fn turn<T: Copy>(
    mut pixels: Vec<T>,
    orientation: u8,
    width: u32,
    height: u32,
) -> Result<Vec<T>, TryReserveError> {
    let (w, h) = (width as usize, height as usize);
    if !is_valid(orientation) || orientation == 1 || w == 0 || h == 0 || pixels.len() != w * h {
        return Ok(pixels);
    }
    match orientation {
        2 => {
            for row in pixels.chunks_exact_mut(w) {
                row.reverse();
            }
        }
        3 => pixels.reverse(),
        4 => {
            for y in 0..h / 2 {
                for x in 0..w {
                    pixels.swap(y * w + x, (h - 1 - y) * w + x);
                }
            }
        }
        _ => {
            let mut turned: Vec<T> = Vec::new();
            turned.try_reserve_exact(pixels.len())?;
            turned.resize(pixels.len(), pixels[0]);
            // The turned picture is `height` wide: a row of it is a column of the original.
            for y in 0..height {
                for x in 0..width {
                    let (dx, dy) = place(orientation, width, height, x, y);
                    turned[dy as usize * h + dx as usize] = pixels[y as usize * w + x as usize];
                }
            }
            return Ok(turned);
        }
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 2 3 / 4 5 6.
    fn picture() -> Vec<u32> {
        (1..=6).collect()
    }

    #[test]
    fn each_orientation_turns_a_three_by_two_picture_as_the_table_says() {
        // (orientation, width, height, rows of the upright picture)
        let expected: [(u8, u32, u32, &[&[u32]]); 8] = [
            (1, 3, 2, &[&[1, 2, 3], &[4, 5, 6]]),
            (2, 3, 2, &[&[3, 2, 1], &[6, 5, 4]]),
            (3, 3, 2, &[&[6, 5, 4], &[3, 2, 1]]),
            (4, 3, 2, &[&[4, 5, 6], &[1, 2, 3]]),
            (5, 2, 3, &[&[1, 4], &[2, 5], &[3, 6]]),
            (6, 2, 3, &[&[4, 1], &[5, 2], &[6, 3]]),
            (7, 2, 3, &[&[6, 3], &[5, 2], &[4, 1]]),
            (8, 2, 3, &[&[3, 6], &[2, 5], &[1, 4]]),
        ];
        for (orientation, width, height, rows) in expected {
            assert_eq!(oriented_size(orientation, 3, 2), (width, height));
            let turned = turn(picture(), orientation, 3, 2).unwrap();
            let want: Vec<u32> = rows.iter().flat_map(|row| row.iter().copied()).collect();
            assert_eq!(turned, want, "orientation {orientation}");
            // And `place` says the same of each pixel.
            for y in 0..2_u32 {
                for x in 0..3_u32 {
                    let (dx, dy) = place(orientation, 3, 2, x, y);
                    assert_eq!(
                        turned[(dy * width + dx) as usize],
                        picture()[(y * 3 + x) as usize],
                        "orientation {orientation} ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn opposite_quarter_turns_cancel_and_two_half_turns_are_nothing() {
        let (w, h) = (7_u32, 4_u32);
        let original: Vec<u32> = (0..w * h).collect();
        let there = turn(original.clone(), 6, w, h).unwrap();
        let back = turn(there, 8, h, w).unwrap();
        assert_eq!(back, original);
        let once = turn(original.clone(), 3, w, h).unwrap();
        assert_ne!(once, original);
        assert_eq!(turn(once, 3, w, h).unwrap(), original);
        // A mirror twice is nothing, and so is a transpose twice.
        for orientation in [2_u8, 4] {
            let again = turn(
                turn(original.clone(), orientation, w, h).unwrap(),
                orientation,
                w,
                h,
            )
            .unwrap();
            assert_eq!(again, original, "orientation {orientation}");
        }
        let transposed = turn(original.clone(), 5, w, h).unwrap();
        assert_eq!(turn(transposed, 5, h, w).unwrap(), original);
    }

    #[test]
    fn a_picture_that_cannot_be_turned_is_left_alone() {
        assert_eq!(turn(vec![1_u8, 2, 3], 6, 3, 2).unwrap(), vec![1, 2, 3]);
        assert_eq!(
            turn(vec![1_u8, 2, 3, 4], 0, 2, 2).unwrap(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(
            turn(vec![1_u8, 2, 3, 4], 9, 2, 2).unwrap(),
            vec![1, 2, 3, 4]
        );
        assert!(turn(Vec::<u8>::new(), 6, 0, 0).unwrap().is_empty());
    }

    #[test]
    fn a_one_pixel_wide_picture_turns() {
        // 1 / 2 / 3 turned a quarter clockwise is 3 2 1.
        assert_eq!(turn(vec![1_u8, 2, 3], 6, 1, 3).unwrap(), vec![3, 2, 1]);
        assert_eq!(turn(vec![1_u8, 2, 3], 8, 1, 3).unwrap(), vec![1, 2, 3]);
    }
}
