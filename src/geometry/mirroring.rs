use std::ops::Mul;

#[cfg(feature = "egui")]
use egui::Vec2b;
use nalgebra::{Point2, Vector2};

#[derive(Debug, Copy, Clone)]
pub struct Mirroring {
    pub x: bool,
    pub y: bool,
}

impl core::ops::BitXor for Mirroring {
    type Output = Self;

    #[inline]
    fn bitxor(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x ^ rhs.x,
            y: self.y ^ rhs.y,
        }
    }
}

impl Mul<(f32, f32)> for Mirroring {
    type Output = (f32, f32);

    #[inline]
    fn mul(self, rhs: (f32, f32)) -> Self::Output {
        (if self.x { -rhs.0 } else { rhs.0 }, if self.y { -rhs.1 } else { rhs.1 })
    }
}

impl Mul<(f64, f64)> for Mirroring {
    type Output = (f64, f64);

    #[inline]
    fn mul(self, rhs: (f64, f64)) -> Self::Output {
        (if self.x { -rhs.0 } else { rhs.0 }, if self.y { -rhs.1 } else { rhs.1 })
    }
}

impl Mul<Vector2<f64>> for Mirroring {
    type Output = Vector2<f64>;

    #[inline]
    fn mul(self, rhs: Vector2<f64>) -> Self::Output {
        Vector2::new(if self.x { -rhs.x } else { rhs.x }, if self.y { -rhs.y } else { rhs.y })
    }
}

impl Mul<Point2<f64>> for Mirroring {
    type Output = Point2<f64>;

    #[inline]
    fn mul(self, rhs: Point2<f64>) -> Self::Output {
        Point2::new(if self.x { -rhs.x } else { rhs.x }, if self.y { -rhs.y } else { rhs.y })
    }
}

#[cfg(test)]
mod mul {
    use nalgebra::Vector2;

    use super::*;

    #[test]
    pub fn test_mul_tuple_f32() {
        let mirroring = Mirroring {
            x: true,
            y: true,
        };

        let values = (10.0, -10.0);

        let result = mirroring * values;

        assert_eq!(result, (-10.0, 10.0));
    }

    #[test]
    pub fn test_mul_tuple_f64() {
        let mirroring = Mirroring {
            x: true,
            y: true,
        };

        let values = (10.0_f64, -10.0_f64);

        let result = mirroring * values;

        assert_eq!(result, (-10.0_f64, 10.0_f64));
    }

    #[test]
    pub fn test_mul_vec2() {
        let mirroring = Mirroring {
            x: true,
            y: true,
        };

        let value = Vector2::new(10.0, -10.0);

        let result = mirroring * value;

        assert_eq!(result, Vector2::new(-10.0, 10.0));
    }
}

impl Default for Mirroring {
    #[inline]
    fn default() -> Self {
        Self {
            x: false,
            y: false,
        }
    }
}

impl From<[bool; 2]> for Mirroring {
    #[inline]
    fn from(value: [bool; 2]) -> Self {
        Self {
            x: value[0],
            y: value[1],
        }
    }
}

impl From<[i8; 2]> for Mirroring {
    #[inline]
    fn from(value: [i8; 2]) -> Self {
        Self {
            x: value[0] != 0,
            y: value[1] != 0,
        }
    }
}

impl From<(bool, bool)> for Mirroring {
    #[inline]
    fn from(value: (bool, bool)) -> Self {
        Self {
            x: value.0,
            y: value.1,
        }
    }
}

impl From<(i8, i8)> for Mirroring {
    #[inline]
    fn from(value: (i8, i8)) -> Self {
        Self {
            x: value.0 != 0,
            y: value.1 != 0,
        }
    }
}

#[cfg(feature = "egui")]
impl From<Vec2b> for Mirroring {
    #[inline]
    fn from(value: Vec2b) -> Self {
        Self {
            x: value.x,
            y: value.y,
        }
    }
}

impl Mirroring {
    #[inline]
    pub fn as_f64(&self) -> [f64; 2] {
        [if self.x { -1.0 } else { 1.0 }, if self.y { -1.0 } else { 1.0 }]
    }

    #[inline]
    pub fn as_f32(&self) -> [f32; 2] {
        [if self.x { -1.0 } else { 1.0 }, if self.y { -1.0 } else { 1.0 }]
    }

    #[inline]
    pub fn as_i8(&self) -> [i8; 2] {
        [if self.x { -1 } else { 1 }, if self.y { -1 } else { 1 }]
    }
}
