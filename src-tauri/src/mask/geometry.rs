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

/// One fully resolved brush input sample.
///
/// Live hardware pressure is intentionally not part of this schema. Callers
/// resolve any optional input-device pressure into the effective diameter and
/// opacity before crossing the deterministic mask boundary.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedBrushSample {
    pub x: f32,
    pub y: f32,
    pub diameter: f32,
    pub opacity: f32,
}

impl ResolvedBrushSample {
    pub fn validate(self) -> Result<(), AppError> {
        Point {
            x: self.x,
            y: self.y,
        }
        .validate()?;
        if !self.diameter.is_finite()
            || !self.opacity.is_finite()
            || !(1.0..=2_048.0).contains(&self.diameter)
            || !(0.0..=1.0).contains(&self.opacity)
        {
            return Err(AppError::InvalidMask(
                "resolved brush diameter or opacity is invalid".into(),
            ));
        }
        Ok(())
    }

    pub fn point(self) -> Point {
        Point {
            x: self.x,
            y: self.y,
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
    ResolvedBrush {
        samples: Vec<ResolvedBrushSample>,
        hardness: f32,
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
            Self::ResolvedBrush { samples, hardness } => {
                if samples.is_empty() || samples.len() > MAX_PATH_POINTS {
                    return Err(AppError::InvalidMask(format!(
                        "resolved brush paths require 1 to {MAX_PATH_POINTS} samples"
                    )));
                }
                if !hardness.is_finite() || !(0.0..=1.0).contains(hardness) {
                    return Err(AppError::InvalidMask(
                        "resolved brush hardness is invalid".into(),
                    ));
                }
                samples.iter().try_for_each(|sample| sample.validate())?;
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

    #[test]
    fn resolved_brush_rejects_malformed_samples_and_raw_pressure() {
        let valid = ResolvedBrushSample {
            x: 1.0,
            y: 2.0,
            diameter: 24.0,
            opacity: 0.5,
        };
        for sample in [
            ResolvedBrushSample {
                x: f32::NAN,
                ..valid
            },
            ResolvedBrushSample {
                y: f32::INFINITY,
                ..valid
            },
            ResolvedBrushSample {
                diameter: 0.99,
                ..valid
            },
            ResolvedBrushSample {
                diameter: 2_048.01,
                ..valid
            },
            ResolvedBrushSample {
                opacity: -0.01,
                ..valid
            },
            ResolvedBrushSample {
                opacity: 1.01,
                ..valid
            },
        ] {
            assert!(sample.validate().is_err());
        }

        assert!(SelectionShape::ResolvedBrush {
            samples: Vec::new(),
            hardness: 0.5,
        }
        .validate()
        .is_err());
        assert!(SelectionShape::ResolvedBrush {
            samples: vec![valid],
            hardness: f32::NAN,
        }
        .validate()
        .is_err());
        assert!(SelectionShape::ResolvedBrush {
            samples: vec![valid; MAX_PATH_POINTS + 1],
            hardness: 0.5,
        }
        .validate()
        .is_err());

        let raw_pressure = r#"{
            "type":"resolved_brush",
            "samples":[{"x":1.0,"y":2.0,"diameter":24.0,"opacity":0.5,"pressure":0.7}],
            "hardness":0.5
        }"#;
        assert!(serde_json::from_str::<SelectionShape>(raw_pressure).is_err());
    }

    #[test]
    fn resolved_brush_round_trip_persists_only_effective_values() {
        let shape = SelectionShape::ResolvedBrush {
            samples: vec![ResolvedBrushSample {
                x: 4.0,
                y: 5.0,
                diameter: 12.0,
                opacity: 0.75,
            }],
            hardness: 0.6,
        };
        let json = serde_json::to_string(&shape).unwrap();
        assert!(json.contains("resolved_brush"));
        assert!(!json.contains("pressure"));
        assert_eq!(
            serde_json::from_str::<SelectionShape>(&json).unwrap(),
            shape
        );
    }
}
