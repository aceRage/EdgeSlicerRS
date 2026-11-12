'use strict';

// "Cut" [C]: an object cut by a plane (Gizmos/GLGizmoCut.cpp, its planar
// mode with a flat surface). The plane is placed here, in the page: drawn
// over the object, which takes two tints, one each side of it; moved by
// its grip or by dragging the plane, turned by the grip's cones, flipped
// by a click on it, laid along a line drawn with Shift; the panel has the
// cut's position, its mirror, its thickness and what is to be done with
// each side. "Perform cut" is one `scene_edit` (`cut`); the section the
// plane makes is asked of the session (`cut_section`) as it moves.
//
// What the reference has besides, and this has not (UI_PARITY.md,
// section 13, "Cut"): the "Dovetail" mode, the curved and drawn surfaces,
// connectors, "Pick face", "Keep cut editable", a part given to the other
// side by the right button. None of them is shown.
(function (GL) {
  const M = GL.math;

  // GLGizmoCut.cpp: the two sides (UPPER_PART_COLOR, LOWER_PART_COLOR),
  // the plane (CUT_PLANE_DEF_COLOR, CUT_PLANE_ERR_COLOR), the grip
  // (GRABBER_COLOR) and the axes' cones (ColorRGBA::X, Y, as Orca has them).
  const UPPER = [0, 1, 1];
  const LOWER = [1, 0, 1];
  const PLANE = [0.9, 0.9, 0.9, 0.5];
  const PLANE_ERROR = [1, 0.8, 0.8, 0.5];
  const GRABBER = [1, 1, 0, 1];
  const GRAY = [0.5, 0.5, 0.5, 1];
  const AXES = [[1, 60 / 255, 91 / 255, 1], [100 / 255, 200 / 255, 24 / 255, 1]];
  const SECTION = [1, 1, 1, 1];
  // GLGizmoCut.cpp `get_grabber_mean_size` (ENABLE_FIXED_GRABBER: 32 px),
  // `get_half_size` (0.35 of it), larger by a quarter while dragged.
  const GRIP_PX = 32 * 0.35;
  // `m_cut_plane_radius_koef`: the plane's half side over the object's
  // bounding sphere's radius.
  const PLANE_SIZE = 1.5;
  // `cut_plane_center_allowed`: how far past the object the plane may go.
  const PAST = 0.5;
  // `m_snap_step`: a move with Shift held goes by whole millimetres.
  const SNAP = 1;
  // `SnapRegionsCount`, `ScaleStepsCount`.
  const [COARSE, FINE] = [8, 72];
  // CurvedCut.hpp `CutThicknessMax`.
  const THICKNESS_MAX = 20;

  const sub = M.sub;
  const unit = (v) => M.normalize(v);
  // A turn about a unit axis, applied to a vector.
  const turn = (v, axis, angle) => M.rows.apply(M.rows.about(axis, angle), v);
  const round = (value, digits = 2) => {
    const power = 10 ** digits;
    const rounded = Math.round(value * power) / power;
    return Object.is(rounded, -0) ? 0 : rounded;
  };
  const fixed = (value) => round(value).toFixed(2);

  const cutter = {
    host: null,
    stage: null,
    editor: null,
    // The cut being placed: the copy, the plane (its middle and its axes
    // x, y and the normal, in the scene), what the session said of it.
    open: null,
    // "After cut" and the thickness: kept from one cut to the next, as
    // the reference's gizmo keeps them, but the thickness, which each
    // opening starts at 0 (`on_set_state`).
    options: {
      keep_upper: true,
      keep_lower: true,
      place_on_cut_upper: false,
      place_on_cut_lower: false,
      flip_upper: false,
      flip_lower: false,
      to_parts: false,
    },
    panelNode: null,
    shapes: null,

    init(host, editor) {
      this.host = host;
      this.stage = host.stage;
      this.editor = editor;
    },

    active() {
      return Boolean(this.open);
    },

    // ---- opening and closing ----

    start(placing) {
      const box = placing.box;
      if (!box) return;
      const centre = M.centre(box);
      this.open = {
        copy: placing.id,
        box,
        centre,
        radius: M.length(sub(box[1], box[0])) / 2,
        middle: centre.slice(),
        axes: [[1, 0, 0], [0, 1, 0], [0, 0, 1]],
        thickness: 0,
        from: 'centred',
        section: null,
        asked: null,
        history: [],
        future: [],
        line: null,
      };
      this.changed(false);
    },

    close() {
      this.open = null;
      this.stage.scene.cutting = null;
      this.stage.canvas.title = '';
      if (this.panelNode) this.panelNode.hidden = true;
      this.dropSection();
    },

    // The copy selected is another (or the same one, moved): the plane
    // starts again round it.
    follow(placing) {
      if (!this.open) return;
      if (!placing) { this.close(); return; }
      const box = placing.box;
      const same = placing.id === this.open.copy && box && this.open.box
        && box.every((corner, at) => corner.every((value, axis) => Math.abs(value - this.open.box[at][axis]) < 1e-6));
      if (!same) this.start(placing);
    },

    // ---- the plane ----

    // The plane's frame in the scene, column by column: its x, its y, its
    // normal, its middle. What `scene_edit` and `cut_section` take.
    frame() {
      const [u, v, n] = this.open.axes;
      const c = this.open.middle;
      return [...u, 0, ...v, 0, ...n, 0, ...c, 1];
    },

    normal() {
      return this.open.axes[2];
    },

    // Whether the plane is where it starts (`is_cut_plane_init`).
    atStart() {
      const o = this.open;
      const square = o.axes.every((axis, at) => axis.every((value, i) => Math.abs(value - (i === at ? 1 : 0)) < 1e-9));
      return square && M.length(sub(o.middle, o.centre)) < 1e-9;
    },

    // The heights of the lower part's and the upper part's faces over the
    // plane (CurvedCut.cpp `curved_cut_thickness_faces`).
    faces() {
      const t = this.open.thickness;
      if (this.open.from === 'above') return [0, t];
      if (this.open.from === 'below') return [-t, 0];
      return [-t / 2, t / 2];
    },

    // The plane as it was, for the tool's own undo while it is open.
    remember() {
      const o = this.open;
      o.history.push({ middle: o.middle.slice(), axes: o.axes.map((axis) => axis.slice()) });
      if (o.history.length > 200) o.history.shift();
      o.future = [];
    },

    // Takes back (or makes again) the last move of the plane. False when
    // there is none, and the project's own history is then the one used.
    step(which) {
      const o = this.open;
      if (!o) return false;
      const [from, onto] = which === 'undo' ? [o.history, o.future] : [o.future, o.history];
      const state = from.pop();
      if (!state) return false;
      onto.push({ middle: o.middle.slice(), axes: o.axes.map((axis) => axis.slice()) });
      o.middle = state.middle;
      o.axes = state.axes;
      this.changed();
      return true;
    },

    // A place for the plane's middle, as the reference allows it
    // (GLGizmoCut.cpp `set_center_pos`): while the plane still meets the
    // object, or half a millimetre past it, or when it comes nearer the
    // middle of the object than it was.
    allowed(middle) {
      const o = this.open;
      const reach = o.section && o.section.reach;
      if (!reach) return true;
      const shift = M.dot(sub(middle, o.middle), this.normal());
      const [low, high] = [reach[0][2] - shift, reach[1][2] - shift];
      if (high > -PAST && low < PAST) return true;
      return M.length(sub(middle, o.centre)) < M.length(sub(o.middle, o.centre));
    },

    // Moves the plane's middle: false when it may not go there.
    moveTo(middle) {
      if (!this.allowed(middle)) return false;
      this.open.middle = middle;
      return true;
    },

    // "Move cut plane" by the arrow keys (GLGizmosManager.cpp, Up and
    // Down: `shift_cut` by 1 mm along the normal).
    shift(by) {
      this.remember();
      if (this.moveTo(M.along(this.open.middle, this.normal(), by))) this.changed();
      else this.open.history.pop();
    },

    // "Flip cut plane": half a turn about the plane's own x.
    flip() {
      this.remember();
      const [u, v, n] = this.open.axes;
      this.open.axes = [u, M.scale(v, -1), M.scale(n, -1)];
      this.changed();
    },

    // "Reset cutting plane".
    reset() {
      this.remember();
      this.open.middle = this.open.centre.slice();
      this.open.axes = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
      this.changed();
    },

    // "Mirror cut" on a world axis, about the middle of the object, the
    // side that was upper staying upper (GLGizmoCut.cpp `mirror_cut`,
    // CutRecipe.cpp `cut_recipe_mirrored`).
    mirror(axis) {
      this.remember();
      const o = this.open;
      const reflect = (v) => v.map((value, at) => (at === axis ? -value : value));
      const middle = o.middle.map((value, at) => (at === axis ? 2 * o.centre[at] - value : value));
      const n = reflect(o.axes[2]);
      const u = reflect(o.axes[0]);
      o.axes = [u, M.cross(n, u), n];
      o.middle = middle;
      this.changed();
    },

    // "Cut position": the plane's middle at this height in the scene.
    setHeight(z) {
      if (!Number.isFinite(z)) { this.panel(); return; }
      this.remember();
      const middle = this.open.middle.slice();
      middle[2] = z;
      if (this.moveTo(middle)) this.changed();
      else { this.open.history.pop(); this.panel(); }
    },

    // The plane changed: the scene's tints follow, the section is asked
    // for again, the panel says it. `soon`: while a drag lasts.
    changed(soon = false) {
      const o = this.open;
      if (!o) return;
      const n = this.normal();
      this.stage.scene.cutting = { copy: o.copy, split: { plane: [...n, M.dot(n, o.middle)], upper: UPPER, lower: LOWER } };
      this.ask(soon);
      this.panel();
      this.stage.frame();
    },

    // ---- what the session says of the plane ----

    // The section and the object's reach, asked for with the latest plane;
    // one call at a time, the last plane wanted asked next.
    async ask() {
      const o = this.open;
      const project = this.host.project();
      if (!o || !project) return;
      const plane = this.frame();
      const key = plane.join(',');
      if (o.asked === key) return;
      o.wanted = key;
      if (o.busy) return;
      o.busy = true;
      try {
        while (o.wanted && o.wanted !== o.asked && this.open === o) {
          const asking = o.wanted;
          const answer = await this.host.call('cut_section', { project: project.id, copy: o.copy, plane: asking.split(',').map(Number) });
          if (this.open !== o) return;
          o.asked = asking;
          o.section = answer;
          this.setSection(answer.lines);
          this.panel();
          this.stage.frame();
        }
      } catch (error) {
        if (this.open === o) this.host.say(error.message, true);
      } finally {
        o.busy = false;
      }
    },

    setSection(lines) {
      this.dropSection();
      if (lines && lines.length) this.sectionLines = this.stage.renderer.lines(lines);
    },

    dropSection() {
      if (this.sectionLines) this.stage.renderer.drop(this.sectionLines);
      this.sectionLines = null;
    },

    // "Cut plane is placed out of object": the plane meets none of it.
    meets() {
      const o = this.open;
      return !o.section || o.section.lines.length > 0;
    },

    // `can_perform_cut`: something is kept.
    canPerform() {
      const p = this.options;
      return this.open && (p.keep_upper || p.keep_lower || p.to_parts) && !this.editor.busy;
    },

    async perform() {
      if (!this.canPerform()) return;
      const o = this.open;
      const p = this.options;
      const args = {
        op: 'cut',
        copy: o.copy,
        plane: this.frame(),
        ...p,
        thickness: o.thickness,
        thickness_from: o.from,
      };
      // The tool closes before the cut is made, as the reference resets
      // its gizmos first (`perform_cut`); what the cut made is selected.
      this.editor.setTool(null);
      await this.editor.commit(args, { select: true });
    },

    // ---- the handles ----

    mm() {
      return 1 / this.stage.camera.zoom;
    },

    // The grip's place (`m_grabber_connection_len`: half the radius out
    // along the normal) and size in mm.
    grip() {
      const o = this.open;
      const size = GRIP_PX * this.mm() * (this.held && this.held.kind === 'cut' ? 1.25 : 1);
      return { at: M.along(o.middle, this.normal(), 0.5 * o.radius), size, reach: 0.5 * o.radius };
    },

    // The cones of the grip: a pair turning about the plane's x (pointing
    // along its y) and a pair turning about its y (pointing along its x).
    cones() {
      const { at, size } = this.grip();
      const [u, v] = this.open.axes;
      return [
        { axis: 0, at: M.along(at, v, 1.25 * size), points: v },
        { axis: 0, at: M.along(at, v, -1.25 * size), points: M.scale(v, -1) },
        { axis: 1, at: M.along(at, u, 1.25 * size), points: u },
        { axis: 1, at: M.along(at, u, -1.25 * size), points: M.scale(u, -1) },
      ];
    },

    // What is under a pixel: the grip, a cone, the plane, or null.
    handleAt(x, y) {
      if (!this.open || this.editor.busy) return null;
      const frame = this.stage.matrices();
      const view = (point) => this.editor.toView(frame, point);
      const { at, size } = this.grip();
      const px = size / this.mm();
      // The ball first, then the cones round it, each by what is drawn of it.
      const g = view(at);
      if (Math.hypot(x - g[0], y - g[1]) < px) return { kind: 'cut', part: 'grip', id: 'cut-grip' };
      for (const cone of this.cones()) {
        const p = view(cone.at);
        if (Math.hypot(x - p[0], y - p[1]) < px * 0.9) return { kind: 'cut', part: cone.axis === 0 ? 'turn-x' : 'turn-y', axis: cone.axis, id: `cut-turn-${cone.axis}` };
      }
      const hit = this.onPlane(x, y);
      if (hit) return { kind: 'cut', part: 'plane', id: 'cut-plane', point: hit };
      return null;
    },

    // Where the line of sight through a pixel meets the plane, when it is
    // on the plane as drawn.
    onPlane(x, y) {
      const o = this.open;
      const n = this.normal();
      const ray = this.stage.camera.ray(x, y);
      const toward = M.dot(ray.direction, n);
      if (Math.abs(toward) < 1e-6) return null;
      const point = M.along(ray.origin, ray.direction, M.dot(sub(o.middle, ray.origin), n) / toward);
      const out = sub(point, o.middle);
      const half = PLANE_SIZE * o.radius;
      return Math.abs(M.dot(out, o.axes[0])) <= half && Math.abs(M.dot(out, o.axes[1])) <= half ? point : null;
    },

    // The tooltip of what is under the pointer: the parts' heights on the
    // grip (`get_tooltip`), what a click and a drag do on the plane.
    tip(handle) {
      if (!handle) return '';
      if (handle.part === 'grip') {
        const reach = this.open.section;
        if (!reach) return '';
        const name = this.options.to_parts ? t('Part') : t('Object');
        const unitWord = t('mm');
        const lines = [];
        if (reach.above > 0) lines.push(`${fixed(reach.above)} ${unitWord} (${name} A)`);
        if (reach.below > 0) lines.push(`${fixed(reach.below)} ${unitWord} (${name} B)`);
        return lines.join('\n');
      }
      if (handle.part === 'plane') return t('Click to flip the cut plane\nDrag to move the cut plane');
      return '';
    },

    hover(x, y) {
      const handle = this.handleAt(x, y);
      this.stage.canvas.title = this.tip(handle);
      return handle;
    },

    // ---- dragging ----

    // What a press takes: a handle, or with Shift a line to lay the plane
    // along (`process_cut_line`).
    grab(x, y, event) {
      if (!this.open || this.editor.busy) return null;
      if (event.shiftKey) return { kind: 'cut', part: 'line', x, y, tap: () => {} };
      const handle = this.handleAt(x, y);
      if (!handle) return null;
      const grab = { kind: 'cut', part: handle.part, handle, x, y };
      // A click on the plane, with no drag, flips it.
      if (handle.part === 'plane') grab.tap = () => this.flip();
      else grab.tap = () => {};
      return grab;
    },

    begin(grab) {
      const o = this.open;
      grab.middle = o.middle.slice();
      grab.axes = o.axes.map((axis) => axis.slice());
      grab.begun = true;
      this.held = grab;
      if (grab.part === 'line') {
        grab.from = [grab.x, grab.y];
        o.line = { from: grab.from, to: grab.from };
        return true;
      }
      this.remember();
      if (grab.part === 'grip' || grab.part === 'plane') {
        const origin = grab.part === 'grip' ? this.grip().at : grab.handle.point;
        grab.origin = origin;
        grab.start = this.editor.along(origin, this.normal(), grab.x, grab.y);
      } else {
        const axis = grab.axes[grab.handle.axis];
        grab.pivotAxis = axis;
        grab.start = this.around(o.middle, axis, grab.x, grab.y);
        if (!grab.start) return false;
      }
      return true;
    },

    // Where the line of sight through a pixel meets the plane through
    // `centre` square to `axis`: the point, relative to the centre.
    around(centre, axis, x, y) {
      const ray = this.stage.camera.ray(x, y);
      const toward = M.dot(ray.direction, axis);
      if (Math.abs(toward) < 1e-6) return null;
      const point = M.along(ray.origin, ray.direction, M.dot(sub(centre, ray.origin), axis) / toward);
      return sub(point, centre);
    },

    drag(grab, x, y, event) {
      if (!grab.begun && !this.begin(grab)) { grab.cancelled = true; return; }
      if (grab.cancelled) return;
      const o = this.open;
      if (grab.part === 'line') {
        o.line = { from: grab.from, to: [x, y] };
        this.stage.frame();
        return;
      }
      if (grab.part === 'grip' || grab.part === 'plane') {
        // GLGizmoCut.cpp `dragging_grabber_move`: along the normal, by
        // whole millimetres with Shift.
        let far = this.editor.along(grab.origin, grab.axes[2], x, y) - grab.start;
        if (event.shiftKey) far = SNAP * Math.round(far / SNAP);
        if (this.moveTo(M.along(grab.middle, grab.axes[2], far))) this.changed(true);
        return;
      }
      // `dragging_grabber_rotation`: about the plane's own x or y, by an
      // eighth of a turn between a third and two thirds of the grip's
      // reach, by 5 degrees about the reach itself.
      const now = this.around(o.middle, grab.pivotAxis, x, y);
      if (!now) return;
      const a = unit(grab.start);
      const b = unit(now);
      let angle = Math.atan2(M.dot(M.cross(a, b), grab.pivotAxis), M.dot(a, b));
      const far = M.length(now);
      const reach = this.grip().reach;
      const radius = reach * 0.85;
      if (far >= radius / 3 && far <= (2 * radius) / 3) angle = ((2 * Math.PI) / COARSE) * Math.round(angle / ((2 * Math.PI) / COARSE));
      else if (far >= reach * 0.85 && far <= reach * 1.15) angle = ((2 * Math.PI) / FINE) * Math.round(angle / ((2 * Math.PI) / FINE));
      const axes = grab.axes.map((axis) => turn(axis, grab.pivotAxis, angle));
      o.axes = M.rows.tidy(axes);
      grab.angle = angle;
      this.stage.canvas.title = `${'XY'[grab.handle.axis]}: ${round((angle * 180) / Math.PI, 1).toFixed(1)}°`;
      this.changed(true);
    },

    drop(grab, done) {
      this.held = null;
      const o = this.open;
      if (!o || !grab.begun) return;
      if (grab.part === 'line') {
        const line = o.line;
        o.line = null;
        if (done && line) this.layAlong(line.from, line.to);
        this.stage.frame();
        return;
      }
      if (!done) {
        o.middle = grab.middle;
        o.axes = grab.axes;
        o.history.pop();
      }
      this.changed();
    },

    // "Cut by line": the plane laid through the line drawn and the line of
    // sight, its middle the object's middle brought onto it
    // (`process_cut_line`).
    layAlong(from, to) {
      if (Math.hypot(to[0] - from[0], to[1] - from[1]) < 3) return;
      const a = this.stage.camera.ray(from[0], from[1]);
      const b = this.stage.camera.ray(to[0], to[1]);
      const p = M.along(a.origin, a.direction, 1);
      const q = M.along(b.origin, b.direction, 1);
      const n = unit(M.cross(sub(q, p), b.direction));
      if (M.length(n) < 0.5) return;
      const o = this.open;
      // The turn taking z to the normal, the least one.
      const z = [0, 0, 1];
      const axis = M.cross(z, n);
      const angle = Math.atan2(M.length(axis), M.dot(z, n));
      const pivot = M.length(axis) > 1e-9 ? unit(axis) : [1, 0, 0];
      const axes = [turn([1, 0, 0], pivot, angle), turn([0, 1, 0], pivot, angle), n];
      const middle = M.along(o.centre, n, M.dot(n, sub(q, o.centre)));
      this.remember();
      o.axes = M.rows.tidy(axes);
      o.middle = middle;
      this.changed();
    },

    // ---- keys ----

    // The tool's own keys, asked before the others: Up and Down move the
    // plane by 1 mm. True when the key was the tool's.
    key(event) {
      if (!this.open || event.ctrlKey || event.metaKey || event.altKey) return false;
      if (event.target.matches('input, select, textarea')) return false;
      if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
        this.shift(event.key === 'ArrowUp' ? 1 : -1);
        return true;
      }
      return false;
    },

    // ---- drawing ----

    parts() {
      if (this.shapes) return this.shapes;
      const r = this.stage.renderer;
      // A square of side 2 about the origin, in z = 0.
      const square = r.triangles([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, -1, 0, 1, 1, 0, -1, 1, 0]);
      const points = [];
      const at = (i, j) => {
        const [theta, phi] = [(Math.PI * i) / 12, (2 * Math.PI * j) / 24];
        const p = [Math.sin(theta) * Math.cos(phi), Math.sin(theta) * Math.sin(phi), Math.cos(theta)];
        return [p, p];
      };
      for (let i = 0; i < 12; i += 1) {
        for (let j = 0; j < 24; j += 1) points.push(at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j), at(i + 1, j + 1), at(i, j + 1));
      }
      this.shapes = { square, sphere: r.meshFromPoints(points), line: r.lines([0, 0, 0, 0, 0, 1]) };
      return this.shapes;
    },

    // The matrix taking z to `to` (a unit vector), at `at`.
    pointing(at, to, scale) {
      const z = [0, 0, 1];
      const axis = M.cross(z, to);
      const angle = Math.atan2(M.length(axis), M.dot(z, to));
      const rows = M.length(axis) > 1e-9 ? M.rows.about(unit(axis), angle) : (to[2] < 0 ? M.rows.about([1, 0, 0], Math.PI) : M.rows.identity());
      const r = rows;
      const linear = [r[0][0], r[1][0], r[2][0], 0, r[0][1], r[1][1], r[2][1], 0, r[0][2], r[1][2], r[2][2], 0, 0, 0, 0, 1];
      return M.chain(M.translation(...at), linear, M.scaling(...scale));
    },

    draw(frame) {
      const o = this.open;
      if (!o) return;
      const r = this.stage.renderer;
      const gl = r.gl;
      const shapes = this.parts();
      const view = M.multiply(frame.projection, frame.view);
      const plane = this.frame();
      const half = PLANE_SIZE * o.radius;
      // The plane, twice with a thickness: the band between is what goes.
      const colour = this.canPerform() || this.editor.busy ? PLANE : PLANE_ERROR;
      r.blend(true);
      const [low, high] = this.faces();
      for (const at of low === high ? [0] : [low, high]) {
        r.drawFlat(shapes.square, M.chain(view, plane, M.translation(0, 0, at), M.scaling(half, half, 1)), colour);
      }
      // The section, over everything.
      gl.disable(gl.DEPTH_TEST);
      if (this.sectionLines) r.drawLines(this.sectionLines, view, SECTION, 1.5);
      // The grip and its cones (`render_cut_plane_grabbers`).
      const { at, size } = this.grip();
      const hot = this.editor.hot;
      const held = this.held && this.held.handle ? this.held.handle.id : null;
      const lit = (id) => hot === id || held === id;
      r.drawLines(shapes.line, M.chain(view, M.translation(...o.middle), this.pointing([0, 0, 0], this.normal(), [1, 1, 1]), M.scaling(1, 1, 0.5 * o.radius)), GRABBER, 2);
      // Over the object, as the reference clears the depth for them; a
      // depth that is not written is not cleared either.
      gl.enable(gl.DEPTH_TEST);
      r.blend(false);
      gl.clear(gl.DEPTH_BUFFER_BIT);
      const gripColour = lit('cut-turn-0') ? AXES[0] : lit('cut-turn-1') ? AXES[1] : lit('cut-grip') ? GRABBER : GRAY;
      r.drawMesh(shapes.sphere, frame, M.chain(M.translation(...at), M.scaling(size, size, size)), { tint: gripColour });
      const turning = held === 'cut-turn-0' || held === 'cut-turn-1';
      for (const cone of this.cones()) {
        const id = `cut-turn-${cone.axis}`;
        if ((turning || this.held) && held !== id) continue;
        r.drawFlat(this.editor.parts().cone, M.chain(view, this.pointing(cone.at, cone.points, [0.75 * size / 0.32, 0.75 * size / 0.32, 1.8 * size])), AXES[cone.axis]);
      }
      // The line being drawn with Shift.
      if (o.line) {
        const a = this.stage.camera.ray(o.line.from[0], o.line.from[1]);
        const b = this.stage.camera.ray(o.line.to[0], o.line.to[1]);
        const ends = r.lines([...M.along(a.origin, a.direction, 1), ...M.along(b.origin, b.direction, 1)]);
        gl.disable(gl.DEPTH_TEST);
        r.drawLines(ends, view, GRABBER, 2);
        gl.enable(gl.DEPTH_TEST);
        r.drop(ends);
      }
      r.blend(false);
    },

    // ---- the panel ----

    panel() {
      const { el } = this.host;
      if (!this.panelNode) {
        this.panelNode = el('div', { className: 'edit-panel overlay cut-panel', id: 'cut-panel', hidden: true });
        this.panelNode.setAttribute('role', 'group');
        this.panelNode.setAttribute('aria-label', t('Cut'));
        this.host.stageNode.append(this.panelNode);
      }
      const node = this.panelNode;
      const o = this.open;
      if (!o) { node.hidden = true; return; }
      // A field being typed in keeps its words while the plane moves.
      if (node.contains(document.activeElement) && document.activeElement.matches('input[type="number"]') && !node.hidden) {
        this.fill();
        return;
      }
      node.hidden = false;
      const p = this.options;
      const busy = this.editor.busy;
      const rows = [el('h3', { textContent: t('Cut') })];
      // "Build Volume": the object's size in the plane's own axes.
      const reach = o.section && o.section.reach;
      const size = reach ? [0, 1, 2].map((axis) => reach[1][axis] - reach[0][axis]) : null;
      const mm = t('mm');
      const sized = el('span', { className: 'cut-size', textContent: size ? `X: ${fixed(size[0])} ${mm},  Y: ${fixed(size[1])} ${mm},  Z: ${fixed(size[2])} ${mm}` : '…' });
      sized.dataset.field = 'size';
      rows.push(el('div', { className: 'cut-row' }, el('span', { className: 'edit-label', textContent: t('Build Volume') }), sized));
      // "Cut position": the plane's middle's height, and its reset.
      const height = el('input', { type: 'number', step: 'any', value: fixed(o.middle[2]), disabled: busy });
      height.dataset.field = 'height';
      height.setAttribute('aria-label', `${t('Cut position')} Z`);
      height.addEventListener('change', () => this.setHeight(Number(height.value)));
      const resetPlane = el('button', { type: 'button', className: 'icon tool', title: t('Reset cutting plane'), disabled: busy || this.atStart(), textContent: '↺' });
      resetPlane.setAttribute('aria-label', t('Reset cutting plane'));
      resetPlane.addEventListener('click', () => this.reset());
      rows.push(el('div', { className: 'cut-row' },
        el('span', { className: 'edit-label', textContent: t('Cut position') }),
        el('span', { className: 'cut-line' }, el('b', { className: 'gl-z', textContent: 'Z' }), el('span', { className: 'unit' }, height, ` ${mm}`), resetPlane)));
      // "Mirror cut": X, Y, Z.
      const mirrors = el('span', { className: 'cut-line joined' });
      const mirrorTips = [
        t("Mirror the whole cut - plane, surface and connectors - about the object's centre on the world X axis"),
        t("Mirror the whole cut - plane, surface and connectors - about the object's centre on the world Y axis"),
        t("Mirror the whole cut - plane, surface and connectors - about the object's centre on the world Z axis"),
      ];
      [t('X'), t('Y'), t('Z')].forEach((label, axis) => {
        const button = el('button', { type: 'button', textContent: label, title: mirrorTips[axis], disabled: busy });
        button.addEventListener('click', () => this.mirror(axis));
        mirrors.append(button);
      });
      rows.push(el('div', { className: 'cut-row' }, el('span', { className: 'edit-label', textContent: t('Mirror cut') }), mirrors));
      // "Thickness": what the cut takes away, and from which side.
      const tip = t('Width of the band of material the cut removes, measured along the cut normal. 0 is a zero-width cut - the two halves meet exactly, as they do today.');
      const range = el('input', { type: 'range', min: '0', max: String(THICKNESS_MAX), step: '0.05', value: String(o.thickness), title: tip, disabled: busy });
      range.setAttribute('aria-label', t('Thickness'));
      const number = el('input', { type: 'number', min: '0', max: String(THICKNESS_MAX), step: '0.1', value: fixed(o.thickness), title: tip, disabled: busy });
      number.dataset.field = 'thickness';
      number.setAttribute('aria-label', t('Thickness'));
      const setThickness = (value, redraw) => {
        if (!Number.isFinite(value)) return;
        o.thickness = Math.min(THICKNESS_MAX, Math.max(0, value));
        if (redraw) this.panel(); else number.value = fixed(o.thickness);
        this.stage.frame();
      };
      range.addEventListener('input', () => setThickness(Number(range.value), false));
      range.addEventListener('change', () => setThickness(Number(range.value), true));
      number.addEventListener('change', () => setThickness(Number(number.value), true));
      rows.push(el('div', { className: 'cut-row' },
        el('span', { className: 'edit-label', textContent: t('Thickness'), title: tip }),
        el('span', { className: 'cut-slide' }, range, el('span', { className: 'unit' }, number, ` ${mm}`))));
      if (o.thickness > 0) {
        const from = el('span', { className: 'cut-line', role: 'radiogroup' });
        from.setAttribute('aria-label', t('Remove from'));
        for (const [value, label] of [['centred', t('Centred')], ['above', t('Above')], ['below', t('Below')]]) {
          const radio = el('input', { type: 'radio', name: 'cut-from', value, checked: o.from === value, disabled: busy });
          radio.addEventListener('change', () => { o.from = value; this.panel(); this.stage.frame(); });
          from.append(el('label', { className: 'edit-check' }, radio, ` ${label}`));
        }
        rows.push(el('div', { className: 'cut-row' }, el('span', { className: 'edit-label', textContent: t('Remove from') }), from));
        rows.push(el('p', { className: 'edit-hint cut-note', textContent: t('The two halves will be this far apart. Connectors span the gap; a Thread connector needs a thickness below its pitch to stay usable.') }));
      }
      // "Reset cut": the plane, and the connectors there are none of.
      const resetCut = el('button', { type: 'button', textContent: t('Reset cut'), title: t('Reset cutting plane and remove connectors'), disabled: busy || this.atStart() });
      resetCut.addEventListener('click', () => this.reset());
      rows.push(el('div', { className: 'cut-foot cut-start' }, resetCut));
      rows.push(el('hr', { className: 'arrange-rule' }));
      // "After cut".
      rows.push(el('span', { className: 'edit-label', textContent: t('After cut') }));
      const side = (label, colour, keep, place, flip) => {
        const box = (word, key, disabled, act) => {
          const input = el('input', { type: 'checkbox', checked: Boolean(p[key]), disabled: disabled || busy });
          input.addEventListener('change', () => { p[key] = input.checked; if (act) act(input.checked); this.panel(); this.stage.frame(); });
          return el('label', { className: 'edit-check' }, input, ` ${word}`);
        };
        const marker = el('span', { className: 'swatch cut-marker' });
        marker.style.setProperty('--swatch', `rgb(${colour.map((c) => Math.round(c * 255)).join(',')})`);
        return el('div', { className: 'cut-side' },
          el('span', { className: 'cut-part' }, marker, label),
          box(t('Keep'), keep, p.to_parts),
          box(t('Place on cut'), place, !p[keep] || p.to_parts, (on) => { if (on) p[flip] = false; }),
          box(t('Flip'), flip, !p[keep] || p.to_parts, (on) => { if (on) p[place] = false; }));
      };
      rows.push(side(t('Upper part'), UPPER, 'keep_upper', 'place_on_cut_upper', 'flip_upper'));
      rows.push(side(t('Lower part'), LOWER, 'keep_lower', 'place_on_cut_lower', 'flip_lower'));
      const parts = el('input', { type: 'checkbox', checked: p.to_parts, disabled: busy });
      parts.addEventListener('change', () => {
        p.to_parts = parts.checked;
        // Both parts are kept, as they stood (`m_keep_as_parts`).
        if (p.to_parts) Object.assign(p, { keep_upper: true, keep_lower: true, place_on_cut_upper: false, place_on_cut_lower: false, flip_upper: false, flip_lower: false });
        this.panel();
        this.stage.frame();
      });
      rows.push(el('label', { className: 'edit-check' }, parts, ` ${t('Cut to parts')}`));
      // What the keys and the pointer do (`m_shortcuts_cut`).
      const help = el('div', { className: 'paint-help' });
      help.append(el('span', { className: 'paint-key', textContent: `${t('Shift+')}${t('Drag')}` }), el('span', { textContent: t('Draw cut line') }));
      rows.push(help);
      // The warnings (`render_input_window_warning`).
      const warnings = [];
      if (!p.keep_upper && !p.keep_lower && !p.to_parts) warnings.push(t('Select at least one object to keep after cutting.'));
      if (!this.meets()) warnings.push(t('Cut plane is placed out of object'));
      for (const warning of warnings) rows.push(el('p', { className: 'edit-hint cut-warning', role: 'status', textContent: `${t('Warning')}: ${warning}` }));
      const perform = el('button', { type: 'button', className: 'primary', textContent: t('Perform cut'), disabled: !this.canPerform() });
      perform.addEventListener('click', () => this.perform());
      rows.push(el('div', { className: 'cut-foot' }, perform));
      node.replaceChildren(...rows);
    },

    // Only the numbers, while a field has the keys.
    fill() {
      const o = this.open;
      const node = this.panelNode;
      const reach = o.section && o.section.reach;
      const size = node.querySelector('[data-field="size"]');
      if (size && reach) {
        const mm = t('mm');
        const s = [0, 1, 2].map((axis) => reach[1][axis] - reach[0][axis]);
        size.textContent = `X: ${fixed(s[0])} ${mm},  Y: ${fixed(s[1])} ${mm},  Z: ${fixed(s[2])} ${mm}`;
      }
      const height = node.querySelector('[data-field="height"]');
      if (height && document.activeElement !== height) height.value = fixed(o.middle[2]);
    },
  };

  GL.cutter = cutter;
})(window.EdgeGL);
