'use strict';

// The 3D views' own arithmetic: points as [x, y, z], matrices as sixteen
// numbers column by column, the way the graphics card takes them. Nothing
// here comes from a library.
window.EdgeGL = window.EdgeGL || {};

(function (GL) {
  const M = {};

  M.add = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
  M.sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
  M.scale = (a, s) => [a[0] * s, a[1] * s, a[2] * s];
  M.dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  M.cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  M.length = (a) => Math.hypot(a[0], a[1], a[2]);
  M.normalize = (a) => {
    const length = M.length(a);
    return length > 0 ? M.scale(a, 1 / length) : [0, 0, 0];
  };
  // a + b * s
  M.along = (a, b, s) => [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s];

  // 3 by 3 rotations as three rows.
  M.rows = {
    identity: () => [[1, 0, 0], [0, 1, 0], [0, 0, 1]],
    apply: (r, v) => [M.dot(r[0], v), M.dot(r[1], v), M.dot(r[2], v)],
    // The transpose applied: for a rotation, its inverse.
    unapply: (r, v) => [
      r[0][0] * v[0] + r[1][0] * v[1] + r[2][0] * v[2],
      r[0][1] * v[0] + r[1][1] * v[1] + r[2][1] * v[2],
      r[0][2] * v[0] + r[1][2] * v[1] + r[2][2] * v[2],
    ],
    multiply: (a, b) => a.map((row) => [0, 1, 2].map((c) => row[0] * b[0][c] + row[1] * b[1][c] + row[2] * b[2][c])),
    // A turn of `angle` radians about a unit `axis`.
    about: (axis, angle) => {
      const [x, y, z] = axis;
      const [c, s] = [Math.cos(angle), Math.sin(angle)];
      const t = 1 - c;
      return [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
      ];
    },
    // Rows made square to one another again after many small turns.
    tidy: (r) => {
      const z = M.normalize(r[2]);
      const x = M.normalize(M.cross(r[1], z));
      return [x, M.cross(z, x), z];
    },
  };

  M.identity = () => [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
  M.translation = (x, y, z) => [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, x, y, z, 1];
  M.scaling = (x, y, z) => [x, 0, 0, 0, 0, y, 0, 0, 0, 0, z, 0, 0, 0, 0, 1];
  // a after b: (a * b) v = a (b v).
  M.multiply = (a, b) => {
    const out = new Array(16);
    for (let c = 0; c < 4; c += 1) {
      for (let r = 0; r < 4; r += 1) {
        out[c * 4 + r] = a[r] * b[c * 4] + a[4 + r] * b[c * 4 + 1] + a[8 + r] * b[c * 4 + 2] + a[12 + r] * b[c * 4 + 3];
      }
    }
    return out;
  };
  M.chain = (...matrices) => matrices.reduce(M.multiply);
  M.point = (m, p) => {
    const w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
    return [
      (m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12]) / w,
      (m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13]) / w,
      (m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14]) / w,
    ];
  };
  M.direction = (m, v) => [
    m[0] * v[0] + m[4] * v[1] + m[8] * v[2],
    m[1] * v[0] + m[5] * v[1] + m[9] * v[2],
    m[2] * v[0] + m[6] * v[1] + m[10] * v[2],
  ];
  M.determinant3 = (m) => m[0] * (m[5] * m[10] - m[6] * m[9]) - m[4] * (m[1] * m[10] - m[2] * m[9]) + m[8] * (m[1] * m[6] - m[2] * m[5]);
  // The upper 3 by 3 of a matrix, inverted and transposed, as nine numbers
  // column by column: what takes normals where the matrix takes points.
  M.normalMatrix = (m) => {
    const [a, b, c, d, e, f, g, h, i] = [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]];
    const det = a * (e * i - h * f) - d * (b * i - h * c) + g * (b * f - e * c);
    const s = det !== 0 ? 1 / det : 0;
    return [
      (e * i - h * f) * s, (g * f - d * i) * s, (d * h - g * e) * s,
      (h * c - b * i) * s, (a * i - g * c) * s, (g * b - a * h) * s,
      (b * f - e * c) * s, (d * c - a * f) * s, (a * e - d * b) * s,
    ];
  };

  // The matrix that undoes an affine one (its last row 0, 0, 0, 1); the
  // identity for one that squashes space flat.
  M.invert = (m) => {
    const n = M.normalMatrix(m);
    if (!n.some((value) => value !== 0)) return M.identity();
    // `normalMatrix` is the inverse transposed: transposed back.
    const out = [n[0], n[3], n[6], 0, n[1], n[4], n[7], 0, n[2], n[5], n[8], 0, 0, 0, 0, 1];
    const t = M.direction(out, [m[12], m[13], m[14]]);
    out[12] = -t[0];
    out[13] = -t[1];
    out[14] = -t[2];
    return out;
  };

  // The eight corners of a box [[x, y, z], [x, y, z]].
  M.corners = (box) => Array.from({ length: 8 }, (_, at) => [0, 1, 2].map((axis) => box[(at >> axis) & 1][axis]));
  M.union = (a, b) => {
    if (!a) return b;
    if (!b) return a;
    return [a[0].map((v, i) => Math.min(v, b[0][i])), a[1].map((v, i) => Math.max(v, b[1][i]))];
  };
  M.centre = (box) => [0, 1, 2].map((axis) => (box[0][axis] + box[1][axis]) / 2);

  GL.math = M;
})(window.EdgeGL);
