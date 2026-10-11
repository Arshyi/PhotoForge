//! Colour primaries: taking an OpenEXR file's RGB to PhotoForge's working space.
//!
//! An OpenEXR file is scene-linear, and says which primaries and white point its RGB means in an
//! optional `chromaticities` attribute (absent: Rec. 709, which is also sRGB's). PhotoForge works
//! in linear light on sRGB primaries, so a file in another space, ACEScg being the common one, is
//! carried across with a matrix, and its white point adapted to D65 with the Bradford transform.
//! Without that an ACEScg render would open as it would on a monitor that mistook it for sRGB:
//! the right shape and the wrong, over-saturated colour.
//!
//! The matrix is applied to straight-alpha colour. Values outside sRGB's gamut come out negative
//! or above one, which the high-precision pipeline holds and only the final encode clips.

/// A white point and three primaries, as CIE xy chromaticity coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Primaries {
    pub red: (f32, f32),
    pub green: (f32, f32),
    pub blue: (f32, f32),
    pub white: (f32, f32),
}

/// Rec. ITU-R BT.709-3, which is what an OpenEXR file with no chromaticities means, and sRGB's.
pub const REC709: Primaries = Primaries {
    red: (0.6400, 0.3300),
    green: (0.3000, 0.6000),
    blue: (0.1500, 0.0600),
    white: (0.3127, 0.3290),
};

type Matrix = [[f64; 3]; 3];

/// How far a coordinate may be from Rec. 709's and still be called Rec. 709. Writers round these
/// to four decimals, and some use 0.3127 / 0.329 and others 0.31271 / 0.32902 for D65.
const SAME_TOLERANCE: f32 = 1.5e-3;

impl Primaries {
    /// Whether these are, to the precision files are written with, Rec. 709's.
    pub fn is_rec709(&self) -> bool {
        let close = |a: (f32, f32), b: (f32, f32)| {
            (a.0 - b.0).abs() <= SAME_TOLERANCE && (a.1 - b.1).abs() <= SAME_TOLERANCE
        };
        close(self.red, REC709.red)
            && close(self.green, REC709.green)
            && close(self.blue, REC709.blue)
            && close(self.white, REC709.white)
    }

    /// Real numbers of a sane size, and a white point with some luminance. (An ACES AP0 blue has
    /// a negative y and is a legitimate primary; a degenerate triangle is caught when the matrix
    /// is inverted.)
    fn is_usable(&self) -> bool {
        let sane =
            |(x, y): (f32, f32)| x.is_finite() && y.is_finite() && x.abs() < 8.0 && y.abs() < 8.0;
        sane(self.red)
            && sane(self.green)
            && sane(self.blue)
            && sane(self.white)
            && self.white.1 > 1e-4
    }

    /// The matrix taking linear RGB in these primaries to XYZ with Y = 1 for white.
    fn rgb_to_xyz(&self) -> Option<Matrix> {
        // A primary is the vector (x, y, z) of its chromaticity, not normalised to Y = 1, so that
        // one with a negative y still has a column. The white point has Y = 1.
        let primary = |(x, y): (f32, f32)| {
            let (x, y) = (f64::from(x), f64::from(y));
            [x, y, 1.0 - x - y]
        };
        let (white_x, white_y) = (f64::from(self.white.0), f64::from(self.white.1));
        let (r, g, b, w) = (
            primary(self.red),
            primary(self.green),
            primary(self.blue),
            [white_x / white_y, 1.0, (1.0 - white_x - white_y) / white_y],
        );
        // [r g b] * s = w: the scale of each primary that makes RGB (1, 1, 1) the white point.
        let primaries: Matrix = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
        let inverse = invert(&primaries)?;
        let s = multiply_vector(&inverse, &w);
        Some([
            [r[0] * s[0], g[0] * s[1], b[0] * s[2]],
            [r[1] * s[0], g[1] * s[1], b[1] * s[2]],
            [r[2] * s[0], g[2] * s[1], b[2] * s[2]],
        ])
    }

    /// The matrix that takes this file's RGB to PhotoForge's linear sRGB, with the white point
    /// adapted (Bradford) to sRGB's. `None` when the primaries cannot be a real colour space (a
    /// degenerate triangle, or coordinates outside the chromaticity diagram).
    pub fn to_working_space(&self) -> Option<[[f32; 3]; 3]> {
        if !self.is_usable() {
            return None;
        }
        let source = self.rgb_to_xyz()?;
        let target = REC709.rgb_to_xyz()?;
        let adaptation = bradford(self.white, REC709.white)?;
        let target_inverse = invert(&target)?;
        let full = multiply(&target_inverse, &multiply(&adaptation, &source));
        let mut out = [[0.0_f32; 3]; 3];
        for (row, values) in full.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                if !value.is_finite() {
                    return None;
                }
                out[row][column] = *value as f32;
            }
        }
        Some(out)
    }
}

/// The Bradford chromatic adaptation taking white `from` to white `to` (XYZ, Y = 1).
fn bradford(from: (f32, f32), to: (f32, f32)) -> Option<Matrix> {
    const CONE: Matrix = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let white = |(x, y): (f32, f32)| {
        let (x, y) = (f64::from(x), f64::from(y));
        [x / y, 1.0, (1.0 - x - y) / y]
    };
    let source = multiply_vector(&CONE, &white(from));
    let target = multiply_vector(&CONE, &white(to));
    if source.iter().any(|v| v.abs() < 1e-9) {
        return None;
    }
    let scale: Matrix = [
        [target[0] / source[0], 0.0, 0.0],
        [0.0, target[1] / source[1], 0.0],
        [0.0, 0.0, target[2] / source[2]],
    ];
    let inverse = invert(&CONE)?;
    Some(multiply(&inverse, &multiply(&scale, &CONE)))
}

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0; 3]; 3];
    for (row, out_row) in out.iter_mut().enumerate() {
        for (column, cell) in out_row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[row][k] * b[k][column]).sum();
        }
    }
    out
}

fn multiply_vector(m: &Matrix, v: &[f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn invert(m: &Matrix) -> Option<Matrix> {
    let determinant = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if !determinant.is_finite() || determinant.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / determinant;
    Some([
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv,
        ],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ACES AP1 ("ACEScg"), as the ACES documents publish it.
    const AP1: Primaries = Primaries {
        red: (0.713, 0.293),
        green: (0.165, 0.830),
        blue: (0.128, 0.044),
        white: (0.32168, 0.33767),
    };

    /// Rec. 2020's primaries with D65.
    const REC2020: Primaries = Primaries {
        red: (0.708, 0.292),
        green: (0.170, 0.797),
        blue: (0.131, 0.046),
        white: (0.3127, 0.3290),
    };

    #[test]
    fn rec709_is_the_identity() {
        let matrix = REC709.to_working_space().unwrap();
        for (row, values) in matrix.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                let expected = if row == column { 1.0 } else { 0.0 };
                assert!((value - expected).abs() < 1e-5, "{row},{column}: {value}");
            }
        }
        assert!(REC709.is_rec709());
    }

    #[test]
    fn acescg_matches_the_published_matrix_to_srgb() {
        // ACEScg to linear sRGB with Bradford adaptation, as colour-science and the ACES
        // documentation list it.
        let published = [
            [1.70505, -0.62179, -0.08326],
            [-0.13026, 1.14080, -0.01055],
            [-0.02400, -0.12897, 1.15297],
        ];
        let matrix = AP1.to_working_space().unwrap();
        for row in 0..3 {
            for column in 0..3 {
                assert!(
                    (matrix[row][column] - published[row][column]).abs() < 3e-3,
                    "{row},{column}: {} against {}",
                    matrix[row][column],
                    published[row][column]
                );
            }
        }
        assert!(!AP1.is_rec709());
    }

    #[test]
    fn white_stays_white_in_every_space() {
        // Whatever the primaries, RGB (1, 1, 1) is the file's white point, which is adapted to
        // sRGB's: so it must come out (1, 1, 1), and every row sums to one.
        for primaries in [AP1, REC2020, REC709] {
            let matrix = primaries.to_working_space().unwrap();
            for row in matrix {
                let sum: f32 = row.iter().sum();
                assert!(
                    (sum - 1.0).abs() < 1e-4,
                    "{primaries:?}: {row:?} sums to {sum}"
                );
            }
        }
    }

    #[test]
    fn rec2020_matches_the_known_matrix() {
        // Rec. 2020 to Rec. 709 linear light (BT.2087).
        let published = [
            [1.6605, -0.5876, -0.0728],
            [-0.1246, 1.1329, -0.0083],
            [-0.0182, -0.1006, 1.1187],
        ];
        let matrix = REC2020.to_working_space().unwrap();
        for row in 0..3 {
            for column in 0..3 {
                assert!(
                    (matrix[row][column] - published[row][column]).abs() < 2e-3,
                    "{row},{column}: {} against {}",
                    matrix[row][column],
                    published[row][column]
                );
            }
        }
    }

    #[test]
    fn aces_ap0_with_its_negative_blue_is_carried_across() {
        // ACES 2065-1 (AP0) to linear sRGB with Bradford adaptation, as colour-science lists it.
        const AP0: Primaries = Primaries {
            red: (0.7347, 0.2653),
            green: (0.0, 1.0),
            blue: (0.0001, -0.0770),
            white: (0.32168, 0.33767),
        };
        let published = [
            [2.52169, -1.13413, -0.38756],
            [-0.27648, 1.37272, -0.09624],
            [-0.01538, -0.15297, 1.16835],
        ];
        let matrix = AP0.to_working_space().unwrap();
        for row in 0..3 {
            for column in 0..3 {
                assert!(
                    (matrix[row][column] - published[row][column]).abs() < 5e-3,
                    "{row},{column}: {} against {}",
                    matrix[row][column],
                    published[row][column]
                );
            }
        }
    }

    #[test]
    fn rounding_in_the_file_is_not_a_different_space() {
        // D65 as another writer rounds it.
        let rounded = Primaries {
            red: (0.6401, 0.3300),
            green: (0.3000, 0.6001),
            blue: (0.1500, 0.0600),
            white: (0.31271, 0.32902),
        };
        assert!(rounded.is_rec709());
        let other = Primaries {
            white: (0.3457, 0.3585),
            ..REC709
        };
        assert!(!other.is_rec709());
    }

    #[test]
    fn nonsense_primaries_are_refused_not_divided_by() {
        let flat = Primaries {
            red: (0.3, 0.3),
            green: (0.3, 0.3),
            blue: (0.3, 0.3),
            white: (0.3127, 0.3290),
        };
        assert!(flat.to_working_space().is_none());
        let huge = Primaries {
            blue: (1.0e6, 0.5),
            ..REC709
        };
        assert!(huge.to_working_space().is_none());
        let black_white_point = Primaries {
            white: (0.3127, 0.0),
            ..REC709
        };
        assert!(black_white_point.to_working_space().is_none());
        let not_a_number = Primaries {
            green: (f32::NAN, 0.6),
            ..REC709
        };
        assert!(not_a_number.to_working_space().is_none());
    }
}
