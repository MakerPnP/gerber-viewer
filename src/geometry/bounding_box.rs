use log::trace;
use nalgebra::{Matrix3, Point2, Vector2, Vector3};

use crate::geometry::transform::GerberTransform;

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub struct BoundingBox {
    pub min: Point2<f64>,
    pub max: Point2<f64>,
}

impl BoundingBox {
    /// Use to generate an outline of the bbox
    pub fn transform_vertices(&self, transform: &GerberTransform) -> Vec<Point2<f64>> {
        self.vertices()
            .into_iter()
            .map(|v| transform.apply_to_position(v))
            .collect::<Vec<_>>()
    }

    pub fn expand(&mut self, other: &BoundingBox) {
        self.min.x = self.min.x.min(other.min.x);
        self.min.y = self.min.y.min(other.min.y);
        self.max.x = self.max.x.max(other.max.x);
        self.max.y = self.max.y.max(other.max.y);
    }
}

impl Default for BoundingBox {
    fn default() -> Self {
        Self {
            min: Point2::new(f64::MAX, f64::MAX),
            max: Point2::new(f64::MIN, f64::MIN),
        }
    }
}

impl BoundingBox {
    /// Note that a bounding box of 0,0 -> 0,0 is NOT empty
    /// e.g., you could have a shape that defines a rectangle with an origin of 0,0 and a width + height of 0,0.
    ///
    /// Only a bounding box which is the same as the one returned by `default` counts as empty.
    pub fn is_empty(&self) -> bool {
        self.eq(&BoundingBox::default())
    }

    pub fn width(&self) -> f64 {
        self.max.x - self.min.x
    }
    pub fn height(&self) -> f64 {
        self.max.y - self.min.y
    }

    pub fn apply_transform_matrix(&self, matrix: &Matrix3<f64>) -> Self {
        // Step 1: Transform each corner of the original bbox
        let transformed_bbox_vertices: Vec<_> = self
            .vertices()
            .into_iter()
            .map(|v| {
                // Convert to homogeneous coordinates
                let point_vec = Vector3::new(v.x, v.y, 1.0);

                let transformed = matrix * point_vec;
                Point2::new(transformed.x, transformed.y)
            })
            .collect();

        // Step 2: Create a new axis-aligned bbox from transformed points (for viewport fitting)
        let result = BoundingBox::from_points(&transformed_bbox_vertices);
        trace!(
            "Applying transform matrix to bbox.  matrix {:?}: before: {:?}, after: {:?}",
            matrix,
            self,
            result
        );
        result
    }

    pub fn apply_transform(&self, transform: &GerberTransform) -> Self {
        // Step 1: Transform each corner of the original bbox
        let transformed_bbox_vertices: Vec<_> = self
            .vertices()
            .into_iter()
            .map(|v| transform.apply_to_position(v))
            .collect();

        // Step 2: Create a new axis-aligned bbox from transformed points (for viewport fitting)
        let result = BoundingBox::from_points(&transformed_bbox_vertices);
        trace!(
            "Applying transform to bbox.  transform {:?}: before: {:?}, after: {:?}",
            transform,
            self,
            result
        );
        result
    }

    /// Returns a new bounding box with X and/or Y mirroring applied.
    pub fn apply_mirroring(&self, mirror_x: bool, mirror_y: bool, offset: Vector2<f64>) -> Self {
        let mut vertices = self.vertices();

        for position in &mut vertices {
            if mirror_x {
                position.x = offset.x - (position.x - offset.x);
            }
            if mirror_y {
                position.y = offset.y - (position.y - offset.y);
            }
        }

        Self::from_points(&vertices)
    }

    /// Returns a new bounding box rotated around origin (0, 0) by given angle in radians.
    /// positive = counter-clockwise
    pub fn apply_rotation(&self, radians: f64, offset: Vector2<f64>) -> Self {
        let (sin_theta, cos_theta) = radians.sin_cos();
        let mut corners = self.vertices();

        for pt in &mut corners {
            let x = pt.x - offset.x;
            let y = pt.y - offset.y;

            let rotated_x = x * cos_theta - y * sin_theta;
            let rotated_y = x * sin_theta + y * cos_theta;

            pt.x = rotated_x + offset.x;
            pt.y = rotated_y + offset.y;
        }

        Self::from_points(&corners)
    }

    /// Returns the geometric center of the bounding box as a Point2
    pub fn center(&self) -> Point2<f64> {
        Point2::new(self.min.x + self.max.x, self.min.y + self.max.y) / 2.0
    }

    /// Returns 4 corner points of the bounding box such that the result is useable as a closed path.
    /// ```plaintext
    /// (min_x, min_y) 1 ┌────────────┐ 2 (max_x, min_y)
    ///                  │            │
    /// (min_x, max_y) 4 └────────────┘ 3 (max_x, max_y)
    /// ```
    pub fn vertices(&self) -> Vec<Point2<f64>> {
        vec![
            Point2::new(self.min.x, self.min.y),
            Point2::new(self.max.x, self.min.y),
            Point2::new(self.max.x, self.max.y),
            Point2::new(self.min.x, self.max.y),
        ]
    }

    /// Constructs a bounding box from a list of points
    pub fn from_points(points: &[Point2<f64>]) -> Self {
        let mut min = Point2::new(f64::MAX, f64::MAX);
        let mut max = Point2::new(f64::MIN, f64::MIN);

        for position in points {
            min.x = min.x.min(position.x);
            min.y = min.y.min(position.y);
            max.x = max.x.max(position.x);
            max.y = max.y.max(position.y);
        }

        Self {
            min,
            max,
        }
    }
}

#[cfg(test)]
mod bbox_tests {
    use nalgebra::{Point2, Vector2};
    use rstest::rstest;

    use crate::geometry::bounding_box::BoundingBox;

    #[rstest]
    #[case(BoundingBox::default(), true)]
    #[case(BoundingBox { min: Point2::new(0.0, 0.0), max: Point2::new(0.0, 0.0) }, false)]
    #[case(BoundingBox { min: Point2::new(-10.0, -10.0), max: Point2::new(10.0, 10.0) }, false)]
    pub fn test_is_empty(#[case] input: BoundingBox, #[case] expected: bool) {
        assert_eq!(input.is_empty(), expected);
    }

    #[test]
    pub fn test_apply_rotation_90_degrees_zero_offset() {
        let bbox = BoundingBox {
            min: Point2::new(1.0, 2.0),
            max: Point2::new(3.0, 4.0),
        };

        let rotated = bbox.apply_rotation(std::f64::consts::FRAC_PI_2, Vector2::new(0.0, 0.0)); // 90 degrees

        // Expected:
        // Points rotate CCW around origin:
        // (1,2) => (-2,1)
        // (1,4) => (-4,1)
        // (3,2) => (-2,3)
        // (3,4) => (-4,3)
        //
        // So bounds are:
        // min_x = -4, max_x = -2
        // min_y = 1,  max_y = 3

        assert!((rotated.min.x - -4.0).abs() < 1e-6);
        assert!((rotated.max.x - -2.0).abs() < 1e-6);
        assert!((rotated.min.y - 1.0).abs() < 1e-6);
        assert!((rotated.max.y - 3.0).abs() < 1e-6);
    }

    #[rstest]
    #[case((0.0, 0.0), (10.0, 10.0), (5.0, 5.0))] // Case 1: Origin 0, 10x10
    #[case((10.0, 10.0), (10.0, 10.0), (15.0, 15.0))] // Case 2: Origin 10, 10x10
    #[case((0.0, 0.0), (5.0, 10.0), (2.5, 5.0))] // Case 3: Origin 0, 5x10
    #[case((0.0, 0.0), (10.0, 5.0), (5.0, 2.5))] // Case 4: Origin 0, 10x5
    #[case((10.0, 10.0), (5.0, 10.0), (12.5, 15.0))] // Case 5: Origin 10, 5x10
    #[case((10.0, 10.0), (10.0, 5.0), (15.0, 12.5))] // Case 6: Origin 10, 10x5
    fn test_geometric_center(#[case] origin: (f64, f64), #[case] size: (f64, f64), #[case] expected: (f64, f64)) {
        // Create bounding box from origin and size
        let bbox = BoundingBox {
            min: Point2::new(origin.0, origin.1),
            max: Point2::new(origin.0 + size.0, origin.1 + size.1),
        };

        let center = bbox.center();

        // Compare with precision to handle floating-point numbers
        let epsilon = 1e-9;
        assert!(
            (center.x - expected.0).abs() < epsilon,
            "X mismatch: expected {}, got {}",
            expected.0,
            center.x
        );
        assert!(
            (center.y - expected.1).abs() < epsilon,
            "Y mismatch: expected {}, got {}",
            expected.1,
            center.y
        );
    }
}
