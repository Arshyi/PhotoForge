use super::{
    apply_mask_operation, import_png, load_mask, rasterize, remap_between_chains, GeometryChain,
    GeometryStep, MaskBitmap, MaskOperation, MaskSnapshot, Point, SelectionShape,
};
use crate::domain::PerspectiveCorners;
use std::fs;

#[test]
fn hostile_dimensions_and_payloads_fail_closed_without_declared_allocations() {
    assert!(MaskBitmap::empty(0, 1).is_err());
    assert!(MaskBitmap::empty(1, 0).is_err());
    assert!(MaskBitmap::empty(u32::MAX, u32::MAX).is_err());
    assert_eq!(MaskBitmap::full(1, 1).unwrap().coverage(), &[255]);

    let huge = MaskSnapshot {
        version: 1,
        width: u32::MAX,
        height: u32::MAX,
        encoding: "base64_u8".into(),
        data: String::new(),
        checksum: "fnv1a64:0000000000000000".into(),
    };
    assert!(huge.decode().is_err());

    let corrupt_cases = [
        MaskSnapshot {
            version: 1,
            width: 2,
            height: 2,
            encoding: "base64_u8".into(),
            data: "%%%".into(),
            checksum: "fnv1a64:0000000000000000".into(),
        },
        MaskSnapshot {
            version: 1,
            width: 2,
            height: 2,
            encoding: "base64_u8".into(),
            data: "AA".into(),
            checksum: "fnv1a64:0000000000000000".into(),
        },
        MaskSnapshot {
            version: 1,
            width: 2,
            height: 2,
            encoding: "base64_rle_u8".into(),
            data: "AAAA".into(),
            checksum: "fnv1a64:0000000000000000".into(),
        },
    ];
    for snapshot in corrupt_cases {
        assert!(snapshot.decode().is_err());
    }

    let directory = tempfile::tempdir().unwrap();
    let malformed_json = directory.path().join("malformed.photoforge-mask.json");
    fs::write(&malformed_json, b"{\"format\":").unwrap();
    assert!(load_mask(&malformed_json).is_err());
    let malformed_png = directory.path().join("malformed.png");
    fs::write(&malformed_png, b"\x89PNG\r\n\x1a\ntruncated").unwrap();
    assert!(import_png(&malformed_png).is_err());
}

#[test]
fn pathological_polygons_are_deterministic_and_bounded() {
    let self_intersecting = SelectionShape::Polygon {
        points: vec![
            Point { x: -50.0, y: -50.0 },
            Point { x: 100.0, y: 100.0 },
            Point { x: -50.0, y: 100.0 },
            Point { x: 100.0, y: -50.0 },
            Point { x: 100.0, y: -50.0 },
        ],
    };
    let first = rasterize(64, 64, &self_intersecting).unwrap();
    let second = rasterize(64, 64, &self_intersecting).unwrap();
    assert_eq!(first, second);

    let points = (0..10_000)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / 10_000.0;
            Point {
                x: 64.0 + angle.cos() * 50.0,
                y: 64.0 + angle.sin() * 50.0,
            }
        })
        .collect();
    let dense = rasterize(128, 128, &SelectionShape::Polygon { points }).unwrap();
    assert!(dense.coverage().contains(&255));
}

#[test]
fn non_finite_geometry_and_singular_perspective_are_rejected() {
    assert!(rasterize(
        8,
        8,
        &SelectionShape::Rectangle {
            start: Point {
                x: f32::NEG_INFINITY,
                y: 0.0,
            },
            end: Point { x: 4.0, y: 4.0 },
        },
    )
    .is_err());

    for invalid in [f32::NAN, f32::INFINITY] {
        assert!(
            GeometryChain::new(8, 8, vec![GeometryStep::Straighten { degrees: invalid }]).is_err()
        );
    }

    let collapsed = PerspectiveCorners {
        top_left: [0.5, 0.5],
        top_right: [0.5, 0.5],
        bottom_right: [0.5, 0.5],
        bottom_left: [0.5, 0.5],
    };
    assert!(
        GeometryChain::new(8, 8, vec![GeometryStep::Perspective { corners: collapsed }]).is_err()
    );
}

#[test]
fn single_pixel_and_extremely_thin_masks_remain_safe_through_operations() {
    let pixel = MaskBitmap::full(1, 1).unwrap();
    let identity = GeometryChain::new(1, 1, vec![]).unwrap();
    let reflected = GeometryChain::new(1, 1, vec![GeometryStep::ReflectHorizontal]).unwrap();
    assert_eq!(
        remap_between_chains(&pixel, &identity, 0, &reflected, 1, None).unwrap(),
        pixel
    );

    let mut thin = MaskBitmap::empty(1, 4_096).unwrap();
    thin.set(0, 2_048, 128);
    let softened =
        apply_mask_operation(&thin, &MaskOperation::Feather { radius: 25 }, None).unwrap();
    assert_eq!((softened.width(), softened.height()), (1, 4_096));
    assert!(softened
        .coverage()
        .iter()
        .any(|coverage| (1..255).contains(coverage)));

    let old = GeometryChain::new(1, 4_096, vec![]).unwrap();
    let rotated = GeometryChain::new(1, 4_096, vec![GeometryStep::Rotate { degrees: 90 }]).unwrap();
    let output = remap_between_chains(&thin, &old, 0, &rotated, 1, None).unwrap();
    assert_eq!((output.width(), output.height()), (4_096, 1));
    assert_eq!(
        output
            .coverage()
            .iter()
            .filter(|value| **value == 128)
            .count(),
        1
    );
}
