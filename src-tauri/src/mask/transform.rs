use super::bitmap::{checked_length, MaskBitmap};
use super::progress::MaskWorkContext;
use crate::domain::PerspectiveCorners;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

pub const MAX_GEOMETRY_STEPS: usize = 200;

const MIN_PERSPECTIVE_JACOBIAN: f64 = 1.0e-6;
const MIN_LENS_JACOBIAN: f64 = 1.0e-6;
const INVERSE_RESIDUAL_TOLERANCE: f64 = 1.0e-8;
const LENS_INVERSE_RESIDUAL_TOLERANCE: f64 = 1.0e-12;
const INVERSE_DOMAIN_TOLERANCE: f64 = 1.0e-7;
const MAX_INVERSE_ITERATIONS: usize = 20;
const MAX_LENS_INVERSE_ITERATIONS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GeometryStep {
    Crop {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    Rotate {
        degrees: i32,
    },
    ReflectHorizontal,
    Straighten {
        degrees: f32,
    },
    Perspective {
        corners: PerspectiveCorners,
    },
    /// Spatially follows the center (green/alpha) sample used by the image lens operation.
    /// Vignetting is intensity-only. Chromatic aberration has distinct red/blue offsets that a
    /// single-channel mask cannot represent, so those values participate in serialization and
    /// workflow identity but do not alter the mask coordinate map.
    LensCorrection {
        distortion: f32,
        vignetting: f32,
        chromatic_aberration: f32,
    },
}

#[derive(Debug, Clone)]
pub struct GeometryChain {
    original: Dimensions,
    steps: Vec<GeometryStep>,
    dimensions: Vec<Dimensions>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Dimensions {
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy)]
struct Coordinate {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Copy)]
struct CropBounds {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy)]
struct BilinearMap {
    origin: Coordinate,
    horizontal: Coordinate,
    vertical: Coordinate,
    cross: Coordinate,
}

#[derive(Debug, Clone, Copy)]
struct LensMap {
    center: Coordinate,
    pixel_scale: Coordinate,
    distortion: f64,
    active_x: bool,
    active_y: bool,
}

impl GeometryChain {
    pub fn new(width: u32, height: u32, steps: Vec<GeometryStep>) -> Result<Self, AppError> {
        if steps.len() > MAX_GEOMETRY_STEPS {
            return Err(AppError::InvalidMask(format!(
                "mask geometry chains may contain at most {MAX_GEOMETRY_STEPS} steps"
            )));
        }
        checked_length(width, height)?;
        let original = Dimensions { width, height };
        let mut dimensions = Vec::with_capacity(steps.len() + 1);
        dimensions.push(original);
        let mut current = original;
        for step in &steps {
            current = step.output_dimensions(current)?;
            checked_length(current.width, current.height)?;
            dimensions.push(current);
        }
        Ok(Self {
            original,
            steps,
            dimensions,
        })
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn dimensions_at(&self, stage: usize) -> Result<(u32, u32), AppError> {
        self.dimensions
            .get(stage)
            .map(|dimensions| (dimensions.width, dimensions.height))
            .ok_or_else(|| {
                AppError::InvalidMask(format!(
                    "mask geometry stage {stage} exceeds the {}-step chain",
                    self.steps.len()
                ))
            })
    }

    pub(crate) fn remap_work_units(
        &self,
        old_stage: usize,
        new_chain: &Self,
        new_stage: usize,
    ) -> Result<u64, AppError> {
        self.dimensions_value(old_stage)?;
        let target = new_chain.dimensions_value(new_stage)?;
        if old_stage <= new_stage
            && spatially_equal(&self.steps[..old_stage], &new_chain.steps[..old_stage])
            && new_chain.steps[old_stage..new_stage]
                .iter()
                .all(|step| step.has_exact_mask_mapping())
        {
            Ok((old_stage..new_stage)
                .map(|index| match new_chain.steps[index] {
                    GeometryStep::Crop { .. } => u64::from(new_chain.dimensions[index + 1].height),
                    GeometryStep::Rotate { .. } | GeometryStep::ReflectHorizontal => {
                        u64::from(new_chain.dimensions[index].height)
                    }
                    GeometryStep::LensCorrection { .. } => {
                        u64::from(new_chain.dimensions[index].height)
                    }
                    GeometryStep::Straighten { .. } | GeometryStep::Perspective { .. } => 0,
                })
                .sum())
        } else {
            Ok(u64::from(target.height))
        }
    }

    fn dimensions_value(&self, stage: usize) -> Result<Dimensions, AppError> {
        self.dimensions.get(stage).copied().ok_or_else(|| {
            AppError::InvalidMask(format!(
                "mask geometry stage {stage} exceeds the {}-step chain",
                self.steps.len()
            ))
        })
    }

    fn output_to_original(&self, stage: usize, mut point: Coordinate) -> Option<Coordinate> {
        if stage > self.steps.len() {
            return None;
        }
        for index in (0..stage).rev() {
            let input = self.dimensions[index];
            let output = self.dimensions[index + 1];
            point = self.steps[index].output_to_input(point, input, output)?;
            if !contains(point, input) {
                return None;
            }
        }
        contains(point, self.original).then_some(point)
    }

    fn original_to_output(&self, stage: usize, mut point: Coordinate) -> Option<Coordinate> {
        if stage > self.steps.len() || !contains(point, self.original) {
            return None;
        }
        for index in 0..stage {
            let input = self.dimensions[index];
            let output = self.dimensions[index + 1];
            point = self.steps[index].input_to_output(point, input, output)?;
            point = clamp_domain(point, output)?;
        }
        Some(point)
    }
}

impl GeometryStep {
    fn output_dimensions(self, input: Dimensions) -> Result<Dimensions, AppError> {
        match self {
            Self::Crop {
                x,
                y,
                width,
                height,
            } => {
                let bounds = crop_bounds(input, x, y, width, height)?;
                Ok(Dimensions {
                    width: bounds.width,
                    height: bounds.height,
                })
            }
            Self::Rotate { degrees } => match degrees.rem_euclid(360) {
                0 | 180 => Ok(input),
                90 | 270 => Ok(Dimensions {
                    width: input.height,
                    height: input.width,
                }),
                _ => Err(AppError::InvalidMask(
                    "mask rotation must be a multiple of 90 degrees".into(),
                )),
            },
            Self::ReflectHorizontal => Ok(input),
            Self::Straighten { degrees } => {
                if degrees.is_finite() && (-45.0..=45.0).contains(&degrees) {
                    Ok(input)
                } else {
                    Err(AppError::InvalidMask(
                        "mask straighten angle must be finite and between -45 and 45 degrees"
                            .into(),
                    ))
                }
            }
            Self::Perspective { corners } => {
                validate_perspective(&corners)?;
                Ok(input)
            }
            Self::LensCorrection {
                distortion,
                vignetting,
                chromatic_aberration,
            } => {
                validate_lens_correction(input, distortion, vignetting, chromatic_aberration)?;
                Ok(input)
            }
        }
    }

    fn spatially_equivalent(self, other: Self) -> bool {
        match (self, other) {
            (
                Self::LensCorrection {
                    distortion: left, ..
                },
                Self::LensCorrection {
                    distortion: right, ..
                },
            ) => left == right,
            _ => self == other,
        }
    }

    fn has_exact_mask_mapping(self) -> bool {
        matches!(
            self,
            Self::Crop { .. } | Self::Rotate { .. } | Self::ReflectHorizontal
        ) || matches!(
            self,
            Self::LensCorrection {
                distortion: 0.0,
                ..
            }
        )
    }

    fn output_to_input(
        self,
        point: Coordinate,
        input: Dimensions,
        _output: Dimensions,
    ) -> Option<Coordinate> {
        match self {
            Self::Crop {
                x,
                y,
                width,
                height,
            } => {
                let bounds = crop_bounds(input, x, y, width, height).ok()?;
                Some(Coordinate {
                    x: point.x + f64::from(bounds.left),
                    y: point.y + f64::from(bounds.top),
                })
            }
            Self::Rotate { degrees } => match degrees.rem_euclid(360) {
                0 => Some(point),
                90 => Some(Coordinate {
                    x: point.y,
                    y: f64::from(input.height - 1) - point.x,
                }),
                180 => Some(Coordinate {
                    x: f64::from(input.width - 1) - point.x,
                    y: f64::from(input.height - 1) - point.y,
                }),
                270 => Some(Coordinate {
                    x: f64::from(input.width - 1) - point.y,
                    y: point.x,
                }),
                _ => None,
            },
            Self::ReflectHorizontal => Some(Coordinate {
                x: f64::from(input.width - 1) - point.x,
                y: point.y,
            }),
            Self::Straighten { degrees } => {
                let radians = f64::from(degrees).to_radians();
                let (sin, cos) = radians.sin_cos();
                let center_x = (f64::from(input.width) - 1.0) * 0.5;
                let center_y = (f64::from(input.height) - 1.0) * 0.5;
                let dx = point.x - center_x;
                let dy = point.y - center_y;
                Some(Coordinate {
                    x: center_x + dx * cos + dy * sin,
                    y: center_y - dx * sin + dy * cos,
                })
            }
            Self::Perspective { corners } => {
                let u = normalize(point.x, input.width);
                let v = normalize(point.y, input.height);
                let mapped = BilinearMap::new(&corners).evaluate(u, v);
                Some(Coordinate {
                    x: denormalize(mapped.x, input.width),
                    y: denormalize(mapped.y, input.height),
                })
            }
            Self::LensCorrection { distortion, .. } => {
                Some(LensMap::new(input, distortion).evaluate(point))
            }
        }
    }

    fn input_to_output(
        self,
        point: Coordinate,
        input: Dimensions,
        output: Dimensions,
    ) -> Option<Coordinate> {
        match self {
            Self::Crop {
                x,
                y,
                width,
                height,
            } => {
                let bounds = crop_bounds(input, x, y, width, height).ok()?;
                Some(Coordinate {
                    x: point.x - f64::from(bounds.left),
                    y: point.y - f64::from(bounds.top),
                })
            }
            Self::Rotate { degrees } => match degrees.rem_euclid(360) {
                0 => Some(point),
                90 => Some(Coordinate {
                    x: f64::from(input.height - 1) - point.y,
                    y: point.x,
                }),
                180 => Some(Coordinate {
                    x: f64::from(input.width - 1) - point.x,
                    y: f64::from(input.height - 1) - point.y,
                }),
                270 => Some(Coordinate {
                    x: point.y,
                    y: f64::from(input.width - 1) - point.x,
                }),
                _ => None,
            },
            Self::ReflectHorizontal => Some(Coordinate {
                x: f64::from(input.width - 1) - point.x,
                y: point.y,
            }),
            Self::Straighten { degrees } => {
                let radians = f64::from(degrees).to_radians();
                let (sin, cos) = radians.sin_cos();
                let center_x = (f64::from(input.width) - 1.0) * 0.5;
                let center_y = (f64::from(input.height) - 1.0) * 0.5;
                let dx = point.x - center_x;
                let dy = point.y - center_y;
                Some(Coordinate {
                    x: center_x + dx * cos - dy * sin,
                    y: center_y + dx * sin + dy * cos,
                })
            }
            Self::Perspective { corners } => {
                if input.width == 1 && input.height == 1 {
                    return Some(Coordinate { x: 0.0, y: 0.0 });
                }
                let target = Coordinate {
                    x: normalize(point.x, input.width),
                    y: normalize(point.y, input.height),
                };
                let (u, v) = if input.width == 1 {
                    let top = f64::from(corners.top_left[1]);
                    let span = f64::from(corners.bottom_left[1] - corners.top_left[1]);
                    if span.abs() < MIN_PERSPECTIVE_JACOBIAN {
                        return None;
                    }
                    (0.0, (target.y - top) / span)
                } else if input.height == 1 {
                    let left = f64::from(corners.top_left[0]);
                    let span = f64::from(corners.top_right[0] - corners.top_left[0]);
                    if span.abs() < MIN_PERSPECTIVE_JACOBIAN {
                        return None;
                    }
                    ((target.x - left) / span, 0.0)
                } else {
                    BilinearMap::new(&corners).invert(target)?
                };
                Some(Coordinate {
                    x: denormalize(u, output.width),
                    y: denormalize(v, output.height),
                })
            }
            Self::LensCorrection { distortion, .. } => {
                LensMap::new(input, distortion).invert(point)
            }
        }
    }
}

impl LensMap {
    fn new(dimensions: Dimensions, distortion: f32) -> Self {
        let center = Coordinate {
            x: (f64::from(dimensions.width) - 1.0) * 0.5,
            y: (f64::from(dimensions.height) - 1.0) * 0.5,
        };
        Self {
            center,
            pixel_scale: Coordinate {
                x: center.x.max(1.0),
                y: center.y.max(1.0),
            },
            distortion: f64::from(distortion),
            active_x: dimensions.width > 1,
            active_y: dimensions.height > 1,
        }
    }

    fn normalize(self, point: Coordinate) -> Coordinate {
        Coordinate {
            x: if self.active_x {
                (point.x - self.center.x) / self.pixel_scale.x
            } else {
                0.0
            },
            y: if self.active_y {
                (point.y - self.center.y) / self.pixel_scale.y
            } else {
                0.0
            },
        }
    }

    fn denormalize(self, point: Coordinate) -> Coordinate {
        Coordinate {
            x: self.center.x + point.x * self.pixel_scale.x,
            y: self.center.y + point.y * self.pixel_scale.y,
        }
    }

    fn evaluate_normalized(self, point: Coordinate) -> Coordinate {
        let radius_squared = point.x * point.x + point.y * point.y;
        scale(point, 1.0 + self.distortion * radius_squared)
    }

    fn evaluate(self, point: Coordinate) -> Coordinate {
        self.denormalize(self.evaluate_normalized(self.normalize(point)))
    }

    fn invert(self, target: Coordinate) -> Option<Coordinate> {
        let target = self.normalize(target);
        if !target.x.is_finite() || !target.y.is_finite() {
            return None;
        }
        let target_radius = target.x.hypot(target.y);
        if target_radius <= LENS_INVERSE_RESIDUAL_TOLERANCE {
            return Some(self.center);
        }

        let direction = scale(target, 1.0 / target_radius);
        let mut maximum_radius = f64::INFINITY;
        if self.active_x && direction.x.abs() > LENS_INVERSE_RESIDUAL_TOLERANCE {
            maximum_radius = maximum_radius.min(1.0 / direction.x.abs());
        }
        if self.active_y && direction.y.abs() > LENS_INVERSE_RESIDUAL_TOLERANCE {
            maximum_radius = maximum_radius.min(1.0 / direction.y.abs());
        }
        if !maximum_radius.is_finite() {
            return Some(self.center);
        }

        let radial =
            |radius: f64| radius * (1.0 + self.distortion * radius * radius) - target_radius;
        if radial(maximum_radius) < -LENS_INVERSE_RESIDUAL_TOLERANCE {
            return None;
        }

        let mut low = 0.0;
        let mut high = maximum_radius;
        let mut radius = target_radius.clamp(low, high);
        let mut converged = false;
        for _ in 0..MAX_LENS_INVERSE_ITERATIONS {
            let residual = radial(radius);
            if residual.abs() <= LENS_INVERSE_RESIDUAL_TOLERANCE {
                converged = true;
                break;
            }
            if residual > 0.0 {
                high = radius;
            } else {
                low = radius;
            }
            let derivative = 1.0 + 3.0 * self.distortion * radius * radius;
            if !derivative.is_finite() || derivative <= MIN_LENS_JACOBIAN {
                return None;
            }
            let newton = radius - residual / derivative;
            radius = if newton > low && newton < high && newton.is_finite() {
                newton
            } else {
                (low + high) * 0.5
            };
        }
        if !converged && radial(radius).abs() > LENS_INVERSE_RESIDUAL_TOLERANCE {
            return None;
        }

        let normalized = scale(direction, radius);
        if normalized.x < -1.0 - INVERSE_DOMAIN_TOLERANCE
            || normalized.x > 1.0 + INVERSE_DOMAIN_TOLERANCE
            || normalized.y < -1.0 - INVERSE_DOMAIN_TOLERANCE
            || normalized.y > 1.0 + INVERSE_DOMAIN_TOLERANCE
        {
            return None;
        }
        Some(self.denormalize(Coordinate {
            x: normalized.x.clamp(-1.0, 1.0),
            y: normalized.y.clamp(-1.0, 1.0),
        }))
    }
}

impl BilinearMap {
    fn new(corners: &PerspectiveCorners) -> Self {
        let top_left = coordinate(corners.top_left);
        let top_right = coordinate(corners.top_right);
        let bottom_right = coordinate(corners.bottom_right);
        let bottom_left = coordinate(corners.bottom_left);
        Self {
            origin: top_left,
            horizontal: subtract(top_right, top_left),
            vertical: subtract(bottom_left, top_left),
            cross: add(
                subtract(bottom_right, top_right),
                subtract(top_left, bottom_left),
            ),
        }
    }

    fn evaluate(self, u: f64, v: f64) -> Coordinate {
        add(
            self.origin,
            add(
                scale(self.horizontal, u),
                add(scale(self.vertical, v), scale(self.cross, u * v)),
            ),
        )
    }

    fn jacobian(self, u: f64, v: f64) -> (Coordinate, Coordinate) {
        (
            add(self.horizontal, scale(self.cross, v)),
            add(self.vertical, scale(self.cross, u)),
        )
    }

    fn invert(self, target: Coordinate) -> Option<(f64, f64)> {
        let relative = subtract(target, self.origin);
        let affine_determinant = cross(self.horizontal, self.vertical);
        let (mut u, mut v) = if affine_determinant.abs() >= MIN_PERSPECTIVE_JACOBIAN {
            (
                cross(relative, self.vertical) / affine_determinant,
                cross(self.horizontal, relative) / affine_determinant,
            )
        } else {
            (target.x, target.y)
        };
        if !u.is_finite() || !v.is_finite() {
            return None;
        }

        let mut converged = false;
        for _ in 0..MAX_INVERSE_ITERATIONS {
            let residual = subtract(self.evaluate(u, v), target);
            if residual.x.abs().max(residual.y.abs()) <= INVERSE_RESIDUAL_TOLERANCE {
                converged = true;
                break;
            }
            let (du_axis, dv_axis) = self.jacobian(u, v);
            let determinant = cross(du_axis, dv_axis);
            if !determinant.is_finite() || determinant.abs() < MIN_PERSPECTIVE_JACOBIAN * 0.01 {
                return None;
            }
            let delta_u = cross(residual, dv_axis) / determinant;
            let delta_v = cross(du_axis, residual) / determinant;
            if !delta_u.is_finite() || !delta_v.is_finite() {
                return None;
            }
            u -= delta_u;
            v -= delta_v;
            if !u.is_finite() || !v.is_finite() || u.abs() > 4.0 || v.abs() > 4.0 {
                return None;
            }
        }
        let residual = subtract(self.evaluate(u, v), target);
        if !converged && residual.x.abs().max(residual.y.abs()) > INVERSE_RESIDUAL_TOLERANCE {
            return None;
        }
        if !(-INVERSE_DOMAIN_TOLERANCE..=1.0 + INVERSE_DOMAIN_TOLERANCE).contains(&u)
            || !(-INVERSE_DOMAIN_TOLERANCE..=1.0 + INVERSE_DOMAIN_TOLERANCE).contains(&v)
        {
            return None;
        }
        Some((u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)))
    }
}

pub fn remap_between_chains(
    mask: &MaskBitmap,
    old_chain: &GeometryChain,
    old_stage: usize,
    new_chain: &GeometryChain,
    new_stage: usize,
    cancelled: Option<&AtomicBool>,
) -> Result<MaskBitmap, AppError> {
    remap_between_chains_with_progress(
        mask,
        old_chain,
        old_stage,
        new_chain,
        new_stage,
        MaskWorkContext::cancellation_only(cancelled),
    )
}

pub(crate) fn remap_between_chains_with_progress(
    mask: &MaskBitmap,
    old_chain: &GeometryChain,
    old_stage: usize,
    new_chain: &GeometryChain,
    new_stage: usize,
    context: MaskWorkContext<'_>,
) -> Result<MaskBitmap, AppError> {
    context.check_cancelled()?;
    if old_chain.original != new_chain.original {
        return Err(AppError::InvalidMask(
            "old and new mask geometry chains require the same original dimensions".into(),
        ));
    }
    let old_dimensions = old_chain.dimensions_value(old_stage)?;
    let new_dimensions = new_chain.dimensions_value(new_stage)?;
    if (mask.width(), mask.height()) != (old_dimensions.width, old_dimensions.height) {
        return Err(AppError::MaskDimensionMismatch {
            mask_width: mask.width(),
            mask_height: mask.height(),
            image_width: old_dimensions.width,
            image_height: old_dimensions.height,
        });
    }

    if old_stage <= new_stage
        && spatially_equal(&old_chain.steps[..old_stage], &new_chain.steps[..old_stage])
        && new_chain.steps[old_stage..new_stage]
            .iter()
            .all(|step| step.has_exact_mask_mapping())
    {
        let mut output = mask.clone();
        for index in old_stage..new_stage {
            let phase = format!("geometry_step_{index}_rows");
            output = apply_exact_step(&output, new_chain.steps[index], context, &phase)?;
        }
        return Ok(output);
    }

    let mut output = MaskBitmap::empty(new_dimensions.width, new_dimensions.height)?;
    for y in 0..new_dimensions.height {
        context.report(
            "geometry_resample_rows",
            u64::from(y),
            u64::from(new_dimensions.height),
        )?;
        for x in 0..new_dimensions.width {
            if x % 4_096 == 0 {
                context.check_cancelled()?;
            }
            let point = Coordinate {
                x: f64::from(x),
                y: f64::from(y),
            };
            let coverage = new_chain
                .output_to_original(new_stage, point)
                .and_then(|original| old_chain.original_to_output(old_stage, original))
                .map_or(0, |old_point| sample_coverage(mask, old_point));
            output.set(x, y, coverage);
        }
    }
    context.report(
        "geometry_resample_rows",
        u64::from(new_dimensions.height),
        u64::from(new_dimensions.height),
    )?;
    Ok(output)
}

fn apply_exact_step(
    mask: &MaskBitmap,
    step: GeometryStep,
    context: MaskWorkContext<'_>,
    phase: &str,
) -> Result<MaskBitmap, AppError> {
    let input = Dimensions {
        width: mask.width(),
        height: mask.height(),
    };
    match step {
        GeometryStep::Crop {
            x,
            y,
            width,
            height,
        } => {
            let bounds = crop_bounds(input, x, y, width, height)?;
            let mut output = MaskBitmap::empty(bounds.width, bounds.height)?;
            for y in 0..bounds.height {
                context.report(phase, u64::from(y), u64::from(bounds.height))?;
                let source_start = ((bounds.top + y) * mask.width() + bounds.left) as usize;
                let destination_start = (y * bounds.width) as usize;
                let row_width = bounds.width as usize;
                for offset in (0..row_width).step_by(4_096) {
                    context.check_cancelled()?;
                    let end = (offset + 4_096).min(row_width);
                    output.coverage_mut()[destination_start + offset..destination_start + end]
                        .copy_from_slice(
                            &mask.coverage()[source_start + offset..source_start + end],
                        );
                }
            }
            context.report(phase, u64::from(bounds.height), u64::from(bounds.height))?;
            Ok(output)
        }
        GeometryStep::Rotate { degrees } => {
            let degrees = degrees.rem_euclid(360);
            if degrees == 0 {
                context.report(phase, u64::from(mask.height()), u64::from(mask.height()))?;
                return Ok(mask.clone());
            }
            let output_dimensions = step.output_dimensions(input)?;
            let mut output = MaskBitmap::empty(output_dimensions.width, output_dimensions.height)?;
            for y in 0..mask.height() {
                context.report(phase, u64::from(y), u64::from(mask.height()))?;
                for x in 0..mask.width() {
                    if x % 4_096 == 0 {
                        context.check_cancelled()?;
                    }
                    let (output_x, output_y) = match degrees {
                        90 => (mask.height() - 1 - y, x),
                        180 => (mask.width() - 1 - x, mask.height() - 1 - y),
                        270 => (y, mask.width() - 1 - x),
                        _ => unreachable!("rotation was validated"),
                    };
                    output.set(output_x, output_y, mask.get(x, y));
                }
            }
            context.report(phase, u64::from(mask.height()), u64::from(mask.height()))?;
            Ok(output)
        }
        GeometryStep::ReflectHorizontal => {
            let mut output = MaskBitmap::empty(mask.width(), mask.height())?;
            for y in 0..mask.height() {
                context.report(phase, u64::from(y), u64::from(mask.height()))?;
                for x in 0..mask.width() {
                    if x % 4_096 == 0 {
                        context.check_cancelled()?;
                    }
                    output.set(mask.width() - 1 - x, y, mask.get(x, y));
                }
            }
            context.report(phase, u64::from(mask.height()), u64::from(mask.height()))?;
            Ok(output)
        }
        GeometryStep::LensCorrection {
            distortion: 0.0, ..
        } => {
            context.report(phase, u64::from(mask.height()), u64::from(mask.height()))?;
            Ok(mask.clone())
        }
        GeometryStep::Straighten { .. }
        | GeometryStep::Perspective { .. }
        | GeometryStep::LensCorrection { .. } => Err(AppError::InvalidMask(
            "interpolated geometry cannot use the exact mask path".into(),
        )),
    }
}

fn sample_coverage(mask: &MaskBitmap, point: Coordinate) -> u8 {
    if !contains(
        point,
        Dimensions {
            width: mask.width(),
            height: mask.height(),
        },
    ) {
        return 0;
    }
    let left = point.x.floor() as u32;
    let top = point.y.floor() as u32;
    let right = (left + 1).min(mask.width() - 1);
    let bottom = (top + 1).min(mask.height() - 1);
    let fraction_x = point.x - f64::from(left);
    let fraction_y = point.y - f64::from(top);
    let top_value = f64::from(mask.get(left, top)) * (1.0 - fraction_x)
        + f64::from(mask.get(right, top)) * fraction_x;
    let bottom_value = f64::from(mask.get(left, bottom)) * (1.0 - fraction_x)
        + f64::from(mask.get(right, bottom)) * fraction_x;
    (top_value * (1.0 - fraction_y) + bottom_value * fraction_y)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn crop_bounds(
    input: Dimensions,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Result<CropBounds, AppError> {
    if ![x, y, width, height].iter().all(|value| value.is_finite())
        || x < 0.0
        || y < 0.0
        || width <= 0.0
        || height <= 0.0
        || x + width > 1.000_001
        || y + height > 1.000_001
    {
        return Err(AppError::InvalidMask(
            "mask crop bounds must be finite normalized coordinates inside the image".into(),
        ));
    }
    let left = (x * input.width as f32).floor() as u32;
    let top = (y * input.height as f32).floor() as u32;
    if left >= input.width || top >= input.height {
        return Err(AppError::InvalidMask(
            "mask crop origin is outside the image".into(),
        ));
    }
    let output_width = ((width * input.width as f32).round() as u32)
        .max(1)
        .min(input.width - left);
    let output_height = ((height * input.height as f32).round() as u32)
        .max(1)
        .min(input.height - top);
    Ok(CropBounds {
        left,
        top,
        width: output_width,
        height: output_height,
    })
}

fn validate_perspective(corners: &PerspectiveCorners) -> Result<(), AppError> {
    let values = [
        corners.top_left,
        corners.top_right,
        corners.bottom_right,
        corners.bottom_left,
    ];
    if !values
        .iter()
        .flatten()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        || corners.top_left[0] >= corners.top_right[0]
        || corners.bottom_left[0] >= corners.bottom_right[0]
        || corners.top_left[1] >= corners.bottom_left[1]
        || corners.top_right[1] >= corners.bottom_right[1]
    {
        return Err(AppError::InvalidMask(
            "perspective corners must form an ordered normalized quadrilateral".into(),
        ));
    }
    let mapping = BilinearMap::new(corners);
    let mut sign = 0.0_f64;
    for (u, v) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let (horizontal, vertical) = mapping.jacobian(u, v);
        let determinant = cross(horizontal, vertical);
        if !determinant.is_finite() || determinant.abs() < MIN_PERSPECTIVE_JACOBIAN {
            return Err(AppError::InvalidMask(
                "perspective mapping is singular or too close to singular".into(),
            ));
        }
        if sign == 0.0 {
            sign = determinant.signum();
        } else if determinant.signum() != sign {
            return Err(AppError::InvalidMask(
                "perspective mapping folds over itself".into(),
            ));
        }
    }
    Ok(())
}

fn validate_lens_correction(
    dimensions: Dimensions,
    distortion: f32,
    vignetting: f32,
    chromatic_aberration: f32,
) -> Result<(), AppError> {
    if !distortion.is_finite()
        || !(-0.16..=1.0).contains(&distortion)
        || ![vignetting, chromatic_aberration]
            .iter()
            .all(|value| value.is_finite() && (-1.0..=1.0).contains(value))
    {
        return Err(AppError::InvalidMask(
            "lens distortion must be between -0.16 and 1; vignetting and chromatic aberration must be between -1 and 1".into(),
        ));
    }

    let maximum_radius_squared =
        f64::from(u8::from(dimensions.width > 1) + u8::from(dimensions.height > 1));
    if maximum_radius_squared > 0.0 && distortion < 0.0 {
        let distortion = f64::from(distortion);
        let tangential = 1.0 + distortion * maximum_radius_squared;
        let radial = 1.0 + 3.0 * distortion * maximum_radius_squared;
        if tangential <= MIN_LENS_JACOBIAN || radial <= MIN_LENS_JACOBIAN {
            return Err(AppError::InvalidMask(
                "lens distortion folds the canvas or is too close to singular for a deterministic mask inverse"
                    .into(),
            ));
        }
    }
    Ok(())
}

fn spatially_equal(left: &[GeometryStep], right: &[GeometryStep]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.spatially_equivalent(*right))
}

fn contains(point: Coordinate, dimensions: Dimensions) -> bool {
    point.x.is_finite()
        && point.y.is_finite()
        && point.x >= 0.0
        && point.y >= 0.0
        && point.x <= f64::from(dimensions.width - 1)
        && point.y <= f64::from(dimensions.height - 1)
}

fn clamp_domain(point: Coordinate, dimensions: Dimensions) -> Option<Coordinate> {
    let maximum_x = f64::from(dimensions.width - 1);
    let maximum_y = f64::from(dimensions.height - 1);
    if !point.x.is_finite()
        || !point.y.is_finite()
        || point.x < -INVERSE_DOMAIN_TOLERANCE
        || point.y < -INVERSE_DOMAIN_TOLERANCE
        || point.x > maximum_x + INVERSE_DOMAIN_TOLERANCE
        || point.y > maximum_y + INVERSE_DOMAIN_TOLERANCE
    {
        return None;
    }
    Some(Coordinate {
        x: point.x.clamp(0.0, maximum_x),
        y: point.y.clamp(0.0, maximum_y),
    })
}

fn normalize(value: f64, length: u32) -> f64 {
    if length > 1 {
        value / f64::from(length - 1)
    } else {
        0.0
    }
}

fn denormalize(value: f64, length: u32) -> f64 {
    value * f64::from(length.saturating_sub(1))
}

fn coordinate(value: [f32; 2]) -> Coordinate {
    Coordinate {
        x: f64::from(value[0]),
        y: f64::from(value[1]),
    }
}

fn add(left: Coordinate, right: Coordinate) -> Coordinate {
    Coordinate {
        x: left.x + right.x,
        y: left.y + right.y,
    }
}

fn subtract(left: Coordinate, right: Coordinate) -> Coordinate {
    Coordinate {
        x: left.x - right.x,
        y: left.y - right.y,
    }
}

fn scale(value: Coordinate, amount: f64) -> Coordinate {
    Coordinate {
        x: value.x * amount,
        y: value.y * amount,
    }
}

fn cross(left: Coordinate, right: Coordinate) -> f64 {
    left.x * right.y - left.y * right.x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(width: u32, height: u32, steps: Vec<GeometryStep>) -> GeometryChain {
        GeometryChain::new(width, height, steps).unwrap()
    }

    fn remap(
        source: &MaskBitmap,
        old: &GeometryChain,
        old_stage: usize,
        new: &GeometryChain,
        new_stage: usize,
    ) -> MaskBitmap {
        remap_between_chains(source, old, old_stage, new, new_stage, None).unwrap()
    }

    fn rotate(degrees: i32) -> GeometryStep {
        GeometryStep::Rotate { degrees }
    }

    fn lens(distortion: f32, vignetting: f32, chromatic_aberration: f32) -> GeometryStep {
        GeometryStep::LensCorrection {
            distortion,
            vignetting,
            chromatic_aberration,
        }
    }

    #[test]
    fn crop_copies_fractional_coverage_exactly() {
        let source =
            MaskBitmap::from_coverage(4, 3, vec![0, 1, 2, 3, 10, 64, 128, 13, 20, 192, 255, 23])
                .unwrap();
        let old = chain(4, 3, vec![]);
        let new = chain(
            4,
            3,
            vec![GeometryStep::Crop {
                x: 0.25,
                y: 1.0 / 3.0,
                width: 0.5,
                height: 2.0 / 3.0,
            }],
        );
        let output = remap(&source, &old, 0, &new, 1);
        assert_eq!((output.width(), output.height()), (2, 2));
        assert_eq!(output.coverage(), &[64, 128, 192, 255]);
    }

    #[test]
    fn expanding_from_an_old_crop_uses_zero_outside_known_coverage() {
        let old = chain(
            4,
            1,
            vec![GeometryStep::Crop {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            }],
        );
        let new = chain(4, 1, vec![]);
        let cropped = MaskBitmap::from_coverage(2, 1, vec![120, 240]).unwrap();
        let output = remap(&cropped, &old, 1, &new, 0);
        assert_eq!(output.coverage(), &[0, 0, 120, 240]);
    }

    #[test]
    fn quadrant_rotations_preserve_rectangular_partial_masks_exactly() {
        let source = MaskBitmap::from_coverage(3, 2, vec![0, 64, 128, 192, 224, 255]).unwrap();
        let old = chain(3, 2, vec![]);
        let expected = [
            (90, (2, 3), vec![192, 0, 224, 64, 255, 128]),
            (180, (3, 2), vec![255, 224, 192, 128, 64, 0]),
            (270, (2, 3), vec![128, 255, 64, 224, 0, 192]),
        ];
        for (degrees, dimensions, coverage) in expected {
            let new = chain(3, 2, vec![rotate(degrees)]);
            let output = remap(&source, &old, 0, &new, 1);
            assert_eq!((output.width(), output.height()), dimensions);
            assert_eq!(output.coverage(), coverage);
        }
    }

    #[test]
    fn straighten_identity_is_exact_and_arbitrary_rotation_is_bilinear() {
        let mut source = MaskBitmap::empty(7, 7).unwrap();
        for y in 1..6 {
            source.set(3, y, 255);
        }
        let old = chain(7, 7, vec![]);
        let identity = chain(7, 7, vec![GeometryStep::Straighten { degrees: 0.0 }]);
        assert_eq!(remap(&source, &old, 0, &identity, 1), source);

        let rotated = chain(7, 7, vec![GeometryStep::Straighten { degrees: 23.0 }]);
        let output = remap(&source, &old, 0, &rotated, 1);
        assert_eq!((output.width(), output.height()), (7, 7));
        assert!(output
            .coverage()
            .iter()
            .any(|value| (1..=254).contains(value)));
    }

    #[test]
    fn perspective_identity_and_valid_warp_are_supported() {
        let source =
            MaskBitmap::from_coverage(5, 5, (0..25).map(|value| (value * 10) as u8).collect())
                .unwrap();
        let old = chain(5, 5, vec![]);
        let identity = chain(
            5,
            5,
            vec![GeometryStep::Perspective {
                corners: PerspectiveCorners::default(),
            }],
        );
        assert_eq!(remap(&source, &old, 0, &identity, 1), source);

        let warped = chain(
            5,
            5,
            vec![GeometryStep::Perspective {
                corners: PerspectiveCorners {
                    top_left: [0.1, 0.05],
                    top_right: [0.9, 0.1],
                    bottom_right: [0.95, 0.9],
                    bottom_left: [0.05, 0.95],
                },
            }],
        );
        let output = remap(&source, &old, 0, &warped, 1);
        assert_eq!((output.width(), output.height()), (5, 5));
        assert_ne!(output, source);
        assert!(output.coverage().iter().any(|value| *value > 0));
    }

    #[test]
    fn lens_identity_and_non_spatial_changes_preserve_coverage_exactly() {
        let source =
            MaskBitmap::from_coverage(5, 3, (0..15).map(|value| value * 17).collect()).unwrap();
        let original = chain(5, 3, vec![]);
        let identity = chain(5, 3, vec![lens(0.0, 1.0, -1.0)]);
        assert_eq!(remap(&source, &original, 0, &identity, 1), source);

        let old = chain(5, 3, vec![lens(0.1, -0.7, -0.4)]);
        let new = chain(5, 3, vec![lens(0.1, 0.8, 0.9)]);
        assert_eq!(old.remap_work_units(1, &new, 1).unwrap(), 0);
        assert_eq!(remap(&source, &old, 1, &new, 1), source);
    }

    #[test]
    fn lens_forward_and_inverse_match_the_image_center_sample_mapping() {
        let dimensions = Dimensions {
            width: 5,
            height: 5,
        };
        let mapping = LensMap::new(dimensions, 0.25);
        let horizontal_edge = mapping.evaluate(Coordinate { x: 4.0, y: 2.0 });
        assert!((horizontal_edge.x - 4.5).abs() < 1.0e-12);
        assert!((horizontal_edge.y - 2.0).abs() < 1.0e-12);

        for distortion in [0.6, -0.1] {
            let mapping = LensMap::new(
                Dimensions {
                    width: 101,
                    height: 61,
                },
                distortion,
            );
            for output in [
                Coordinate { x: 50.0, y: 30.0 },
                Coordinate { x: 63.5, y: 19.25 },
                Coordinate { x: 27.75, y: 42.0 },
            ] {
                let input = mapping.evaluate(output);
                let restored = mapping.invert(input).unwrap();
                assert!((restored.x - output.x).abs() < 1.0e-6);
                assert!((restored.y - output.y).abs() < 1.0e-6);
            }
        }

        let wide = LensMap::new(
            Dimensions {
                width: 100_000_000,
                height: 1,
            },
            0.2,
        );
        let adjacent_to_center = Coordinate {
            x: 50_000_000.0,
            y: 0.0,
        };
        let restored = wide.invert(wide.evaluate(adjacent_to_center)).unwrap();
        assert!((restored.x - adjacent_to_center.x).abs() < 1.0e-4);
    }

    #[test]
    fn lens_distortion_resamples_once_and_uses_zero_outside_source_bounds() {
        use std::sync::Mutex;

        let source = MaskBitmap::full(9, 9).unwrap();
        let original = chain(9, 9, vec![]);
        let transformed = chain(
            9,
            9,
            vec![
                lens(0.5, 0.2, -0.3),
                GeometryStep::Straighten { degrees: 3.0 },
            ],
        );
        let reports = Mutex::new(Vec::new());
        let callback = |phase: &str, completed: u64, total: u64| {
            reports
                .lock()
                .unwrap()
                .push((phase.to_owned(), completed, total));
            Ok(())
        };
        let output = remap_between_chains_with_progress(
            &source,
            &original,
            0,
            &transformed,
            2,
            MaskWorkContext::new(None, Some(&callback)),
        )
        .unwrap();
        assert_eq!(output.get(4, 4), 255);
        assert_eq!(output.get(0, 0), 0);
        assert!(reports
            .into_inner()
            .unwrap()
            .iter()
            .all(|(phase, _, _)| phase == "geometry_resample_rows"));
    }

    #[test]
    fn folded_extreme_and_invalid_lens_settings_are_rejected() {
        for distortion in [-1.0, -0.2, -1.0 / 6.0] {
            assert!(GeometryChain::new(10, 10, vec![lens(distortion, 0.0, 0.0)]).is_err());
        }
        assert!(GeometryChain::new(10, 10, vec![lens(f32::NAN, 0.0, 0.0)]).is_err());
        assert!(GeometryChain::new(10, 10, vec![lens(1.01, 0.0, 0.0)]).is_err());
        assert!(GeometryChain::new(10, 10, vec![lens(0.0, 1.01, 0.0)]).is_err());
        assert!(GeometryChain::new(10, 10, vec![lens(0.0, 0.0, f32::INFINITY)]).is_err());

        assert!(GeometryChain::new(10, 10, vec![lens(-0.16, 0.0, 0.0)]).is_ok());
        assert!(GeometryChain::new(1, 10, vec![lens(-0.16, 0.0, 0.0)]).is_ok());
        assert!(GeometryChain::new(1, 1, vec![lens(-0.16, 1.0, -1.0)]).is_ok());
    }

    #[test]
    fn lens_geometry_wire_format_preserves_all_workflow_fields() {
        let step = lens(0.125, -0.25, 0.5);
        let value = serde_json::to_value(step).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "type": "lens_correction",
                "distortion": 0.125,
                "vignetting": -0.25,
                "chromatic_aberration": 0.5
            })
        );
        assert_eq!(serde_json::from_value::<GeometryStep>(value).unwrap(), step);
    }

    #[test]
    fn lens_preserves_stage_dimensions_in_ordered_geometry_chains() {
        let geometry = chain(
            8,
            6,
            vec![
                GeometryStep::Crop {
                    x: 0.25,
                    y: 0.0,
                    width: 0.5,
                    height: 1.0,
                },
                lens(0.1, 0.4, -0.2),
                rotate(90),
            ],
        );
        assert_eq!(geometry.dimensions_at(1).unwrap(), (4, 6));
        assert_eq!(geometry.dimensions_at(2).unwrap(), (4, 6));
        assert_eq!(geometry.dimensions_at(3).unwrap(), (6, 4));
    }

    #[test]
    fn valid_perspective_inverse_converges_when_rebasing_to_original_space() {
        let source = MaskBitmap::from_coverage(
            17,
            17,
            (0..17)
                .flat_map(|y| (0..17).map(move |x| ((x * 7 + y * 5).min(255)) as u8))
                .collect(),
        )
        .unwrap();
        let original = chain(17, 17, vec![]);
        let perspective = chain(
            17,
            17,
            vec![GeometryStep::Perspective {
                corners: PerspectiveCorners {
                    top_left: [0.08, 0.04],
                    top_right: [0.94, 0.12],
                    bottom_right: [0.9, 0.94],
                    bottom_left: [0.12, 0.88],
                },
            }],
        );
        let warped = remap(&source, &original, 0, &perspective, 1);
        let restored = remap(&warped, &perspective, 1, &original, 0);
        assert!(restored.get(8, 8).abs_diff(source.get(8, 8)) <= 2);
        assert!(restored.coverage().iter().any(|value| *value > 0));
    }

    #[test]
    fn thin_identity_perspective_remains_bounded() {
        for source in [
            MaskBitmap::from_coverage(1, 4, vec![0, 64, 192, 255]).unwrap(),
            MaskBitmap::from_coverage(4, 1, vec![0, 64, 192, 255]).unwrap(),
        ] {
            let original = chain(source.width(), source.height(), vec![]);
            let perspective = chain(
                source.width(),
                source.height(),
                vec![GeometryStep::Perspective {
                    corners: PerspectiveCorners::default(),
                }],
            );
            let warped = remap(&source, &original, 0, &perspective, 1);
            let restored = remap(&warped, &perspective, 1, &original, 0);
            assert_eq!(warped, source);
            assert_eq!(restored, source);
        }
    }

    #[test]
    fn folded_and_near_singular_perspective_are_rejected() {
        let folded = GeometryStep::Perspective {
            corners: PerspectiveCorners {
                top_left: [0.0, 0.0],
                top_right: [1.0, 0.0],
                bottom_right: [0.4, 0.4],
                bottom_left: [0.0, 1.0],
            },
        };
        assert!(GeometryChain::new(10, 10, vec![folded]).is_err());

        let singular = GeometryStep::Perspective {
            corners: PerspectiveCorners {
                top_left: [0.0, 0.0],
                top_right: [1.0, 0.0],
                bottom_right: [1.0, 0.000_000_1],
                bottom_left: [0.0, 0.000_000_1],
            },
        };
        assert!(GeometryChain::new(10, 10, vec![singular]).is_err());
    }

    #[test]
    fn crop_then_rotate_uses_ordered_stage_dimensions() {
        let source = MaskBitmap::from_coverage(4, 2, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let old = chain(4, 2, vec![]);
        let new = chain(
            4,
            2,
            vec![
                GeometryStep::Crop {
                    x: 0.25,
                    y: 0.0,
                    width: 0.5,
                    height: 1.0,
                },
                rotate(90),
            ],
        );
        let output = remap(&source, &old, 0, &new, 2);
        assert_eq!((output.width(), output.height()), (2, 2));
        assert_eq!(output.coverage(), &[6, 2, 7, 3]);
    }

    #[test]
    fn general_old_to_new_rebase_matches_direct_content_mapping() {
        let source = MaskBitmap::from_coverage(3, 2, vec![0, 64, 128, 192, 224, 255]).unwrap();
        let original = chain(3, 2, vec![]);
        let old = chain(3, 2, vec![rotate(90)]);
        let new = chain(3, 2, vec![rotate(180)]);
        let old_mask = remap(&source, &original, 0, &old, 1);
        let rebased = remap(&old_mask, &old, 1, &new, 1);
        let direct = remap(&source, &original, 0, &new, 1);
        assert_eq!(rebased, direct);
    }

    #[test]
    fn invalid_stage_dimensions_and_cancellation_fail_closed() {
        let source = MaskBitmap::full(2, 2).unwrap();
        let old = chain(2, 2, vec![]);
        let new = chain(2, 2, vec![rotate(90)]);
        assert!(remap_between_chains(&source, &old, 1, &new, 1, None).is_err());

        let wrong = MaskBitmap::full(1, 2).unwrap();
        assert!(matches!(
            remap_between_chains(&wrong, &old, 0, &new, 1, None),
            Err(AppError::MaskDimensionMismatch { .. })
        ));

        let cancelled = AtomicBool::new(true);
        assert!(matches!(
            remap_between_chains(&source, &old, 0, &new, 1, Some(&cancelled)),
            Err(AppError::MaskCancelled)
        ));

        assert!(GeometryChain::new(
            1,
            1,
            vec![GeometryStep::Rotate { degrees: 0 }; MAX_GEOMETRY_STEPS + 1],
        )
        .is_err());
    }

    #[test]
    fn horizontal_reflect_round_trips_rectangular_partial_coverage() {
        let source =
            MaskBitmap::from_coverage(4, 2, vec![0, 64, 128, 255, 17, 93, 201, 240]).unwrap();
        let original = chain(4, 2, vec![]);
        let reflected = chain(4, 2, vec![GeometryStep::ReflectHorizontal]);
        let flipped = remap(&source, &original, 0, &reflected, 1);
        assert_eq!(flipped.coverage(), &[255, 128, 64, 0, 240, 201, 93, 17]);
        assert_eq!(remap(&flipped, &reflected, 1, &original, 0), source);
    }

    #[test]
    fn general_transform_reports_real_output_rows() {
        use std::sync::Mutex;

        let source = MaskBitmap::full(8, 6).unwrap();
        let old = chain(8, 6, vec![]);
        let new = chain(8, 6, vec![GeometryStep::Straighten { degrees: 11.0 }]);
        let reports = Mutex::new(Vec::new());
        let callback = |phase: &str, completed: u64, total: u64| {
            reports
                .lock()
                .unwrap()
                .push((phase.to_owned(), completed, total));
            Ok(())
        };
        remap_between_chains_with_progress(
            &source,
            &old,
            0,
            &new,
            1,
            MaskWorkContext::new(None, Some(&callback)),
        )
        .unwrap();
        let reports = reports.into_inner().unwrap();
        assert!(reports
            .iter()
            .any(|(_, completed, total)| { *completed > 0 && *completed < *total && *total == 6 }));
        assert_eq!(reports.last().unwrap().1, 6);
        assert_eq!(reports.last().unwrap().2, 6);
    }
}
