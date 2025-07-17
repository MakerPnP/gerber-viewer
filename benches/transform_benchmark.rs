use criterion::{Criterion, black_box, criterion_group, criterion_main};
use gerber_viewer::{GerberTransform, Matrix3Point2Ext};
use nalgebra::{Point2, Vector2};
use rand::Rng;

// Implement a basic Transform struct for testing
pub struct TestTransform {
    // Add relevant fields to match your actual Transform struct
    scale_x: f64,
    scale_y: f64,
    rotation: f64,
    translation_x: f64,
    translation_y: f64,
}

impl TestTransform {
    fn new_random() -> Self {
        let mut rng = rand::rng();
        Self {
            scale_x: rng.random_range(0.5..2.0),
            scale_y: rng.random_range(0.5..2.0),
            rotation: rng.random_range(0.0..std::f64::consts::PI * 2.0),
            translation_x: rng.random_range(-100.0..100.0),
            translation_y: rng.random_range(-100.0..100.0),
        }
    }
}

fn generate_random_points(count: usize) -> Vec<Point2<f64>> {
    let mut rng = rand::rng();
    (0..count)
        .map(|_| Point2::new(rng.random_range(-1000.0..1000.0), rng.random_range(-1000.0..1000.0)))
        .collect()
}

fn benchmark_transforms(c: &mut Criterion) {
    let num_points = 10000;
    let points = generate_random_points(num_points);
    let transform = TestTransform::new_random();

    // Create a GerberTransform with similar properties
    let gerber_transform = GerberTransform {
        rotation: transform.rotation as f32,
        mirroring: [false, false].into(),
        origin: Vector2::new(0.0, 0.0),
        offset: Vector2::new(transform.translation_x, transform.translation_y),
        scale: ((transform.scale_x + transform.scale_y) / 2.0), // Average scale as GerberTransform uses uniform scaling
    };

    let matrix = gerber_transform.to_matrix();

    let mut group = c.benchmark_group("Point Transformations");

    // Benchmark transform_point2
    group.bench_function("transform_point2", |b| {
        b.iter(|| {
            for point in &points {
                black_box(matrix.transform_point2(black_box(*point)));
            }
        })
    });

    // New benchmarks for GerberTransform
    group.bench_function("gerber_apply_to_position", |b| {
        b.iter(|| {
            for point in &points {
                black_box(gerber_transform.apply_to_position(black_box(*point)));
            }
        })
    });

    group.bench_function("gerber_apply_to_position_matrix", |b| {
        b.iter(|| {
            for point in &points {
                black_box(gerber_transform.apply_to_position_matrix(black_box(*point)));
            }
        })
    });

    group.finish();
}

criterion_group!(benches, benchmark_transforms);
criterion_main!(benches);
