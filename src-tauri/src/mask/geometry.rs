use crate::error::AppError;
use serde::{Deserialize, Serialize};

pub const MAX_PATH_POINTS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn validate(self) -> Result<(), AppError> {
        if self.x.is_finite() && self.y.is_finite() {
            Ok(())
        } else {
            Err(AppError::InvalidMask(
                "selection coordinates must be finite".into(),
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SelectionShape {
    Rectangle {
        start: Point,
        end: Point,
    },
    Ellipse {
        start: Point,
        end: Point,
    },
    Polygon {
        points: Vec<Point>,
    },
    Freehand {
        points: Vec<Point>,
    },
    Brush {
        points: Vec<Point>,
        diameter: f32,
        hardness: f32,
        opacity: f32,
    },
}

impl SelectionShape {
    pub fn validate(&self) -> Result<(), AppError> {
        match self {
            Self::Rectangle { start, end } | Self::Ellipse { start, end } => {
                start.validate()?;
                end.validate()?;
                if (start.x - end.x).abs() < f32::EPSILON || (start.y - end.y).abs() < f32::EPSILON
                {
                    return Err(AppError::InvalidMask(
                        "selection geometry must have a non-zero area".into(),
                    ));
                }
            }
            Self::Polygon { points } | Self::Freehand { points } => {
                validate_points(points, 3)?;
            }
            Self::Brush {
                points,
                diameter,
                hardness,
                opacity,
            } => {
                validate_points(points, 1)?;
                if !diameter.is_finite()
                    || !hardness.is_finite()
                    || !opacity.is_finite()
                    || !(1.0..=2_048.0).contains(diameter)
                    || !(0.0..=1.0).contains(hardness)
                    || !(0.0..=1.0).contains(opacity)
                {
                    return Err(AppError::InvalidMask(
                        "brush diameter, hardness, or opacity is invalid".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_points(points: &[Point], minimum: usize) -> Result<(), AppError> {
    if points.len() < minimum || points.len() > MAX_PATH_POINTS {
        return Err(AppError::InvalidMask(format!(
            "selection paths require {minimum} to {MAX_PATH_POINTS} points"
        )));
    }
    points.iter().try_for_each(|point| point.validate())
}

pub fn simplify_path(points: &[Point], tolerance: f32) -> Vec<Point> {
    if points.len() <= 2 || tolerance <= 0.0 {
        return points.to_vec();
    }
    let mut kept = vec![points[0]];
    let mut anchor = points[0];
    let tolerance_squared = tolerance * tolerance;
    for point in &points[1..points.len() - 1] {
        let dx = point.x - anchor.x;
        let dy = point.y - anchor.y;
        if dx * dx + dy * dy >= tolerance_squared {
            kept.push(*point);
            anchor = *point;
        }
    }
    kept.push(points[points.len() - 1]);
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_nan_and_oversized_paths() {
        assert!(Point {
            x: f32::NAN,
            y: 1.0
        }
        .validate()
        .is_err());
        let points = vec![Point { x: 0.0, y: 0.0 }; MAX_PATH_POINTS + 1];
        assert!(SelectionShape::Polygon { points }.validate().is_err());
    }

    #[test]
    fn simplification_keeps_endpoints() {
        let points = [
            Point { x: 0.0, y: 0.0 },
            Point { x: 0.1, y: 0.1 },
            Point { x: 5.0, y: 5.0 },
        ];
        assert_eq!(simplify_path(&points, 1.0), vec![points[0], points[2]]);
    }
}
