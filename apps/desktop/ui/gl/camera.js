'use strict';

// The camera of the 3D views, as the reference has it (Camera.cpp,
// Camera.hpp): it looks at a target from a distance, its zoom is how many
// pixels a millimetre takes at the target, and the same zoom holds in
// perspective and in orthographic view. Up is +Z.
(function (GL) {
  const M = GL.math;

  // Camera.cpp: DefaultDistance, FrustrumMinZRange, FrustrumMinNearZ,
  // FrustrumZMargin, ZoomUnit; Camera.hpp max_zoom.
  const DEFAULT_DISTANCE = 1000;
  const MIN_Z_RANGE = 50;
  const MIN_NEAR_Z = 100;
  const Z_MARGIN = 10;
  const ZOOM_UNIT = 0.1;
  const MAX_ZOOM = 250;

  class Camera {
    constructor() {
      // Rows: right, up, and back (from the target towards the eye).
      this.rotation = M.rows.identity();
      this.target = [0, 0, 0];
      this.distance = DEFAULT_DISTANCE;
      this.zoom = 1;
      this.zenit = 45;
      this.perspective = true;
      // In CSS pixels.
      this.viewport = [1, 1];
      // What the zoom is kept within sight of.
      this.sceneBox = null;
      this.setDefaultOrientation();
    }

    get right() { return this.rotation[0]; }
    get up() { return this.rotation[1]; }
    get back() { return this.rotation[2]; }
    get forward() { return M.scale(this.rotation[2], -1); }
    get position() { return M.along(this.target, this.back, this.distance); }

    // Camera::look_at.
    lookAt(position, target, up) {
      const z = M.normalize(M.sub(position, target));
      const x = M.normalize(M.cross(up, z));
      const y = M.normalize(M.cross(z, x));
      this.target = target.slice();
      this.distance = M.length(M.sub(position, target));
      this.rotation = [x, y, z];
      this.updateZenit();
    }

    // Camera::update_zenit.
    updateZenit() {
      const down = Math.max(-1, Math.min(1, -this.forward[2]));
      this.zenit = 90 - (Math.acos(down) * 180) / Math.PI;
    }

    // Camera::set_default_orientation: from the front right, 45 degrees up.
    setDefaultOrientation() {
      this.zenit = 45;
      const theta = (-45 * Math.PI) / 180;
      const phi = (45 * Math.PI) / 180;
      this.rotation = M.rows.multiply(M.rows.about([1, 0, 0], theta), M.rows.about([0, 0, 1], phi));
    }

    // Camera::select_view. The reference switches between perspective and
    // orthographic here only with its "Auto Perspective" preference on;
    // `auto` is that preference.
    selectView(name, auto = false) {
      const [t, d] = [this.target, this.distance];
      const from = (x, y, z) => M.add(t, [x * d, y * d, z * d]);
      let flat = true;
      switch (name) {
        case 'iso': this.setDefaultOrientation(); flat = false; break;
        case 'left': this.lookAt(from(-1, 0, 0), t, [0, 0, 1]); break;
        case 'right': this.lookAt(from(1, 0, 0), t, [0, 0, 1]); break;
        case 'top': this.lookAt(from(0, 0, 1), t, [0, 1, 0]); break;
        case 'bottom': this.lookAt(from(0, 0, -1), t, [0, -1, 0]); break;
        case 'front': this.lookAt(from(0, -1, 0), t, [0, 0, 1]); break;
        case 'rear': this.lookAt(from(0, 1, 0), t, [0, 0, 1]); break;
        case 'topfront':
        case 'plate': this.lookAt(from(0, -0.707, 0.707), t, [0, 1, 1]); flat = false; break;
        default: return;
      }
      if (auto) this.perspective = !flat;
    }

    // Camera::rotate_on_sphere_with_target: round the vertical through
    // `pivot` by `azimuth`, and up or down by `zenit`, both in radians.
    // The pivot stays where it is on screen. With `limits` the camera
    // does not go over the top or under the bottom.
    rotateOnSphere(azimuth, zenit, limits, pivot) {
      this.zenit += (zenit * 180) / Math.PI;
      if (limits) {
        if (this.zenit > 90) {
          zenit -= ((this.zenit - 90) * Math.PI) / 180;
          this.zenit = 90;
        } else if (this.zenit < -90) {
          zenit -= ((this.zenit + 90) * Math.PI) / 180;
          this.zenit = -90;
        }
      }
      const eye = this.position;
      const held = M.rows.apply(this.rotation, M.sub(pivot, eye));
      const turn = M.rows.about([0, 0, 1], azimuth);
      const axis = M.normalize(M.rows.unapply(turn, this.right));
      this.rotation = M.rows.tidy(M.rows.multiply(this.rotation, M.rows.multiply(turn, M.rows.about(axis, zenit))));
      // Where the eye must be for the pivot to be seen where it was.
      const moved = M.sub(pivot, M.rows.unapply(this.rotation, held));
      this.target = M.along(moved, this.forward, this.distance);
    }

    // Camera::rotate_local_around_target: about the camera's own up and
    // right through the target, by `yaw` and `pitch` radians, nothing
    // kept upright ("Use free camera").
    rotateFree(yaw, pitch) {
      const turn = M.rows.multiply(M.rows.about(this.up, yaw), M.rows.about(this.right, pitch));
      this.rotation = M.rows.tidy(M.rows.multiply(this.rotation, turn));
      this.updateZenit();
    }

    // Camera::update_zoom: a step of the wheel is a tenth.
    updateZoom(delta) {
      this.setZoom(this.zoom / (1 - Math.max(-4, Math.min(4, delta)) * ZOOM_UNIT));
    }

    // Camera::set_zoom: not so far out that the scene is lost, nor so
    // close that it is passed through.
    setZoom(zoom) {
      const least = this.sceneBox ? 0.2 * this.zoomToFit(this.sceneBox, 1) : 0;
      if (least > 0) zoom = Math.max(zoom, least);
      this.zoom = Math.min(zoom, MAX_ZOOM);
    }

    // Camera::calc_zoom_to_bounding_box_factor: the zoom at which a box
    // fills the view, seen from where the camera looks now.
    zoomToFit(box, margin) {
      const centre = M.centre(box);
      let [minX, minY, maxX, maxY] = [Infinity, Infinity, -Infinity, -Infinity];
      for (const corner of M.corners(box)) {
        const at = M.sub(corner, centre);
        const [x, y] = [M.dot(at, this.right), M.dot(at, this.up)];
        minX = Math.min(minX, x); maxX = Math.max(maxX, x);
        minY = Math.min(minY, y); maxY = Math.max(maxY, y);
      }
      const [dx, dy] = [(maxX - minX) * margin, (maxY - minY) * margin];
      if (dx <= 0 || dy <= 0) return -1;
      return Math.min(this.viewport[0] / dx, this.viewport[1] / dy);
    }

    // Camera::zoom_to_box. `aside` is room, in pixels, taken at the right
    // of the view by what is drawn over it; the box is fitted in the rest.
    zoomToBox(box, margin, aside = 0) {
      const whole = this.viewport[0];
      this.viewport[0] = Math.max(whole - aside, 50);
      const zoom = this.zoomToFit(box, margin);
      this.viewport[0] = whole;
      if (zoom <= 0) return;
      this.zoom = zoom;
      this.target = M.along(M.centre(box), this.right, aside / 2 / zoom);
    }

    // Camera::apply_projection with calc_tight_frustrum_zs_around: the
    // near and far planes hug `box`; in perspective the near plane is
    // sized so that the zoom holds at the target. Moves the eye back when
    // the scene comes too close to it.
    matrices(box) {
      const r = this.rotation;
      let [near, far] = [MIN_NEAR_Z, MIN_NEAR_Z + MIN_Z_RANGE];
      if (box) {
        const eye = this.position;
        let [least, most] = [Infinity, -Infinity];
        for (const corner of M.corners(box)) {
          const depth = -M.dot(r[2], M.sub(corner, eye));
          least = Math.min(least, depth);
          most = Math.max(most, depth);
        }
        near = least - Z_MARGIN;
        far = most + Z_MARGIN;
        if (far - near < MIN_Z_RANGE) {
          const middle = (near + far) / 2;
          near = middle - MIN_Z_RANGE / 2;
          far = middle + MIN_Z_RANGE / 2;
        }
        if (near < MIN_NEAR_Z) {
          const more = MIN_NEAR_Z - near;
          this.distance += more;
          near += more;
          far += more;
        }
      }
      const eye = this.position;
      const view = [
        r[0][0], r[1][0], r[2][0], 0,
        r[0][1], r[1][1], r[2][1], 0,
        r[0][2], r[1][2], r[2][2], 0,
        -M.dot(r[0], eye), -M.dot(r[1], eye), -M.dot(r[2], eye), 1,
      ];
      let w = (0.5 * this.viewport[0]) / this.zoom;
      let h = (0.5 * this.viewport[1]) / this.zoom;
      let projection;
      if (this.perspective) {
        const scale = near / this.distance;
        w *= scale;
        h *= scale;
        projection = [
          near / w, 0, 0, 0,
          0, near / h, 0, 0,
          0, 0, -(near + far) / (far - near), -1,
          0, 0, (-2 * near * far) / (far - near), 0,
        ];
      } else {
        projection = [
          1 / w, 0, 0, 0,
          0, 1 / h, 0, 0,
          0, 0, -2 / (far - near), 0,
          0, 0, -(near + far) / (far - near), 1,
        ];
      }
      return { view, projection, near, far, eye };
    }

    // The point of the plane through the target, square to the view, that
    // a pixel of the view shows.
    atTarget(x, y) {
      const [cx, cy] = [x - this.viewport[0] / 2, this.viewport[1] / 2 - y];
      return M.along(M.along(this.target, this.right, cx / this.zoom), this.up, cy / this.zoom);
    }

    // The line of sight through a pixel: where it starts and which way it goes.
    ray(x, y) {
      const through = this.atTarget(x, y);
      if (this.perspective) {
        const eye = this.position;
        return { origin: eye, direction: M.normalize(M.sub(through, eye)) };
      }
      return { origin: M.along(through, this.back, this.distance), direction: this.forward };
    }

    // Where the line of sight through a pixel meets the plane z = `height`.
    onGround(x, y, height = 0) {
      const { origin, direction } = this.ray(x, y);
      if (Math.abs(direction[2]) < 1e-9) return null;
      const along = (height - origin[2]) / direction[2];
      return along > 0 ? M.along(origin, direction, along) : null;
    }

    // Moves the view by pixels: what was under the pointer on the plane
    // through the target stays under it.
    pan(dx, dy) {
      this.target = M.along(M.along(this.target, this.right, -dx / this.zoom), this.up, dy / this.zoom);
    }

    // Zooms by wheel steps, keeping what is under a pixel under it.
    zoomAt(delta, x, y) {
      const held = this.atTarget(x, y);
      this.updateZoom(delta);
      const [cx, cy] = [x - this.viewport[0] / 2, this.viewport[1] / 2 - y];
      this.target = M.along(M.along(held, this.right, -cx / this.zoom), this.up, -cy / this.zoom);
    }

    save() {
      return {
        rotation: this.rotation.map((row) => row.slice()),
        target: this.target.slice(),
        distance: this.distance,
        zoom: this.zoom,
        zenit: this.zenit,
      };
    }

    load(kept) {
      this.rotation = kept.rotation.map((row) => row.slice());
      this.target = kept.target.slice();
      this.distance = kept.distance;
      this.zoom = kept.zoom;
      this.zenit = kept.zenit;
    }
  }

  GL.Camera = Camera;
})(window.EdgeGL);
