'use strict';

// "Cut"'s connectors (Gizmos/GLGizmoCut.cpp: "Add connectors" / "Edit
// connectors", `render_connectors_input_window`, `add_connector`,
// `gizmo_event`, `check_and_update_connectors_state`, `render_connectors`):
// plugs, dowels and snaps placed on the cut face by a click, chosen,
// moved, taken off, and their settings. Each is kept in the plane's own
// coordinates, so it goes with the plane as it moves, flips or is
// mirrored; the cut (`scene_edit` `cut` with `connectors`) makes them.
// Added to `GL.cutter` (cut.js).
(function (GL) {
  const M = GL.math;
  const cutter = GL.cutter;
  const sub = M.sub;

  // GLGizmoCut.cpp l.39–46: plugs yellow, dowels dark yellow, cyan (dark
  // cyan) under the pointer, grey (dark grey) chosen; white over the face
  // while placed, pink where it does not fit, and pink as a hole once the
  // connectors are confirmed.
  const PLUG = [1, 1, 0, 1];
  const DOWEL = [0.5, 0.5, 0, 1];
  const HOVERED_PLUG = [0, 1, 1, 1];
  const HOVERED_DOWEL = [0, 0.5, 0.5, 1];
  const SELECTED_PLUG = [0.5, 0.5, 0.5, 1];
  const SELECTED_DOWEL = [0.25, 0.25, 0.25, 1];
  const ERROR = [1, 0.3, 0.3, 0.5];
  const HOVERED_ERROR = [1, 0.3, 0.3, 1];
  const CAP = [0.8, 0.8, 0.8, 1];

  // `validate_connector_settings`: what each setting starts at.
  const START = { depth: 3, depthTolerance: 0.1, size: 2.5, sizeTolerance: 0, angle: 0 };
  const SNAP_START = { space: 0.3, bulge: 0.15 };
  const TYPES = ['plug', 'dowel', 'snap', 'pin'];
  const STYLES = ['prism', 'frustum'];
  const SHAPES = ['triangle', 'square', 'hexagon', 'circle'];
  const SIDES = { triangle: 3, square: 4, hexagon: 6, circle: 60 };
  const fixed = (value) => (Math.round(value * 100) / 100).toFixed(2);

  Object.assign(cutter, {
    // The settings new connectors take, kept from one cut to the next.
    connector: { type: 'plug', style: 'prism', shape: 'circle', ...START },
    snap: { ...SNAP_START },
    // The pin's shape new pins take (`m_pushfit`), what the session says of
    // it, and the panel's own choices (`m_pushfit_same_length` and the rest).
    pin: null,
    pinInfo: null,
    pinUi: { same_length: true, same_clearance: true, advanced: false, misfit: false },
    // Each shape's mesh at unit size, by its kind, style and shape.
    connectorMeshes: new Map(),

    connectorsOf() {
      return (this.open && this.open.connectors) || [];
    },

    // Whether the connectors can be added: both parts kept as objects of
    // their own, not every piece on one side, a planar cut.
    connectorsAllowed() {
      const p = this.options;
      const parts = this.open && this.open.parts;
      // Not on a bent surface: placing them on it is not done here yet.
      return !this.dovetail() && !this.bent() && !this.drawing() && p.keep_upper && p.keep_lower && !p.to_parts
        && !(parts && parts.sides.every((side) => side === parts.sides[0]));
    },

    // A connector's point on the plane, in the scene.
    connectorAt(c) {
      const o = this.open;
      return M.add(M.along(o.middle, o.axes[0], c.x), M.scale(o.axes[1], c.y));
    },

    // The connectors as the session takes them.
    connectorsArgs() {
      return this.connectorsOf().map((c) => ({
        at: this.connectorAt(c), radius: c.radius, height: c.height, radius_tolerance: c.radius_tolerance,
        height_tolerance: c.height_tolerance, z_angle: c.z_angle, type: c.type, style: c.style, shape: c.shape,
        pin: c.type === 'pin' ? c.pin : null,
      }));
    },

    // Connectors given in the scene (a recipe's), into the plane's own
    // coordinates.
    takeConnectors(list) {
      const o = this.open;
      o.connectors = (list || []).map((c) => {
        const out = sub(c.at, o.middle);
        return { ...c, x: M.dot(out, o.axes[0]), y: M.dot(out, o.axes[1]) };
      });
      o.selected = new Set();
    },

    // The plane flipped or mirrored: the connectors stay where they are in
    // the scene, a mirrored outline turned the other way.
    connectorsMirrored(mirror) {
      for (const c of this.connectorsOf()) {
        c.y = -c.y;
        if (mirror && c.shape !== 'circle') c.z_angle = -c.z_angle;
      }
    },

    // ---- editing ----

    async startConnectors() {
      const o = this.open;
      if (!o || !this.connectorsAllowed()) return;
      o.picking = false;
      o.editing = true;
      o.selected = new Set();
      this.changed();
      await this.fetchCap();
    },

    // The cut face, for the connectors to stand on (`render_clipper_cut`).
    async fetchCap() {
      const o = this.open;
      const project = this.host.project();
      const key = JSON.stringify(this.frame());
      if (o.cap && o.cap.key === key) return;
      let answer;
      try {
        answer = await this.host.call('cut_section', { project: project.id, copy: o.copy, plane: this.frame(), cap: true });
      } catch (error) {
        this.host.say(error.message, true);
        return;
      }
      if (this.open !== o || JSON.stringify(this.frame()) !== key) return;
      this.dropCap();
      const points = answer.cap || [];
      // In the plane's own coordinates, for what a click lands on.
      const flat = [];
      for (let at = 0; at + 2 < points.length; at += 3) {
        const out = sub(points.slice(at, at + 3), o.middle);
        flat.push(M.dot(out, o.axes[0]), M.dot(out, o.axes[1]));
      }
      // Drawn a hair over the plane, toward the eye side kept.
      const lift = M.scale(o.axes[2], 0.02);
      const shown = [];
      for (let at = 0; at + 2 < points.length; at += 3) shown.push(...M.add(points.slice(at, at + 3), lift));
      o.cap = { key, flat, mesh: shown.length ? this.stage.renderer.triangles(shown) : null };
      this.checkConnectors();
      this.panel();
      this.stage.frame();
    },

    dropCap() {
      const o = this.open;
      if (o && o.cap && o.cap.mesh) this.stage.renderer.drop(o.cap.mesh);
      if (o) o.cap = null;
    },

    // "Confirm connectors".
    confirmConnectors() {
      const o = this.open;
      o.selected = new Set();
      o.editing = false;
      this.changed();
    },

    // "Cancel": every connector taken off (`reset_connectors`).
    cancelConnectors() {
      const o = this.open;
      o.connectors = [];
      o.selected = new Set();
      o.editing = false;
      this.changed();
    },

    // "Remove connectors".
    removeConnectors() {
      const o = this.open;
      o.connectors = [];
      o.selected = new Set();
      this.checkConnectors();
      this.panel();
      this.stage.frame();
    },

    // Whether a point of the plane, in its own coordinates, is on the cut
    // face.
    onCap(x, y) {
      const flat = this.open.cap ? this.open.cap.flat : [];
      for (let at = 0; at + 5 < flat.length; at += 6) {
        const [ax, ay, bx, by, cx, cy] = flat.slice(at, at + 6);
        const d1 = (x - bx) * (ay - by) - (ax - bx) * (y - by);
        const d2 = (x - cx) * (by - cy) - (bx - cx) * (y - cy);
        const d3 = (x - ax) * (cy - ay) - (cx - ax) * (y - ay);
        const negative = d1 < 0 || d2 < 0 || d3 < 0;
        const positive = d1 > 0 || d2 > 0 || d3 > 0;
        if (!(negative && positive)) return true;
      }
      return false;
    },

    // Where the line of sight through a pixel meets the plane, in its own
    // coordinates.
    planeAt(x, y) {
      const o = this.open;
      const n = o.axes[2];
      const ray = this.stage.camera.ray(x, y);
      const toward = M.dot(ray.direction, n);
      if (Math.abs(toward) < 1e-9) return null;
      const point = M.along(ray.origin, ray.direction, M.dot(sub(o.middle, ray.origin), n) / toward);
      const out = sub(point, o.middle);
      return [M.dot(out, o.axes[0]), M.dot(out, o.axes[1])];
    },

    // The connector under a pixel: the nearest whose outline the line of
    // sight meets on the plane.
    connectorUnder(x, y) {
      const hit = this.planeAt(x, y);
      if (!hit) return -1;
      let best = -1;
      let near = Infinity;
      this.connectorsOf().forEach((c, at) => {
        const far = Math.hypot(hit[0] - c.x, hit[1] - c.y);
        if (far <= c.radius && far < near) { best = at; near = far; }
      });
      return best;
    },

    // `check_and_update_connectors_state`: which do not fit, and why.
    checkConnectors() {
      const o = this.open;
      const list = this.connectorsOf();
      const stats = { outside: 0, outsideBox: 0, overlap: false };
      const bad = new Set();
      const reach = o.section && o.section.reach;
      const [lo, hi] = this.faces();
      list.forEach((c, at) => {
        // Its outline on the cut face (`is_outside_of_cut_contour`; the
        // reference leaves its turn out, this measures it as drawn).
        if (o.cap) {
          const sides = SIDES[c.shape];
          let outside = !this.onCap(c.x, c.y);
          for (let k = 0; k < sides && !outside; k += 1) {
            const a = (2 * Math.PI * k) / sides - c.z_angle;
            outside = !this.onCap(c.x - c.radius * Math.sin(a), c.y + c.radius * Math.cos(a));
          }
          if (outside) { stats.outside += 1; bad.add(at); return; }
        }
        // Within the object's box (`m_bounding_box.contains`).
        if (reach) {
          // A dowel reaches its height each way, a plug or a snap over the
          // lower part's face; each as long again as the cut is thick; a pin
          // its two lengths.
          const [from, to] = c.type === 'pin'
            ? [-c.pin.length_lower + Math.min(lo, 0), c.pin.length_upper + Math.max(hi, 0)]
            : c.type === 'dowel'
            ? [-c.height - (hi - lo) / 2, c.height + (hi - lo) / 2]
            : [lo, lo + c.height + (hi - lo)];
          const inside = c.x - c.radius >= reach[0][0] - 1e-6 && c.x + c.radius <= reach[1][0] + 1e-6
            && c.y - c.radius >= reach[0][1] - 1e-6 && c.y + c.radius <= reach[1][1] + 1e-6
            && from >= reach[0][2] - 1e-6 && to <= reach[1][2] + 1e-6;
          if (!inside) { stats.outsideBox += 1; bad.add(at); return; }
        }
        // Not over another (`is_conflict_for_connector`).
        for (let other = 0; other < list.length; other += 1) {
          if (other !== at && Math.hypot(list[other].x - c.x, list[other].y - c.y) < list[other].radius + c.radius) {
            stats.overlap = true;
            bad.add(at);
            return;
          }
        }
      });
      // A pin that cannot be made (`can_perform_cut`: `pushfit_validate`).
      list.forEach((c, at) => {
        if (c.type === 'pin' && c.pinInvalid) { bad.add(at); stats.badPin = c.pinInvalid; }
      });
      o.invalid = bad;
      o.stats = stats;
    },

    // The warning lines (`render_input_window_warning`).
    connectorWarnings() {
      const o = this.open;
      if (!o || !o.invalid || !o.invalid.size) return [];
      const lines = [`${t('Warning')}: ${t('Invalid connectors detected')}:`];
      const s = o.stats;
      if (s.outside) lines.push(` - ${tn('%1$d connector is out of cut contour', '%1$d connectors are out of cut contour', s.outside).replace('%1$d', String(s.outside))}`);
      if (s.outsideBox) lines.push(` - ${tn('%1$d connector is out of object', '%1$d connectors are out of object', s.outsideBox).replace('%1$d', String(s.outsideBox))}`);
      if (s.overlap) lines.push(` - ${t('Some connectors are overlapped')}`);
      if (s.badPin) lines.push(` - ${s.badPin}`);
      return [lines.join('\n')];
    },

    // A new connector where a click lands on the cut face (`add_connector`).
    addConnector(x, y) {
      const hit = this.planeAt(x, y);
      if (!hit || !this.onCap(hit[0], hit[1])) return false;
      const o = this.open;
      const s = this.connector;
      const pin = s.type === 'pin' ? { ...this.pinShape() } : null;
      o.connectors.push({
        x: hit[0], y: hit[1], radius: pin ? pin.diameter / 2 : s.size / 2, height: pin ? pin.length_upper + pin.length_lower : s.depth,
        radius_tolerance: pin ? 0 : s.sizeTolerance / 2, height_tolerance: pin ? 0 : s.depthTolerance, z_angle: s.angle,
        type: s.type, style: pin ? 'prism' : s.style, shape: pin ? 'circle' : s.shape, pin,
        pinInvalid: pin && this.pinInfo ? this.pinInfo.invalid : null,
      });
      o.selected = new Set([o.connectors.length - 1]);
      this.connectorsChanged();
      return true;
    },

    deleteSelected() {
      const o = this.open;
      if (!o.selected.size) return false;
      o.connectors = o.connectors.filter((_, at) => !o.selected.has(at));
      o.selected = new Set();
      this.connectorsChanged();
      return true;
    },

    connectorsChanged() {
      this.checkConnectors();
      this.panel();
      this.stage.frame();
    },

    // A setting changed: the default for new connectors, and every chosen
    // one (`apply_selected_connectors`).
    setConnector(key, value) {
      const o = this.open;
      const s = this.connector;
      s[key] = value;
      // A dowel is a prism; a snap is round (`render_connectors_input_window`).
      if (key === 'type' && value === 'dowel') s.style = 'prism';
      if (key === 'type' && value === 'snap') s.shape = 'circle';
      if (key === 'type' && value === 'pin' && !this.pin) this.askPin({});
      if (s.type === 'snap' && s.depth < s.size) s.depth = s.size;
      for (const at of o.selected) {
        const c = o.connectors[at];
        if (!c) continue;
        if (key === 'type') {
          c.type = s.type; c.style = s.style; c.shape = s.shape;
          if (c.type === 'pin') this.pinOnto(c);
        }
        else if (key === 'style') c.style = s.style;
        else if (key === 'shape') c.shape = s.shape;
        else if (key === 'depth') c.height = s.depth;
        else if (key === 'depthTolerance') c.height_tolerance = s.depthTolerance;
        else if (key === 'size') c.radius = s.size / 2;
        else if (key === 'sizeTolerance') c.radius_tolerance = s.sizeTolerance / 2;
        else if (key === 'angle') c.z_angle = s.angle;
      }
      this.connectorsChanged();
    },

    // The fields show the first chosen connector's settings
    // (`init_input_window_data`).
    showChosen() {
      const o = this.open;
      const first = [...o.selected][0];
      const c = o.connectors[first];
      if (!c) return;
      Object.assign(this.connector, {
        type: c.type, style: c.style, shape: c.shape, depth: c.height, depthTolerance: c.height_tolerance,
        size: 2 * c.radius, sizeTolerance: 2 * c.radius_tolerance, angle: c.z_angle,
      });
      // A chosen pin's shape fills the pin's fields.
      if (c.type === 'pin' && c.pin) {
        this.pin = { ...c.pin };
        this.pinUi.same_length = c.pin.length_upper === c.pin.length_lower;
        this.pinUi.same_clearance = c.pin.clearance_upper === c.pin.clearance_lower;
        this.askPin({});
      }
    },

    // ---- the pointer while editing ----

    connectorHandle(x, y) {
      const at = this.connectorUnder(x, y);
      return at >= 0 ? { kind: 'cut', part: 'connector', index: at, id: `cut-connector-${at}` } : null;
    },

    connectorGrab(x, y, event) {
      const o = this.open;
      const handle = this.connectorHandle(x, y);
      if (handle) {
        const grab = { kind: 'cut', part: 'connector', handle, x, y };
        // A click chooses it: alone, with Shift as well, with Alt not
        // (`is_selection_changed`).
        grab.tap = () => {
          if (event.altKey) o.selected.delete(handle.index);
          else {
            if (!event.shiftKey) o.selected = new Set();
            o.selected.add(handle.index);
          }
          this.showChosen();
          this.connectorsChanged();
        };
        return grab;
      }
      // A click on the face adds one; elsewhere lets the choice go.
      return { kind: 'cut', part: 'add', x, y, tap: () => {
        if (event.shiftKey || event.altKey) return;
        if (!this.addConnector(x, y)) { o.selected = new Set(); this.connectorsChanged(); }
      } };
    },

    // Dragging a connector moves it over the plane ("Move connector").
    connectorDrag(grab, x, y) {
      const o = this.open;
      if (grab.part !== 'connector') { grab.cancelled = true; return; }
      const c = o.connectors[grab.handle.index];
      const hit = this.planeAt(x, y);
      if (!c || !hit) return;
      if (!grab.begun) {
        grab.begun = true;
        grab.from = [c.x, c.y];
        grab.start = hit;
        this.held = grab;
      }
      c.x = grab.from[0] + hit[0] - grab.start[0];
      c.y = grab.from[1] + hit[1] - grab.start[1];
      this.checkConnectors();
      this.stage.frame();
    },

    connectorDrop(grab, done) {
      this.held = null;
      const c = this.open.connectors[grab.handle ? grab.handle.index : -1];
      if (c && grab.begun && !done) { c.x = grab.from[0]; c.y = grab.from[1]; }
      this.connectorsChanged();
    },

    // The right button takes the connector under it off ("Remove
    // connector").
    connectorRightClick(x, y) {
      const at = this.connectorUnder(x, y);
      if (at < 0) return false;
      this.open.selected = new Set([at]);
      return this.deleteSelected();
    },

    // Delete takes the chosen ones off; Ctrl+A chooses them all.
    connectorKey(event) {
      const o = this.open;
      if (event.target.matches('input, select, textarea')) return false;
      if ((event.key === 'Delete' || event.key === 'Backspace') && !event.ctrlKey && !event.metaKey) return this.deleteSelected();
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') {
        o.selected = new Set(o.connectors.map((_, at) => at));
        this.showChosen();
        this.connectorsChanged();
        return true;
      }
      return false;
    },

    // ---- drawing ----

    meshKey(c) {
      return `${c.type}|${c.style}|${c.shape}|${c.type === 'snap' ? `${this.snap.space}|${this.snap.bulge}` : ''}|${c.type === 'pin' ? JSON.stringify(c.pin) : ''}`;
    },

    async connectorMesh(c, hole = null) {
      const key = `${this.meshKey(c)}|${hole || ''}`;
      const held = this.connectorMeshes.get(key);
      if (held) return held.mesh || null;
      const entry = { mesh: null };
      this.connectorMeshes.set(key, entry);
      try {
        const bytes = await this.stage.bytes('cut_connector_mesh', { type: c.type, style: c.style, shape: c.shape, snap: this.snap, pin: c.type === 'pin' ? c.pin : null, hole });
        entry.mesh = this.stage.renderer.meshFromBytes(bytes);
        this.stage.frame();
      } catch (error) {
        this.connectorMeshes.delete(key);
      }
      return null;
    },

    // Where a connector's shape stands, as the cut makes it: a plug or a
    // snap over the plane from the lower part's face, a dowel each way.
    connectorMatrix(c) {
      const o = this.open;
      const [u, v, n] = o.axes;
      const [lo, hi] = this.faces();
      const kerf = hi - lo;
      let height;
      let middle;
      // A pin's shape is in mm, straddling the cut.
      if (c.type === 'pin') {
        const turn = -c.z_angle;
        const [cs, sn] = [Math.cos(turn), Math.sin(turn)];
        const x = M.add(M.scale(u, cs), M.scale(v, sn));
        const y = M.add(M.scale(u, -sn), M.scale(v, cs));
        return [...x, 0, ...y, 0, ...n, 0, ...this.connectorAt(c), 1];
      }
      if (c.type === 'dowel') {
        height = (c.style === 'prism' ? 2 * c.height : c.height) + kerf;
        middle = this.connectorAt(c);
      } else {
        height = c.height + kerf;
        middle = M.along(this.connectorAt(c), n, lo + height / 2);
      }
      const turn = -c.z_angle;
      const [cs, sn] = [Math.cos(turn), Math.sin(turn)];
      const x = M.add(M.scale(u, cs), M.scale(v, sn));
      const y = M.add(M.scale(u, -sn), M.scale(v, cs));
      return M.chain([...x, 0, ...y, 0, ...n, 0, ...middle, 1], M.scaling(c.radius, c.radius, height));
    },

    drawConnectors(frame) {
      const o = this.open;
      const r = this.stage.renderer;
      const gl = r.gl;
      const view = M.multiply(frame.projection, frame.view);
      if (o.editing && o.cap && o.cap.mesh) {
        gl.enable(gl.DEPTH_TEST);
        r.drawFlat(o.cap.mesh, view, CAP);
      }
      const hot = this.editor.hot;
      r.blend(true);
      this.connectorsOf().forEach((c, at) => {
        const mesh = this.connectorMeshes.get(`${this.meshKey(c)}|`);
        if (!mesh || !mesh.mesh) { this.connectorMesh(c); return; }
        const dowel = c.type === 'dowel' || c.type === 'pin';
        const invalid = o.invalid && o.invalid.has(at);
        let colour = invalid ? ERROR : dowel ? DOWEL : PLUG;
        if (!o.editing) colour = ERROR;
        else if (hot === `cut-connector-${at}`) colour = invalid ? HOVERED_ERROR : dowel ? HOVERED_DOWEL : HOVERED_PLUG;
        else if (o.selected.has(at)) colour = dowel ? SELECTED_DOWEL : SELECTED_PLUG;
        r.drawMesh(mesh.mesh, frame, this.connectorMatrix(c), { tint: colour, alpha: colour[3] });
        // A pin being placed shows its holes round it, faint
        // (`render_pushfit_hole_envelopes`).
        if (c.type === 'pin' && o.editing) {
          gl.depthMask(false);
          for (const side of ['upper', 'lower']) {
            const hole = this.connectorMeshes.get(`${this.meshKey(c)}|${side}`);
            if (!hole) { this.connectorMesh(c, side); continue; }
            if (hole.mesh) r.drawMesh(hole.mesh, frame, this.connectorMatrix(c), { tint: [0.3, 0.65, 1, 0.22], alpha: 0.22 });
          }
          gl.depthMask(true);
        }
      });
      r.blend(false);
    },

    // ---- the panel while editing (`render_connectors_input_window`) ----

    connectorRows(busy) {
      const { el } = this.host;
      const o = this.open;
      const s = this.connector;
      const mm = t('mm');
      const rows = [];
      const row = (label, ...cells) => el('div', { className: 'cut-row' }, el('span', { className: 'edit-label', textContent: label }), el('span', { className: 'cut-line' }, ...cells));
      const remove = el('button', { type: 'button', className: 'icon tool', title: t('Remove connectors'), disabled: busy || !o.connectors.length, textContent: '↺' });
      remove.setAttribute('aria-label', t('Remove connectors'));
      remove.addEventListener('click', () => this.removeConnectors());
      rows.push(el('div', { className: 'cut-row' }, el('span', { className: 'edit-label cut-group', textContent: t('Connectors') }), el('span', { className: 'cut-line' }, remove)));
      // "Type": one of three, as joined buttons.
      const choose = (label, key, list, words, disabled) => {
        const group = el('div', { className: 'joined segmented' });
        group.setAttribute('role', 'group');
        group.setAttribute('aria-label', label);
        list.forEach((value, at) => {
          const button = el('button', { type: 'button', textContent: words[at], disabled: busy || disabled });
          button.setAttribute('aria-pressed', String(s[key] === value));
          button.addEventListener('click', () => this.setConnector(key, value));
          group.append(button);
        });
        return row(label, group);
      };
      rows.push(choose(t('Type'), 'type', TYPES, [t('Plug'), t('Dowel'), t('Snap'), t('Pin')], false));
      if (s.type === 'pin') {
        rows.push(...this.pinRows(busy, row));
        return this.connectorFoot(rows, busy);
      }
      const select = (label, key, list, words, disabled) => {
        const input = el('select', { disabled: busy || disabled });
        input.setAttribute('aria-label', label);
        list.forEach((value, at) => input.append(new Option(words[at], value, false, s[key] === value)));
        input.addEventListener('change', () => this.setConnector(key, input.value));
        return row(label, input);
      };
      rows.push(select(t('Style'), 'style', STYLES, [t('Prism'), t('Frustum')], s.type !== 'plug'));
      rows.push(select(t('Shape'), 'shape', SHAPES, [t('Triangle'), t('Square'), t('Hexagon'), t('Circle')], s.type === 'snap'));
      // "Depth" and "Size", each with its tolerance (`render_slider_two_input`:
      // from 1, or the size for a snap's depth, to a ninth of the object's
      // box's sides together; a tolerance to half that).
      const box = o.box;
      const mean = (box[1][0] - box[0][0] + box[1][1] - box[0][1] + box[1][2] - box[0][2]) / 9;
      const reset = (label, keys) => {
        const changed = keys.some((key) => Math.abs((key in START ? START[key] : 0) - s[key]) > 1e-9);
        const node = el('button', { type: 'button', className: 'icon tool', title: `${t('Reset')}: ${label}`, disabled: busy || !changed, textContent: '↺' });
        node.setAttribute('aria-label', `${t('Reset')}: ${label}`);
        node.addEventListener('click', () => { for (const key of keys) this.setConnector(key, START[key]); });
        return node;
      };
      const number = (label, key, min, max, { step = 0.05, scale = 1, unit = mm } = {}) => {
        const input = el('input', { type: 'number', min: String(min * scale), max: String(max * scale), step: String(step), value: fixed(s[key] * scale), disabled: busy });
        input.dataset.field = `connector-${key}`;
        input.setAttribute('aria-label', label);
        input.title = label;
        input.addEventListener('change', () => {
          const value = Number(input.value) / scale;
          if (!Number.isFinite(value)) { this.panel(); return; }
          this.setConnector(key, Math.min(max, Math.max(min, value)));
        });
        return el('span', { className: 'unit' }, input, unit ? ` ${unit}` : '');
      };
      const tolerance = t('Tolerance');
      const top = Math.max(mean, 1);
      rows.push(row(t('Depth'), number(t('Depth'), 'depth', s.type === 'snap' ? s.size : 1, Math.max(top, s.size)), number(`${t('Depth')}: ${tolerance}`, 'depthTolerance', 0, 0.5 * top), reset(t('Depth'), ['depth', 'depthTolerance'])));
      rows.push(row(t('Size'), number(t('Size'), 'size', 1, top), number(`${t('Size')}: ${tolerance}`, 'sizeTolerance', 0, 0.5 * top), reset(t('Size'), ['size', 'sizeTolerance'])));
      rows.push(row(t('Rotation'), number(t('Rotation'), 'angle', 0, Math.PI, { step: 1, scale: 180 / Math.PI, unit: '°' }), reset(t('Rotation'), ['angle'])));
      // A snap's bulge and slit, in percent of its radius
      // (`render_snap_specific_input`).
      if (s.type === 'snap') {
        const percent = (label, tip, key, min, max) => {
          const input = el('input', { type: 'number', min: String(min), max: String(max), step: '1', value: String(Math.round(this.snap[key] * 100)), title: tip, disabled: busy });
          input.setAttribute('aria-label', label);
          input.addEventListener('change', () => {
            const value = Number(input.value);
            if (Number.isFinite(value)) this.snap[key] = Math.min(max, Math.max(min, value)) / 100;
            this.connectorsChanged();
          });
          const back = el('button', { type: 'button', className: 'icon tool', title: `${t('Reset')}: ${label}`, disabled: busy || Math.abs(this.snap[key] - SNAP_START[key]) < 1e-9, textContent: '↺' });
          back.setAttribute('aria-label', `${t('Reset')}: ${label}`);
          back.addEventListener('click', () => { this.snap[key] = SNAP_START[key]; this.connectorsChanged(); });
          return row(label, el('span', { className: 'unit' }, input, ' %'), back);
        };
        rows.push(percent(t('Bulge'), t('Bulge proportion related to radius'), 'bulge', 5, 100 * this.snap.space));
        rows.push(percent(t('Space'), t('Space proportion related to radius'), 'space', 10, 50));
      }
      return this.connectorFoot(rows, busy);
    },

    // The keys, the warnings and the buttons at the foot.
    connectorFoot(rows, busy) {
      const { el } = this.host;
      // What the keys and the pointer do (`m_shortcuts_connector`).
      const help = el('div', { className: 'paint-help' });
      const shift = t('Shift+');
      const alt = t('Alt+');
      for (const [key, does] of [
        [t('Left click'), t('Add connector')],
        [t('Right click'), t('Remove connector')],
        [t('Drag'), t('Move connector')],
        [`${shift}${t('Left click')}`, t('Add connector to selection')],
        [`${alt}${t('Left click')}`, t('Remove connector from selection')],
        [`${t('Ctrl+')}A`, t('Select all connectors')],
      ]) help.append(el('span', { className: 'paint-key', textContent: key }), el('span', { textContent: does }));
      rows.push(help);
      for (const warning of this.connectorWarnings()) rows.push(el('p', { className: 'edit-hint cut-warning', role: 'status', textContent: warning }));
      const confirm = el('button', { type: 'button', className: 'primary', textContent: t('Confirm connectors'), disabled: busy });
      confirm.addEventListener('click', () => this.confirmConnectors());
      const cancel = el('button', { type: 'button', textContent: t('Cancel'), disabled: busy });
      cancel.addEventListener('click', () => this.cancelConnectors());
      rows.push(el('div', { className: 'cut-foot' }, cancel, confirm));
      return rows;
    },

    // ---- the pin (`render_pushfit_inputs`) ----

    // The pin's shape new pins take: the session's default until it is set.
    pinShape() {
      return this.pin || {};
    },

    // Asks the session about the pin's shape (a preset, `auto`, or the
    // shape as it is), takes the shape back and gives it to the chosen
    // pins.
    async askPin({ preset = null } = {}) {
      const o = this.open;
      const settled = this.host.settled();
      const nozzles = settled && settled.printer && settled.printer.nozzle_diameter;
      const nozzle = Number(Array.isArray(nozzles) ? nozzles[0] : String(nozzles || '').split(/[;,]/)[0]) || 0.4;
      const reach = o && o.section && o.section.reach;
      // `flexi_section_inscribed_radius`: half the section's narrower side.
      const inscribed = reach ? 0.5 * Math.min(reach[1][0] - reach[0][0], reach[1][1] - reach[0][1]) : 0;
      let info;
      try {
        info = await this.host.call('cut_pin', { pin: this.pin || {}, preset, nozzle, inscribed });
      } catch (error) {
        this.host.say(error.message, true);
        return;
      }
      this.nozzle = nozzle;
      this.pin = info.pin;
      this.pinInfo = info;
      if (preset !== null) this.pinUi.misfit = !info.fits;
      if (this.open) for (const at of this.open.selected) {
        const c = this.open.connectors[at];
        if (c && c.type === 'pin') this.pinOnto(c);
      }
      this.connectorsChanged();
    },

    // The pin's shape given to a connector.
    pinOnto(c) {
      const pin = { ...this.pinShape() };
      c.pin = pin;
      c.radius = (pin.diameter || 6) / 2;
      c.height = (pin.length_upper || 7) + (pin.length_lower || 7);
      c.radius_tolerance = 0;
      c.height_tolerance = 0;
      c.style = 'prism';
      c.shape = 'circle';
      c.pinInvalid = this.pinInfo ? this.pinInfo.invalid : null;
    },

    // A field changed: the preset is then the pin's own ("Custom").
    setPin(changes) {
      this.pin = { ...this.pinShape(), ...changes, preset: 3 };
      if (this.pinUi.same_length) this.pin.length_lower = this.pin.length_upper;
      if (this.pinUi.same_clearance) this.pin.clearance_lower = this.pin.clearance_upper;
      this.pinUi.misfit = false;
      this.askPin({});
    },

    pinRows(busy, row) {
      const { el } = this.host;
      const p = this.pinShape();
      const info = this.pinInfo;
      const mm = t('mm');
      const rows = [];
      if (!info) { this.askPin({}); return rows; }
      // "Preset": S, M, L, Auto, Custom.
      const preset = el('select', { disabled: busy });
      preset.setAttribute('aria-label', t('Preset'));
      [t('S'), t('M'), t('L'), t('Auto'), t('Custom')].forEach((word, at) => preset.append(new Option(word, String(at), false, (p.preset === 3 ? 4 : p.preset) === at && !(at === 3))));
      if (this.pinUi.auto) preset.value = '3';
      preset.addEventListener('change', () => {
        const at = Number(preset.value);
        this.pinUi.same_length = true;
        this.pinUi.same_clearance = true;
        this.pinUi.auto = at === 3;
        if (at <= 2) this.askPin({ preset: at });
        else if (at === 3) this.askPin({ preset: 'auto' });
        else this.setPin({});
      });
      rows.push(row(t('Preset'), preset));
      if (this.pinUi.misfit) rows.push(el('p', { className: 'edit-hint cut-note', textContent: t('No preset fits this cut; S is used. Check the fit.') }));
      const number = (label, key, min, max, tip, { step = 0.05, scale = 1, unit = mm, also = null } = {}) => {
        const input = el('input', { type: 'number', min: String(min * scale), max: String(max * scale), step: String(step), value: (Math.round(p[key] * scale * 100) / 100).toFixed(scale === 1 ? 2 : 0), title: tip, disabled: busy });
        input.dataset.field = `pin-${key}`;
        input.setAttribute('aria-label', label);
        input.addEventListener('change', () => {
          const value = Number(input.value) / scale;
          if (!Number.isFinite(value)) { this.panel(); return; }
          const clamped = Math.min(max, Math.max(min, value));
          this.pinUi.auto = false;
          this.setPin({ [key]: clamped, ...(also ? { [also]: clamped } : {}) });
        });
        return row(label, el('span', { className: 'unit' }, input, unit ? ` ${unit}` : ''));
      };
      const tick = (label, value, act, tip) => {
        const input = el('input', { type: 'checkbox', checked: Boolean(value), disabled: busy });
        input.addEventListener('change', () => act(input.checked));
        return el('label', { className: 'edit-check', title: tip || '' }, input, ` ${label}`);
      };
      const choose = (label, key, words, tip) => {
        const input = el('select', { disabled: busy, title: tip || '' });
        input.setAttribute('aria-label', label);
        words.forEach((word, at) => input.append(new Option(word, String(at), false, Number(p[key]) === at)));
        input.addEventListener('change', () => { this.pinUi.auto = false; this.setPin({ [key]: Number(input.value) }); });
        return row(label, input);
      };
      rows.push(number(t('Diameter'), 'diameter', 3, 50, t('Nominal pin diameter.')));
      rows.push(tick(t('Same length both sides'), this.pinUi.same_length, (on) => { this.pinUi.same_length = on; this.setPin({}); }));
      if (this.pinUi.same_length) rows.push(number(t('Length'), 'length_upper', 2, 100, t("How far the pin reaches into each half, from that half's cut face."), { also: 'length_lower' }));
      else {
        rows.push(number(t('Upper length'), 'length_upper', 2, 100, t('How far the pin reaches into the upper half.')));
        rows.push(number(t('Lower length'), 'length_lower', 2, 100, t('How far the pin reaches into the lower half.')));
      }
      rows.push(tick(t('Same clearance both sides'), this.pinUi.same_clearance, (on) => { this.pinUi.same_clearance = on; this.setPin({}); }));
      if (this.pinUi.same_clearance) rows.push(number(t('Clearance'), 'clearance_upper', 0, 1, t('Radial gap between the pin and its hole. Printed holes come out small, so a little clearance is needed even for a tight fit.'), { also: 'clearance_lower' }));
      else {
        rows.push(number(t('Upper clearance'), 'clearance_upper', 0, 1, t('Radial clearance of the upper hole.')));
        rows.push(number(t('Lower clearance'), 'clearance_lower', 0, 1, t('Radial clearance of the lower hole.')));
      }
      rows.push(tick(t('Groove for the lip'), p.hole_groove, (on) => this.setPin({ hole_groove: on })));
      rows.push(tick(t('Lip'), p.lip, (on) => this.setPin({ lip: on })));
      if (p.lip) {
        rows.push(number(t('Lip height'), 'lip_height', 0.05, 0.24 * p.diameter, t('How far the lip stands proud of the pin.')));
        rows.push(number(t('Lip width'), 'lip_width', 0.2, 10, t('Axial length of the lip, flanks included.')));
        rows.push(number(t('Lip position'), 'lip_position', 0.1, 0.9, t('Where the lip sits along each side, from the cut face.'), { step: 1, scale: 100, unit: '%' }));
      }
      rows.push(choose(t('Cutout'), 'cutout', [t('None'), t('Flat'), t('Slot'), t('Flat + slot')]));
      if (p.cutout === 1 || p.cutout === 3) {
        rows.push(number(t('Flat depth'), 'flat_depth', 0.1, 0.33 * p.diameter, t('How much the printing flat takes off. 0.15 x the diameter keeps every overhang at 45 degrees or less, so the pin lies on it with no supports.')));
        rows.push(tick(t('Flats on both sides'), p.flat_both_sides, (on) => this.setPin({ flat_both_sides: on })));
      }
      if (p.cutout === 2 || p.cutout === 3) {
        rows.push(number(t('Slot width'), 'slot_width', 0.6, 0.5 * p.diameter, t('Width of the slit at each end. It lets the lip side flex so the pin goes in by hand.')));
        rows.push(number(t('Slot length'), 'slot_length', 1, 100, t("How far each slit runs in from the pin's end.")));
      }
      rows.push(choose(t('Print'), 'pose', [t('Auto (lying when it can)'), t('Upright')], t('Auto lays the pin on a flat when it has one flat enough to print on without supports, which puts the layer lines along the pin (strongest). Upright prints round holes and pins most accurately, but layer lines then cross the pin at the joint, where it is weakest.')));
      // How it prints (`pushfit_resolve_pose`); the reason is the
      // reference's own sentence, which its catalog does not have.
      const pose = info.lies ? t('Prints lying on its flat.') : p.pose === 1 ? t('Prints upright.') : `${t('Prints upright:')} ${info.why || ''}`;
      rows.push(el('p', { className: 'edit-hint', textContent: pose }));
      rows.push(tick(t('Advanced'), this.pinUi.advanced, (on) => { this.pinUi.advanced = on; this.panel(); }));
      if (this.pinUi.advanced) {
        rows.push(number(t('End chamfer'), 'end_chamfer', 0, 0.25 * p.diameter, t('45 degree lead-in at each end of the pin.')));
        rows.push(number(t('Mouth chamfer'), 'mouth_chamfer', 0, 0.25 * p.diameter, t("45 degree flare at each hole's mouth. It also takes up elephant's foot.")));
        rows.push(number(t('Depth clearance'), 'depth_clearance', 0, 5, t('Extra depth at the bottom of each hole, for air and squish.')));
        rows.push(tick(t('Pointed hole end'), p.pointed_end, (on) => this.setPin({ pointed_end: on })));
      }
      // "Rotation", as every connector has.
      const s = this.connector;
      const angle = el('input', { type: 'number', min: '0', max: '180', step: '1', value: (Math.round((s.angle * 180) / Math.PI * 100) / 100).toFixed(2), disabled: busy });
      angle.setAttribute('aria-label', t('Rotation'));
      angle.addEventListener('change', () => { const value = Number(angle.value); if (Number.isFinite(value)) this.setConnector('angle', (Math.min(180, Math.max(0, value)) * Math.PI) / 180); });
      rows.push(row(t('Rotation'), el('span', { className: 'unit' }, angle, ' °')));
      rows.push(this.pinDrawing(p, info));
      // Why it cannot be made, and what may go wrong: the reference's own
      // sentences, which it does not translate.
      if (info.invalid) rows.push(el('p', { className: 'edit-hint cut-note', textContent: info.invalid }));
      for (const warning of info.warnings || []) rows.push(el('p', { className: 'edit-hint cut-note', textContent: warning }));
      rows.push(el('p', { className: 'edit-hint', textContent: t('Each pin becomes its own object beside the part. PETG and ABS/ASA flex better than PLA; for PLA prefer a groove over a slot.') }));
      return rows;
    },

    // The pin's outline across, its flats and its slits, the cut between
    // the halves (`render_pushfit_profile_inset`).
    pinDrawing(p, info) {
      const ns = 'http://www.w3.org/2000/svg';
      const make = (name, attrs) => {
        const node = document.createElementNS(ns, name);
        for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, String(value));
        return node;
      };
      const [w, h] = [240, 84];
      const r = p.diameter / 2;
      const lip = p.lip ? p.lip_height : 0;
      const [zmin, zmax] = [-p.length_lower, p.length_upper];
      const scale = Math.min((0.84 * w) / Math.max(1e-3, zmax - zmin), (0.8 * h) / Math.max(1e-3, 2 * (r + lip)));
      const cx = w / 2 - ((zmin + zmax) / 2) * scale;
      const at = (z, radius) => [cx + z * scale, h / 2 - radius * scale];
      const svg = make('svg', { viewBox: `0 0 ${w} ${h}`, class: 'pin-drawing', role: 'img', 'aria-label': t('Pin') });
      const profile = info.profile || [];
      for (const sign of [1, -1]) {
        const points = profile.map(([radius, z]) => at(z, sign * radius).join(',')).join(' ');
        svg.append(make('polyline', { points, class: 'pin-body' }));
      }
      if (p.cutout === 1 || p.cutout === 3) {
        const a = r - p.flat_depth;
        for (const sign of p.flat_both_sides ? [1, -1] : [1]) {
          const [x0, y] = at(zmin, -sign * a);
          const [x1] = at(zmax, 0);
          svg.append(make('line', { x1: x0, y1: y, x2: x1, y2: y, class: 'pin-flat' }));
        }
      }
      if (p.cutout === 2 || p.cutout === 3) {
        const half = p.slot_width / 2;
        for (const [from, to] of [[zmax - p.slot_length + half, zmax], [zmin, zmin + p.slot_length - half]]) {
          const [x0, y0] = at(from, half);
          const [x1, y1] = at(to, -half);
          svg.append(make('rect', { x: Math.min(x0, x1), y: y0, width: Math.abs(x1 - x0), height: y1 - y0, class: 'pin-slot' }));
        }
      }
      const [px] = at(0, 0);
      svg.append(make('line', { x1: px, y1: 4, x2: px, y2: h - 4, class: 'pin-cut' }));
      const upper = make('text', { x: px + 4, y: 11, class: 'pin-word' });
      upper.textContent = t('upper');
      const lower = make('text', { x: px - 4, y: 11, class: 'pin-word', 'text-anchor': 'end' });
      lower.textContent = t('lower');
      svg.append(upper, lower);
      return svg;
    },
  });
})(window.EdgeGL);
