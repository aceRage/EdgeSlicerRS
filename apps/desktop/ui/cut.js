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
// Besides: "Dovetail", "Pick face", a part given to the other side by the
// right button, "Keep cut editable" with "Edit cut..." and "Copy cut
// to...", and the connectors (connectors.js). What the reference has and
// this has not (UI_PARITY.md, section 13, "Cut"): the curved and drawn
// surfaces and the fork's own connectors; neither is shown.
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
  // GLGizmoBase.cpp FLATTEN_HOVER_COLOR: the face "Pick face" would take,
  // lifted 0.05 mm off it (FacetPicker.cpp `render`).
  const PICKED = [1, 1, 1, 0.75];
  const LIFT = 0.05;
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
  const degrees = (radians) => (radians * 180) / Math.PI;
  const radians = (value) => (value * Math.PI) / 180;
  // GLGizmoCut.hpp `GrooveCountMax`.
  const GROOVE_COUNT_MAX = 20;

  // The dovetail's plane, as triangles in the plane's own coordinates
  // (GLGizmoCut.cpp `its_make_groove_plane`: a slot seen from above, its
  // mouth, neck and flaps, a thin wall of it for each dovetail).
  function groovePlane(g, radius) {
    const flapTaper = Math.abs(g.flaps_angle) < 1e-9 ? g.depth : g.depth / Math.sin(g.flaps_angle);
    const totalFlap = 2 * flapTaper * Math.cos(g.flaps_angle);
    const neckHalf = 0.5 * g.width;
    const mouthHalf = 0.5 * (g.width + totalFlap);
    const planeRadius = 1.5 * radius;
    const halfWidth = 0.5 * planeRadius;
    const halfHeight = 0.5 * 1.5 * planeRadius;
    const taper = halfHeight * Math.tan(g.angle);
    const outerMax = 0.5 * grooveWidth(g, radius);
    const wall = 0.02;
    const out = [];
    const count = Math.max(1, g.count);
    for (let i = 0; i < count; i += 1) {
      const first = i === 0;
      const last = i === count - 1;
      const offset = (i - 0.5 * (count - 1)) * (g.gap + 2 * outerMax);
      const leftX = first ? -halfWidth + offset : -(g.gap / 2) - outerMax + offset;
      const rightX = last ? halfWidth + offset : g.gap / 2 + outerMax + offset;
      let front = 0.5 * g.depth;
      let back = -0.5 * g.depth;
      let mouthOuter = neckHalf + taper;
      let neckOuter = mouthHalf + taper;
      let neckInner = neckHalf - taper;
      let mouthInner = mouthHalf - taper;
      const shift = wall / Math.tan(0.5 * g.flaps_angle);
      let v;
      let idx;
      if (neckHalf > taper && mouthHalf > taper) {
        const make = () => [
          [leftX, -halfHeight, front], [leftX, halfHeight, front], [-neckInner + offset, halfHeight, front], [-mouthOuter + offset, -halfHeight, front],
          [-neckOuter + offset, -halfHeight, back], [-mouthInner + offset, halfHeight, back], [mouthInner + offset, halfHeight, back], [neckOuter + offset, -halfHeight, back],
          [mouthOuter + offset, -halfHeight, front], [neckInner + offset, halfHeight, front], [rightX, halfHeight, front], [rightX, -halfHeight, front],
        ];
        v = make();
        front -= wall; back -= wall;
        neckInner += shift; mouthInner += shift; mouthOuter += shift; neckOuter += shift;
        v = v.concat(make());
        idx = [[5, 4, 7], [5, 7, 6], [3, 4, 5], [3, 5, 2], [9, 6, 8], [8, 6, 7], [1, 0, 2], [2, 0, 3], [9, 8, 10], [10, 8, 11],
          [20, 21, 22], [20, 22, 23], [12, 13, 14], [12, 14, 15], [18, 21, 20], [18, 20, 19], [16, 15, 14], [16, 14, 17], [16, 17, 18], [16, 18, 19],
          [1, 13, 0], [0, 13, 12],
          [0, 12, 3], [3, 12, 15], [3, 15, 4], [4, 15, 16], [4, 16, 7], [7, 16, 19], [7, 19, 20], [7, 20, 8], [8, 20, 11], [11, 20, 23],
          [11, 23, 10], [10, 23, 22],
          [1, 13, 2], [2, 13, 14], [2, 14, 17], [2, 17, 5], [5, 17, 6], [6, 17, 18], [6, 18, 9], [9, 18, 21], [9, 21, 10], [10, 21, 22]];
      } else if (neckHalf < taper && mouthHalf < taper) {
        let neckApex = neckHalf / Math.tan(g.angle);
        let mouthApex = mouthHalf / Math.tan(g.angle);
        const make = () => [
          [leftX, -halfHeight, front], [leftX, halfHeight, front], [rightX, halfHeight, front], [rightX, -halfHeight, front],
          [mouthOuter + offset, -halfHeight, front], [offset, neckApex, front], [-mouthOuter + offset, -halfHeight, front],
          [-neckOuter + offset, -halfHeight, back], [offset, mouthApex, back], [neckOuter + offset, -halfHeight, back],
        ];
        v = make();
        front -= wall; back -= wall; neckApex += wall; mouthApex += wall;
        mouthOuter += shift; neckOuter += shift;
        v = v.concat(make());
        idx = [[8, 7, 9], [5, 8, 6], [6, 8, 7], [4, 9, 8], [4, 8, 5], [1, 0, 6], [1, 6, 5], [1, 5, 2], [2, 5, 4], [2, 4, 3],
          [10, 11, 16], [16, 11, 15], [15, 11, 12], [15, 12, 14], [14, 12, 13], [18, 15, 14], [14, 18, 19], [17, 16, 15], [17, 15, 18], [17, 18, 19],
          [1, 11, 0], [0, 11, 10],
          [0, 10, 6], [6, 10, 16], [6, 17, 16], [6, 7, 17], [7, 17, 19], [7, 19, 9], [4, 14, 19], [4, 19, 9], [4, 14, 13], [4, 13, 3],
          [3, 13, 12], [3, 12, 2],
          [2, 12, 11], [2, 11, 1]];
      } else {
        const neckApex = neckHalf / Math.tan(g.angle);
        v = [
          [leftX, -halfHeight, front], [leftX, halfHeight, front], [rightX, halfHeight, front], [rightX, -halfHeight, front],
          [mouthOuter + offset, -halfHeight, front], [offset, neckApex, front], [-mouthOuter + offset, -halfHeight, front],
          [-neckOuter + offset, -halfHeight, back], [-mouthInner + offset, halfHeight, back], [mouthInner + offset, halfHeight, back], [neckOuter + offset, -halfHeight, back],
        ];
        front -= wall; back -= wall;
        mouthInner += shift; mouthOuter += shift; neckOuter += shift;
        v = v.concat([
          [leftX, -halfHeight, front], [leftX, halfHeight, front], [rightX, halfHeight, front], [rightX, -halfHeight, front],
          [mouthOuter + offset, -halfHeight, front], [shift + offset, neckApex, front], [-shift + offset, neckApex, front], [-mouthOuter + offset, -halfHeight, front],
          [-neckOuter + offset, -halfHeight, back], [-mouthInner + offset, halfHeight, back], [mouthInner + offset, halfHeight, back], [neckOuter + offset, -halfHeight, back],
        ]);
        idx = [[8, 7, 10], [8, 10, 9], [5, 8, 7], [5, 7, 6], [4, 10, 9], [4, 9, 5], [1, 0, 6], [1, 6, 5], [1, 5, 2], [2, 5, 4], [2, 4, 3],
          [11, 12, 18], [18, 12, 17], [17, 12, 16], [16, 12, 13], [16, 13, 15], [15, 13, 14], [21, 16, 15], [21, 15, 22], [19, 18, 17], [19, 17, 20], [19, 20, 21], [19, 21, 22],
          [1, 12, 11], [1, 11, 0],
          [0, 11, 18], [0, 18, 6], [7, 19, 18], [7, 18, 6], [7, 19, 22], [7, 22, 10], [10, 22, 15], [10, 15, 4], [4, 15, 14], [4, 14, 3],
          [3, 14, 13], [3, 14, 2],
          [2, 13, 12], [2, 12, 1], [5, 16, 21], [5, 21, 9], [9, 21, 20], [9, 20, 8], [5, 17, 20], [5, 20, 8]];
      }
      for (const tri of idx) for (const at of tri) out.push(...v[at]);
    }
    return out;
  }

  // The width a dovetail takes (CutUtils.cpp `Cut::calculate_groove_width`).
  function grooveWidth(g, radius) {
    const flap = Math.abs(g.flaps_angle) < 1e-9 ? g.depth : g.depth / Math.sin(g.flaps_angle);
    const flaps = 2 * flap * Math.cos(g.flaps_angle);
    const neck = 0.5 * g.width;
    const mouth = 0.5 * (g.width + flaps);
    const halfHeight = 0.5 * (1.5 * (1.5 * radius));
    const taper = halfHeight * Math.tan(g.angle);
    return 2 * Math.max(mouth + taper, neck + taper);
  }

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
      // "Keep cut editable", on unless turned off (`m_keep_cut_editable`).
      editable: true,
    },
    // A cut to set the tool up with when it next opens: one put back by
    // "Edit cut..." (`arm_reedit`) or copied by "Copy cut to..."
    // (`arm_copy`).
    pending: null,
    panelNode: null,
    shapes: null,
    // "Mode": 'planar' or 'dovetail', kept from one cut to the next as the
    // reference's gizmo keeps `m_mode`.
    mode: 'planar',

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
        // "Pick face": armed, and the face under the pointer.
        picking: false,
        picked: null,
        // The cut's pieces once one is given to the other side (the
        // reference's part selection), for the plane they were made for;
        // with a dovetail, what it would make.
        parts: null,
        // The dovetail (`m_groove`, set up in `update_bb`), and what it
        // started at, for each field's reset. Its depth goes by the box's
        // size (`get_grabber_mean_size` before the grips were drawn at a
        // fixed size on the screen, which made it go by the camera's zoom).
        groove: null,
        grooveStart: null,
        grooveValid: null,
        // The connectors (connectors.js): each in the plane's own
        // coordinates; which are chosen, which do not fit; whether they are
        // being placed, on the cut face (`cap`).
        connectors: [],
        selected: new Set(),
        invalid: new Set(),
        stats: null,
        editing: false,
        cap: null,
      };
      const mean = (box[1][0] - box[0][0] + box[1][1] - box[0][1] + box[1][2] - box[0][2]) / 30;
      const depth = Math.max(1, 0.5 * mean);
      this.open.groove = { depth, width: 4 * depth, flaps_angle: Math.PI / 3, angle: 0, depth_tolerance: 0.1, width_tolerance: 0.1, count: 1, gap: 10 };
      this.open.grooveStart = { ...this.open.groove };
      const pending = this.pending;
      this.pending = null;
      if (pending && pending.copy === placing.id) this.apply(pending.recipe, pending.reedit);
      this.changed(false);
    },

    // The tool set up as a cut was made (`apply_recipe_to_gizmo`): its
    // mode, its plane, its settings and its dovetail. `reedit`: the cut is
    // being made again, of the object it was put back as.
    apply(recipe, reedit) {
      const o = this.open;
      const m = recipe.plane;
      o.axes = [m.slice(0, 3), m.slice(4, 7), m.slice(8, 11)];
      o.middle = m.slice(12, 15);
      this.mode = recipe.mode === 'dovetail' ? 'dovetail' : 'planar';
      for (const key of ['keep_upper', 'keep_lower', 'place_on_cut_upper', 'place_on_cut_lower', 'flip_upper', 'flip_lower', 'to_parts']) this.options[key] = Boolean(recipe[key]);
      o.thickness = Math.min(THICKNESS_MAX, Math.max(0, recipe.thickness || 0));
      o.from = recipe.thickness_from || 'centred';
      const g = recipe.groove;
      if (g) {
        o.groove = { depth: g.depth, width: g.width, flaps_angle: g.flaps_angle, angle: g.angle, depth_tolerance: g.depth_tolerance, width_tolerance: g.width_tolerance, count: Math.min(GROOVE_COUNT_MAX, Math.max(1, Math.round(g.count))), gap: g.gap };
        const [depth, width, flaps, angle] = g.init;
        o.grooveStart = { ...o.groove, depth, width, flaps_angle: flaps, angle };
        // The dovetails keep the spacing they were cut with, here and on
        // an object of another size (`m_groove_plane_radius`).
        if (g.plane_radius > 0) o.grooveRadius = g.plane_radius;
      }
      if (recipe.connectors && recipe.connectors.length) this.takeConnectors(recipe.connectors);
      if (reedit) o.reedit = { missing: Boolean(recipe.missing) };
    },

    // The radius the dovetails are spaced by.
    grooveRadius() {
      return this.open.grooveRadius || this.open.radius;
    },

    // "Edit cut...": the halves of the cut that made a copy's object put
    // back as the object was, and the tool opened on it as the cut was
    // made (GUI_ObjectList.cpp `edit_cut`).
    async editCut(copy) {
      if (this.editor.tool) this.editor.setTool(null);
      const answer = await this.editor.commit({ op: 'cut_edit', copy }, { select: true });
      if (!answer || !answer.recipe || !answer.made || !answer.made.length) return;
      this.pending = { copy: answer.made[0], recipe: answer.recipe, reedit: true };
      this.editor.setTool('cut');
    },

    // "Copy cut to...": the tool opened on `target` with the cut of
    // `source`'s object set up on it, in its own place (`copy_cut_to`).
    async copyCut(source, target) {
      const project = this.host.project();
      if (!project) return;
      let recipe;
      try {
        recipe = await this.host.call('cut_recipe', { project: project.id, copy: source, onto: target });
      } catch (error) {
        this.host.say(error.message, true);
        return;
      }
      if (this.editor.tool) this.editor.setTool(null);
      this.stage.select([target]);
      this.pending = { copy: target, recipe, reedit: false };
      this.editor.setTool('cut');
    },

    close() {
      if (this.open) { this.dropParts(); this.dropCap(); }
      // Closed without cutting again: the halves come back
      // (`cancel_reedit`), by the step that put them away.
      const reedit = this.open && this.open.reedit;
      this.open = null;
      if (reedit) {
        setTimeout(() => {
          const project = this.host.project();
          if (project && project.edit && project.edit.undo === 'Edit cut') this.editor.step('undo');
        }, 0);
      }
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
      if (same) return;
      // Another copy chosen while a cut is being made again: that is put
      // away first, as the reference's gizmo closes.
      if (this.open.reedit) { this.editor.setTool(null); return; }
      this.start(placing);
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
      this.connectorsMirrored(false);
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
      this.connectorsMirrored(true);
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
      // The pieces were of another plane: the reference lets them go as the
      // plane moves (`reset_cut_by_contours`).
      if (o.parts && o.parts.key !== this.partsKey()) this.dropParts();
      this.stage.scene.cutting = {
        copy: o.copy,
        // A dovetail tints neither side (`apply_color_clip_plane_colors`:
        // both the plane's colour); its result is shown once it is made.
        split: { plane: [...n, M.dot(n, o.middle)], upper: this.dovetail() ? PLANE.slice(0, 3) : UPPER, lower: this.dovetail() ? PLANE.slice(0, 3) : LOWER },
        pieces: o.parts ? { mesh: o.parts.mesh, ranges: o.parts.ranges, sides: o.parts.sides, offset: o.parts.offset, upper: UPPER, lower: LOWER } : null,
        // While connectors are placed, what is over the plane is not drawn.
        clip: o.editing ? [...n, M.dot(n, o.middle)] : null,
      };
      // The cut face was of another plane.
      if (o.cap && o.cap.key !== JSON.stringify(this.frame())) this.dropCap();
      this.checkConnectors();
      this.ask(soon);
      this.panel();
      this.stage.frame();
    },

    dovetail() {
      return this.mode === 'dovetail';
    },

    // The dovetail as the session takes it.
    grooveArgs() {
      const o = this.open;
      const start = o.grooveStart;
      return { ...o.groove, plane_radius: this.grooveRadius(), init: [start.depth, start.width, start.flaps_angle, start.angle] };
    },

    // "Mode": "Planar" or "Dovetail" ("Change cut mode", `switch_to_mode`).
    setMode(mode) {
      if (!this.open || mode === this.mode) return;
      this.mode = mode;
      this.dropParts();
      this.open.grooveValid = null;
      this.open.asked = null;
      this.changed();
    },

    // A field of the dovetail changed ("Groove change").
    setGroove(key, value) {
      const o = this.open;
      if (!Number.isFinite(value)) { this.panel(); return; }
      o.groove = { ...o.groove, [key]: value };
      o.asked = null;
      this.changed();
    },

    // ---- what the session says of the plane ----

    // The section and the object's reach, asked for with the latest plane;
    // one call at a time, the last plane wanted asked next.
    async ask() {
      const o = this.open;
      const project = this.host.project();
      if (!o || !project) return;
      const plane = this.frame();
      const key = `${plane.join(',')}|${this.dovetail() ? JSON.stringify(o.groove) : ''}`;
      if (o.asked === key) return;
      o.wanted = key;
      if (o.busy) return;
      o.busy = true;
      try {
        while (o.wanted && o.wanted !== o.asked && this.open === o) {
          const asking = o.wanted;
          const args = { project: project.id, copy: o.copy, plane: asking.split('|')[0].split(',').map(Number) };
          if (this.dovetail()) args.groove = this.grooveArgs();
          const answer = await this.host.call('cut_section', args);
          if (this.open !== o) return;
          o.asked = asking;
          o.section = answer;
          o.grooveValid = answer.groove === undefined ? null : answer.groove;
          this.setSection(this.dovetail() ? [] : answer.lines);
          // The connectors are measured on the cut face where the plane is.
          if (!this.dovetail() && (o.editing || o.connectors.length)) this.fetchCap();
          this.checkConnectors();
          // The dovetail's result shown while the plane stands still
          // (`reset_cut_by_contours`, `process_contours`).
          if (this.dovetail() && o.grooveValid && !this.held) this.previewGroove();
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
      // Not while connectors are placed, nor with one that does not fit.
      if (this.open && !this.dovetail() && (this.open.editing || (this.open.invalid && this.open.invalid.size))) return false;
      const parts = this.open && this.open.parts;
      // A dovetail only where it can be cut (`has_valid_groove`).
      if (this.open && this.dovetail()) return this.open.grooveValid === true && (p.keep_upper || p.keep_lower) && !this.editor.busy;
      // With pieces given to sides, not every piece on one side
      // (`m_part_selection.is_one_object()`).
      if (parts && parts.sides.every((side) => side === parts.sides[0])) return false;
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
        connectors: this.dovetail() ? [] : this.connectorsArgs(),
        snap: this.snap,
        // Cut to parts is not offered with pieces given to sides.
        ...(o.parts && !this.dovetail() ? { sides: o.parts.sides, to_parts: false } : {}),
        ...(this.dovetail() ? { groove: this.grooveArgs(), to_parts: false, thickness: 0, sides: null } : {}),
      };
      // A cut made again is one step with putting it back.
      if (o.reedit) args.amend = 'Edit cut';
      o.reedit = null;
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

    // A dovetail's own grips, where they stand.
    grooveGrips() {
      const o = this.open;
      const [u, v] = o.axes;
      const reach = 0.5 * o.radius;
      const grips = [
        { part: 'turn-z', axis: 2, at: M.along(o.middle, v, -1.75 * reach) },
        { part: 'move-x', axis: 0, at: M.along(o.middle, u, 0.75 * reach) },
      ];
      if (o.groove.angle > 0) grips.push({ part: 'move-y', axis: 1, at: M.along(o.middle, v, 0.75 * reach) });
      return grips;
    },

    // What is under a pixel: the grip, a cone, the plane, or null.
    handleAt(x, y) {
      if (!this.open || this.editor.busy) return null;
      if (this.open.editing) return this.connectorHandle(x, y);
      const frame = this.stage.matrices();
      const view = (point) => this.editor.toView(frame, point);
      const { at, size } = this.grip();
      const px = size / this.mm();
      // A dovetail's own grips (`render_cut_plane_grabbers`, its
      // tongue-and-groove part): a ball to turn it about its normal, a
      // cube to move it along its x, and along its y once its sides lean.
      if (this.dovetail()) {
        for (const extra of this.grooveGrips()) {
          const p = view(extra.at);
          if (Math.hypot(x - p[0], y - p[1]) < px) return { kind: 'cut', part: extra.part, axis: extra.axis, id: `cut-${extra.part}` };
        }
      }
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
      if (handle.part === 'plane') return t('Click to flip the cut plane\nDrag to move the cut plane\nRight-click a part to assign it to the other side');
      return '';
    },

    hover(x, y) {
      if (this.open && this.open.picking) {
        this.pickAt(x, y);
        this.stage.canvas.title = '';
        return null;
      }
      const handle = this.handleAt(x, y);
      this.stage.canvas.title = this.tip(handle);
      return handle;
    },

    // ---- dragging ----

    // What a press takes: a handle, or with Shift a line to lay the plane
    // along (`process_cut_line`).
    grab(x, y, event) {
      if (!this.open || this.editor.busy) return null;
      // Armed, a click lays the plane on the face (`on_mouse`: the press is
      // the pick's and nothing else's).
      if (this.open.editing) return this.connectorGrab(x, y, event);
      if (this.open.picking) return { kind: 'cut', part: 'pick', x, y, tap: () => this.applyPick(x, y) };
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
      } else if (grab.part === 'move-x' || grab.part === 'move-y') {
        grab.along = grab.axes[grab.part === 'move-x' ? 0 : 1];
        grab.origin = o.middle.slice();
        grab.start = this.editor.along(grab.origin, grab.along, grab.x, grab.y);
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
      if (grab.part === 'connector' || grab.part === 'add') { this.connectorDrag(grab, x, y); return; }
      if (grab.part === 'pick') { grab.cancelled = true; return; }
      if (!grab.begun && !this.begin(grab)) { grab.cancelled = true; return; }
      if (grab.cancelled) return;
      const o = this.open;
      if (grab.part === 'line') {
        o.line = { from: grab.from, to: [x, y] };
        this.stage.frame();
        return;
      }
      if (grab.part === 'move-x' || grab.part === 'move-y') {
        // Along the plane's own x or y (`CutPlaneXMove`, `CutPlaneYMove`).
        let far = this.editor.along(grab.origin, grab.along, x, y) - grab.start;
        if (event.shiftKey) far = SNAP * Math.round(far / SNAP);
        o.middle = M.along(grab.middle, grab.along, far);
        this.changed(true);
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
      this.stage.canvas.title = `${'XYZ'[grab.handle.axis]}: ${round((angle * 180) / Math.PI, 1).toFixed(1)}°`;
      this.changed(true);
    },

    drop(grab, done) {
      if (grab.part === 'connector' || grab.part === 'add') { if (grab.begun) this.connectorDrop(grab, done); else this.held = null; return; }
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
      o.asked = null;
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

    // ---- giving a piece to the other side ----

    // What the pieces are made for: the plane and the thickness.
    partsKey() {
      const o = this.open;
      return `${this.frame().join(',')}|${o.thickness}|${o.from}|${this.dovetail() ? JSON.stringify(o.groove) : ''}`;
    },

    dropParts() {
      const o = this.open;
      if (o && o.parts && o.parts.mesh) this.stage.renderer.drop(o.parts.mesh);
      if (o) o.parts = null;
    },

    // The right button on the object (`on_mouse`, RightDown with no key
    // held): the piece under the pointer goes to the other side; the
    // pieces are asked for the first time.
    async rightClick(x, y) {
      const o = this.open;
      const project = this.host.project();
      if (!o || !project || this.editor.busy || o.picking) return;
      if (o.editing) { this.connectorRightClick(x, y); return; }
      if (this.dovetail()) return;
      if (!(await this.fetchParts())) return;
      const piece = this.pieceAt(x, y);
      if (piece !== null) o.parts.sides[piece] = !o.parts.sides[piece];
      // Cut to parts is not offered with pieces given to sides.
      this.options.to_parts = false;
      this.changed();
    },

    // The pieces for the plane as it stands, asked for when they are not
    // here. False when they could not be had.
    async fetchParts() {
      const o = this.open;
      const project = this.host.project();
      const key = this.partsKey();
      if (o.parts && o.parts.key === key) return true;
      const args = { project: project.id, copy: o.copy, plane: this.frame(), thickness: o.thickness, thickness_from: o.from };
      if (this.dovetail()) args.groove = this.grooveArgs();
      let info;
      let bytes;
      try {
        info = await this.host.call('cut_parts', args);
        bytes = await this.stage.bytes('cut_parts_mesh', args);
      } catch (error) {
        this.host.say(error.message, true);
        return false;
      }
      if (this.open !== o || this.partsKey() !== key) return false;
      this.dropParts();
      const view = new DataView(bytes);
      const vertices = view.getUint32(8, true);
      const triangles = view.getUint32(12, true);
      o.parts = {
        key,
        ranges: info.ranges,
        sides: info.upper.slice(),
        offset: info.offset,
        mesh: this.stage.renderer.meshFromBytes(bytes),
        points: new Float32Array(bytes, 24, vertices * 3),
        corners: new Uint32Array(bytes, 24 + 12 * vertices, triangles * 3),
      };
      return true;
    },

    // What a dovetail would make, each side in its colour.
    async previewGroove() {
      if (await this.fetchParts()) this.changed();
    },

    // The piece a line of sight meets first (`toggle_selection`).
    pieceAt(x, y) {
      const parts = this.open.parts;
      const ray = this.stage.camera.ray(x, y);
      const o = sub(ray.origin, parts.offset);
      const d = ray.direction;
      const p = parts.points;
      const c = parts.corners;
      let best = Infinity;
      let found = -1;
      for (let t = 0; t < c.length; t += 3) {
        const a = c[t] * 3;
        const b = c[t + 1] * 3;
        const e = c[t + 2] * 3;
        const e1 = [p[b] - p[a], p[b + 1] - p[a + 1], p[b + 2] - p[a + 2]];
        const e2 = [p[e] - p[a], p[e + 1] - p[a + 1], p[e + 2] - p[a + 2]];
        const h = M.cross(d, e2);
        const det = M.dot(e1, h);
        if (Math.abs(det) < 1e-12) continue;
        const s0 = [o[0] - p[a], o[1] - p[a + 1], o[2] - p[a + 2]];
        const u = M.dot(s0, h) / det;
        if (u < 0 || u > 1) continue;
        const q = M.cross(s0, e1);
        const v = M.dot(d, q) / det;
        if (v < 0 || u + v > 1) continue;
        const far = M.dot(e2, q) / det;
        if (far > 0 && far < best) { best = far; found = t / 3; }
      }
      if (found < 0) return null;
      const piece = parts.ranges.findIndex(([first, count]) => found >= first && found < first + count);
      return piece < 0 ? null : piece;
    },

    // ---- "Pick face" ----

    // Arms or disarms it (`m_facet_picker.set_active`).
    setPicking(on) {
      if (!this.open) return;
      this.open.picking = on;
      this.open.picked = null;
      this.panel();
      this.stage.frame();
    },

    // The face under a pixel, asked of the session (its tree of boxes),
    // one question at a time, the last pixel asked next.
    async pickAt(x, y) {
      const o = this.open;
      o.wantPick = [x, y];
      if (o.picking_busy) return;
      o.picking_busy = true;
      try {
        while (o.wantPick && this.open === o && o.picking) {
          const [px, py] = o.wantPick;
          o.wantPick = null;
          o.picked = await this.faceAt(px, py);
          this.stage.frame();
        }
      } finally {
        o.picking_busy = false;
      }
    },

    async faceAt(x, y) {
      const project = this.host.project();
      if (!project || !this.open) return null;
      const ray = this.stage.camera.ray(x, y);
      try {
        const answer = await this.host.call('cut_pick', { project: project.id, copy: this.open.copy, origin: ray.origin, direction: ray.direction });
        return answer.hit;
      } catch (error) {
        this.host.say(error.message, true);
        return null;
      }
    },

    // The plane laid on the face under a pixel, its normal the face's,
    // its middle where the line of sight met it ("Cut plane from face",
    // `apply_picked_facet`), when the plane then cuts the object (its box
    // in the new plane's axes crosses it, by half a millimetre). It is
    // disarmed after one face, as the reference's is.
    async applyPick(x, y) {
      const o = this.open;
      if (!o) return;
      const hit = await this.faceAt(x, y);
      if (!hit || this.open !== o) return;
      const n = unit(hit.normal);
      const z = [0, 0, 1];
      const axis = M.cross(z, n);
      const angle = Math.atan2(M.length(axis), M.dot(z, n));
      const pivot = M.length(axis) > 1e-9 ? unit(axis) : [1, 0, 0];
      const axes = M.rows.tidy([turn([1, 0, 0], pivot, angle), turn([0, 1, 0], pivot, angle), n]);
      const plane = [...axes[0], 0, ...axes[1], 0, ...axes[2], 0, ...hit.point, 1];
      const project = this.host.project();
      let answer;
      try {
        answer = await this.host.call('cut_section', { project: project.id, copy: o.copy, plane });
      } catch (error) {
        this.host.say(error.message, true);
        return;
      }
      const reach = answer.reach;
      if (!reach || !(reach[1][2] > -PAST && reach[0][2] < PAST)) return;
      this.remember();
      o.axes = axes;
      o.middle = hit.point.slice();
      o.picking = false;
      o.picked = null;
      this.changed();
    },

    // ---- keys ----

    // The tool's own keys, asked before the others: Up and Down move the
    // plane by 1 mm. True when the key was the tool's.
    key(event) {
      if (this.open && this.open.editing && this.connectorKey(event)) return true;
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
      // Placing connectors: the cut face, the section and the connectors
      // only (`render_connectors`, the plane hidden).
      if (o.editing) {
        this.drawConnectors(frame);
        const r = this.stage.renderer;
        r.gl.disable(r.gl.DEPTH_TEST);
        if (this.sectionLines) r.drawLines(this.sectionLines, M.multiply(frame.projection, frame.view), SECTION, 1.5);
        r.gl.enable(r.gl.DEPTH_TEST);
        return;
      }
      if (!this.dovetail() && o.connectors.length) this.drawConnectors(frame);
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
      if (this.dovetail()) {
        // The dovetail's plane (`its_make_groove_plane`), a tenth more
        // see-through (`render_cut_plane`).
        const key = `${JSON.stringify(o.groove)}|${this.grooveRadius()}`;
        if (!this.groovePlane || this.groovePlane.key !== key) {
          if (this.groovePlane) r.drop(this.groovePlane.mesh);
          this.groovePlane = { key, mesh: r.triangles(groovePlane(o.groove, this.grooveRadius())) };
        }
        const valid = o.grooveValid !== false;
        r.drawFlat(this.groovePlane.mesh, M.chain(view, plane), [...(valid ? PLANE : PLANE_ERROR).slice(0, 3), 0.4]);
      } else {
        for (const at of low === high ? [0] : [low, high]) {
          r.drawFlat(shapes.square, M.chain(view, plane, M.translation(0, 0, at), M.scaling(half, half, 1)), colour);
        }
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
      // A dovetail's own grips: the ball to turn it about its normal in the
      // plane's colour (blue under the pointer), the cubes in the plane's
      // colour (the axis's under the pointer).
      if (this.dovetail()) {
        for (const extra of this.grooveGrips()) {
          const id = `cut-${extra.part}`;
          const tint = lit(id) ? [[1, 60 / 255, 91 / 255, 1], [100 / 255, 200 / 255, 24 / 255, 1], [47 / 255, 136 / 255, 233 / 255, 1]][extra.axis] : [...PLANE.slice(0, 3), 1];
          r.drawLines(shapes.line, M.chain(view, M.translation(...o.middle), this.pointing([0, 0, 0], unit(sub(extra.at, o.middle)), [1, 1, 1]), M.scaling(1, 1, M.length(sub(extra.at, o.middle)))), GRABBER, 1.5);
          if (extra.part === 'turn-z') r.drawMesh(shapes.sphere, frame, M.chain(M.translation(...extra.at), M.scaling(0.75 * size, 0.75 * size, 0.75 * size)), { tint });
          else r.drawFlat(this.editor.parts().cube, M.chain(view, M.translation(...extra.at), M.scaling(size, size, size)), tint);
        }
      }
      const turning = held === 'cut-turn-0' || held === 'cut-turn-1';
      for (const cone of this.cones()) {
        const id = `cut-turn-${cone.axis}`;
        if ((turning || this.held) && held !== id) continue;
        r.drawFlat(this.editor.parts().cone, M.chain(view, this.pointing(cone.at, cone.points, [0.75 * size / 0.32, 0.75 * size / 0.32, 1.8 * size])), AXES[cone.axis]);
      }
      // The face "Pick face" would take.
      if (o.picking && o.picked) {
        const lift = M.scale(unit(o.picked.normal), LIFT);
        const face = r.triangles(o.picked.corners.flatMap((corner) => M.add(corner, lift)));
        gl.enable(gl.DEPTH_TEST);
        r.blend(true);
        r.drawFlat(face, view, PICKED);
        r.drop(face);
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
      if (o.editing) {
        rows.push(...this.connectorRows(busy));
        node.replaceChildren(...rows);
        return;
      }
      // A cut being made again (`render_reedit_notice`).
      if (o.reedit) {
        rows.push(el('p', { className: 'edit-hint', textContent: t('Editing an existing cut. Perform cut replaces both halves.') }));
        if (o.reedit.missing) rows.push(el('p', { className: 'edit-hint cut-warning', role: 'status', textContent: t('One half of this cut is missing from the project. Cutting again will produce both halves.') }));
      }
      const dovetail = this.dovetail();
      // "Mode": "Planar" or "Dovetail" (`render_cut_mode_combo`).
      // Two joined buttons, as STYLE.md has a choice of which of two.
      const mode = el('div', { className: 'joined segmented' });
      mode.setAttribute('role', 'group');
      mode.setAttribute('aria-label', t('Mode'));
      for (const [value, label] of [['planar', t('Planar')], ['dovetail', t('Dovetail')]]) {
        const button = el('button', { type: 'button', textContent: label, disabled: busy });
        button.setAttribute('aria-pressed', String(this.mode === value));
        button.addEventListener('click', () => this.setMode(value));
        mode.append(button);
      }
      rows.push(el('div', { className: 'cut-row' }, el('span', { className: 'edit-label', textContent: t('Mode') }), el('span', { className: 'cut-line' }, mode)));
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
      if (dovetail) rows.push(...this.grooveRows(busy));
      // "Thickness": what the cut takes away, and from which side.
      if (!dovetail) {
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
      const connected = o.connectors.length > 0;
      const resetCut = el('button', { type: 'button', textContent: t('Reset cut'), title: t('Reset cutting plane and remove connectors'), disabled: busy || (this.atStart() && !connected) });
      resetCut.addEventListener('click', () => { o.connectors = []; o.selected = new Set(); this.reset(); });
      // "Add connectors" / "Edit connectors": both parts kept as objects
      // of their own.
      const add = el('button', { type: 'button', textContent: connected ? t('Edit connectors') : t('Add connectors'), disabled: busy || !this.connectorsAllowed() });
      add.addEventListener('click', () => this.startConnectors());
      // "Pick face": armed by a press, a click on the object then lays the
      // plane on the face under it.
      const pick = el('button', { type: 'button', textContent: t('Pick face'), title: t('Click a model face to set the cut plane flush with it; use Cut position to offset'), disabled: busy || connected });
      pick.setAttribute('aria-pressed', String(o.picking));
      pick.classList.toggle('cut-armed', o.picking);
      pick.addEventListener('click', () => this.setPicking(!o.picking));
      const picking = o.picking ? [el('span', { className: 'edit-hint', textContent: t('Click a face of the model.') })] : [];
      rows.push(el('div', { className: 'cut-foot cut-start' }, add, resetCut, pick, ...picking));
      }
      rows.push(el('hr', { className: 'arrange-rule' }));
      // "After cut".
      rows.push(el('span', { className: 'edit-label', textContent: t('After cut') }));
      const side = (label, colour, keep, place, flip) => {
        const box = (word, key, disabled, act) => {
          // With connectors both parts are kept (`render_part_action_line`).
          const forced = key.startsWith('keep') && o.connectors.length > 0 && !dovetail;
          const input = el('input', { type: 'checkbox', checked: forced || Boolean(p[key]), disabled: disabled || busy || forced });
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
      // Not with pieces given to sides, nor with a dovetail.
      const parts = el('input', { type: 'checkbox', checked: p.to_parts && !o.parts && !dovetail, disabled: busy || Boolean(o.parts) || dovetail || o.connectors.length > 0 });
      parts.addEventListener('change', () => {
        p.to_parts = parts.checked;
        // Both parts are kept, as they stood (`m_keep_as_parts`).
        if (p.to_parts) Object.assign(p, { keep_upper: true, keep_lower: true, place_on_cut_upper: false, place_on_cut_lower: false, flip_upper: false, flip_lower: false });
        this.panel();
        this.stage.frame();
      });
      rows.push(el('label', { className: 'edit-check' }, parts, ` ${t('Cut to parts')}`));
      // "Keep cut editable".
      const editable = el('input', { type: 'checkbox', checked: p.editable, disabled: busy });
      editable.addEventListener('change', () => { p.editable = editable.checked; });
      rows.push(el('label', { className: 'edit-check', title: t('Remember this cut so it can be edited later: select either half and choose "Edit cut..." to reopen this panel with the same surface and settings.\n\nThe original, uncut shape is stored in the project file, so a cut object takes about twice the space. Turn this off for a one-off cut.') }, editable, ` ${t('Keep cut editable')}`));
      // What the keys and the pointer do (`m_shortcuts_cut`).
      const help = el('div', { className: 'paint-help' });
      help.append(el('span', { className: 'paint-key', textContent: `${t('Shift+')}${t('Drag')}` }), el('span', { textContent: t('Draw cut line') }));
      rows.push(help);
      // The warnings (`render_input_window_warning`).
      const warnings = [];
      if (!p.keep_upper && !p.keep_lower && !p.to_parts) warnings.push(t('Select at least one object to keep after cutting.'));
      if (!this.meets()) warnings.push(t('Cut plane is placed out of object'));
      if (!dovetail) warnings.push(...this.connectorWarnings().map((line) => line.replace(`${t('Warning')}: `, '')));
      else if (dovetail && o.grooveValid === false) warnings.push(t('Cut plane with groove is invalid'));
      for (const warning of warnings) rows.push(el('p', { className: 'edit-hint cut-warning', role: 'status', textContent: `${t('Warning')}: ${warning}` }));
      const perform = el('button', { type: 'button', className: 'primary', textContent: t('Perform cut'), disabled: !this.canPerform() });
      perform.addEventListener('click', () => this.perform());
      rows.push(el('div', { className: 'cut-foot' }, perform));
      node.replaceChildren(...rows);
    },

    // The dovetail's fields (`render_cut_plane_input_window`, its
    // tongue-and-groove part): "Groove" with "Depth" and "Width", each with
    // its tolerance, "Flap Angle", "Groove Angle"; "Multiple" with "Count",
    // "Gap" and the "Spacing" they come to. Each with its reset.
    grooveRows(busy) {
      const { el } = this.host;
      const o = this.open;
      const g = o.groove;
      const start = o.grooveStart;
      const mm = t('mm');
      const reset = (label, keys) => {
        const changed = keys.some((key) => Math.abs(g[key] - start[key]) > 1e-9);
        const node = el('button', { type: 'button', className: 'icon tool', title: `${t('Reset')}: ${label}`, disabled: busy || !changed, textContent: '↺' });
        node.setAttribute('aria-label', `${t('Reset')}: ${label}`);
        node.addEventListener('click', () => {
          const back = {};
          for (const key of keys) back[key] = start[key];
          o.groove = { ...o.groove, ...back };
          o.asked = null;
          this.changed();
        });
        return node;
      };
      const number = (label, key, { min, max, step = 0.1, unit = mm, angle = false, disabled = false, title = label }) => {
        const shown = angle ? degrees(g[key]) : g[key];
        const text = key === 'count' ? String(shown) : angle ? round(shown, 1).toFixed(1) : fixed(shown);
        const input = el('input', { type: 'number', min: String(min), max: String(max), step: String(step), value: text, title, disabled: busy || disabled });
        input.dataset.field = `groove-${key}`;
        input.setAttribute('aria-label', title);
        input.addEventListener('change', () => {
          const value = Math.min(max, Math.max(min, Number(input.value)));
          this.setGroove(key, angle ? radians(value) : (key === 'count' ? Math.round(value) : value));
        });
        return el('span', { className: 'unit' }, input, unit ? ` ${unit}` : '');
      };
      const row = (label, ...cells) => el('div', { className: 'cut-row' }, el('span', { className: 'edit-label', textContent: label }), el('span', { className: 'cut-line' }, ...cells));
      const tolerance = t('Tolerance');
      const width = grooveWidth(g, this.grooveRadius());
      const bed = this.host.settled() && this.host.settled().printer && this.host.settled().printer.bed;
      const plate = bed ? Math.max(bed[2] - bed[0], bed[3] - bed[1]) : 300;
      return [
        el('span', { className: 'edit-label cut-group', textContent: t('Groove') }),
        row(t('Depth'), number(t('Depth'), 'depth', { min: 0.1, max: 100 }), number(tolerance, 'depth_tolerance', { min: 0, max: Math.min(0.3 * g.depth, 1.5), step: 0.05, title: `${t('Depth')}: ${tolerance}` }), reset(t('Depth'), ['depth', 'depth_tolerance'])),
        row(t('Width'), number(t('Width'), 'width', { min: 0.1, max: 200 }), number(tolerance, 'width_tolerance', { min: 0, max: Math.min(0.3 * g.width, 1.5), step: 0.05, title: `${t('Width')}: ${tolerance}` }), reset(t('Width'), ['width', 'width_tolerance'])),
        row(t('Flap Angle'), number(t('Flap Angle'), 'flaps_angle', { min: 30, max: 120, step: 1, unit: '°', angle: true }), reset(t('Flap Angle'), ['flaps_angle'])),
        row(t('Groove Angle'), number(t('Groove Angle'), 'angle', { min: 0, max: 15, step: 0.5, unit: '°', angle: true }), reset(t('Groove Angle'), ['angle'])),
        el('span', { className: 'edit-label cut-group', textContent: t('Multiple') }),
        row(t('Count'), number(t('Count'), 'count', { min: 1, max: GROOVE_COUNT_MAX, step: 1, unit: '' }), reset(t('Count'), ['count'])),
        row(t('Gap'), number(t('Gap'), 'gap', { min: 0, max: plate / Math.max(g.count - 1, 1), disabled: g.count === 1 }), reset(t('Gap'), ['gap'])),
        row(t('Spacing'), el('span', { className: 'cut-size', textContent: `${fixed(g.gap + width)} ${mm}` })),
      ];
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
