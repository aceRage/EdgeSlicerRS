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

    /// The map that undoes this one; none when it squashes space flat.
    pub fn inverse(&self) -> Option<Transform> {
        let det = self.determinant();
        if !det.is_finite() || det.abs() < 1e-300 {
            return None;
        }
        let r = &self.rows;
        // The adjugate over the determinant, for the linear part.
        let cofactor =
            |a: usize, b: usize, c: usize, d: usize| r[a][b] * r[c][d] - r[a][d] * r[c][b];
        let linear = [
            [
                cofactor(1, 1, 2, 2) / det,
                -cofactor(0, 1, 2, 2) / det,
                cofactor(0, 1, 1, 2) / det,
            ],
            [
                -cofactor(1, 0, 2, 2) / det,
                cofactor(0, 0, 2, 2) / det,
                -cofactor(0, 0, 1, 2) / det,
            ],
            [
                cofactor(1, 0, 2, 1) / det,
                -cofactor(0, 0, 2, 1) / det,
                cofactor(0, 0, 1, 1) / det,
            ],
        ];
        let t = r[3];
        let offset: [f64; 3] = std::array::from_fn(|c| {
            -(t[0] * linear[0][c] + t[1] * linear[1][c] + t[2] * linear[2][c])
        });
        Some(Transform {
            rows: [linear[0], linear[1], linear[2], offset],
        })
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
    fn inverse_undoes() {
        let map = Transform {
            rows: [
                [0.0, 2.0, 0.0],
                [-1.0, 0.0, 0.5],
                [0.0, 0.0, -3.0],
                [4.0, -5.0, 6.0],
            ],
        };
        let back = map.inverse().expect("not flat");
        for p in [[1.0, 2.0, 3.0], [-7.0, 0.5, 11.0]] {
            let there = back.apply(map.apply(p));
            for axis in 0..3 {
                assert!((there[axis] - p[axis]).abs() < 1e-12, "{there:?} {p:?}");
            }
        }
        assert!(Transform::scaling(0.0).inverse().is_none());
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
