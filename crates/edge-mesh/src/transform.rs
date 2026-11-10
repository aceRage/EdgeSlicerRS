/// An affine map laid out as 3MF writes it: a point is a row vector, so
/// `p' = p · L + t`, where `rows[0..3]` are the rows of `L` and `rows[3]` is `t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub rows: [[f64; 3]; 4],
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        rows: [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0],
        ],
    };

    pub fn translation(offset: [f64; 3]) -> Self {
        Self {
            rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], offset],
        }
    }

    pub fn scaling(factor: f64) -> Self {
        Self {
            rows: [
                [factor, 0.0, 0.0],
                [0.0, factor, 0.0],
                [0.0, 0.0, factor],
                [0.0; 3],
            ],
        }
    }

    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let r = &self.rows;
        std::array::from_fn(|c| p[0] * r[0][c] + p[1] * r[1][c] + p[2] * r[2][c] + r[3][c])
    }

    /// The map that applies `self` first and `next` second.
    pub fn then(&self, next: &Transform) -> Transform {
        let (a, b) = (&self.rows, &next.rows);
        let mut rows: [[f64; 3]; 4] = std::array::from_fn(|i| {
            std::array::from_fn(|c| a[i][0] * b[0][c] + a[i][1] * b[1][c] + a[i][2] * b[2][c])
        });
        for (out, offset) in rows[3].iter_mut().zip(b[3]) {
            *out += offset;
        }
        Transform { rows }
    }

    pub fn determinant(&self) -> f64 {
        let r = &self.rows;
        r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
            - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
            + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0])
    }

    /// A mirroring map turns a solid inside out unless triangle winding is
    /// reversed along with it.
    pub fn is_mirrored(&self) -> bool {
        self.determinant() < 0.0
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn then_applies_left_to_right() {
        let rotate_z = Transform {
            rows: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]],
        };
        let shift = Transform::translation([10.0, 0.0, 0.0]);
        let p = [1.0, 0.0, 0.0];
        assert_eq!(rotate_z.apply(p), [0.0, 1.0, 0.0]);
        assert_eq!(rotate_z.then(&shift).apply(p), [10.0, 1.0, 0.0]);
        assert_eq!(shift.then(&rotate_z).apply(p), [0.0, 11.0, 0.0]);
        assert_eq!(
            shift.then(&rotate_z).apply(p),
            rotate_z.apply(shift.apply(p))
        );
        assert_eq!(Transform::IDENTITY.then(&shift), shift);
    }

    #[test]
    fn mirrors_are_detected() {
        let mut mirror = Transform::IDENTITY;
        mirror.rows[0][0] = -1.0;
        assert!(mirror.is_mirrored());
        assert!(!Transform::scaling(25.4).is_mirrored());
        assert_eq!(Transform::scaling(2.0).determinant(), 8.0);
    }
}
