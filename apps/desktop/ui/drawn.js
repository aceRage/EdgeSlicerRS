'use strict';

// "Cut"'s drawn surface ("Surface": "Draw"; Gizmos/GLGizmoCut.cpp
// `render_draw_surface_inputs`, `draw_on_mouse`, `render_draw_stroke`,
// `render_draw_chain_endpoints`, `close_draw_chain_on_surface`): a line
// drawn on the model stroke by stroke, each carrying on from an end until
// it closes or is used open, and the sweep it makes ("Direction",
// "Extension", "Extension angle", "Angle", "Depth", "Smoothing"); its
// points can be moved, taken away and added. The session finds where the
// pointer meets the model (`cut_draw_hits`) and keeps the sums: the chain,
// the finished line put back onto the model, the extension's clamp, the
// checks and the surface (`cut_draw`); the page keeps the chain and its
// undo. While the line can be cut with, its halves are worked out by the
// booleans as a curved surface's are (curved.js). Added to `GL.cutter`.
(function (GL) {
  const M = GL.math;
  const cutter = GL.cutter;
  const sub = M.sub;

  // GLGizmoCut.cpp `render_draw_stroke`: the line's ribbon, its ends'
  // spheres (hovered, and armed to close), the handles.
  const RIBBON = [1, 0.65, 0, 1];
  const END = [1, 0.65, 0, 1];
  const END_HOT = [1, 0.85, 0.35, 1];
  const END_ARMED = [0.1, 0.95, 0.3, 1];
  const HANDLE = [0.75, 0.75, 0.75, 1];
  const HANDLE_HOT = [1, 1, 0, 1];
  // GLGizmoCut.hpp: how near an end a press carries the line on
  // (`DrawEndPickPx`), a handle's and a segment's reach.
  const END_PICK_PX = 16;
  const POINT_PICK_PX = 14;
  const SEGMENT_PICK_PX = 12;
  // DrawCut.hpp `MinChainSamples`, `DrawCutStroke::MinSamples`.
  const MIN_CHAIN = 6;
  const MIN_POINTS = 4;
  // The most rays one move of the pointer sends (`draw_interpolate_to`).
  const MOST_RAYS = 256;
  // "Depth": the slider's reach and the field's (GLGizmoCut.hpp).
  const DEPTH_SLIDER = 20;
  const DEPTH_MIN = 0.01;
  const DEPTH_MAX = 1000;

  const emptyChain = () => ({ samples: [], strokes: [], closed: false, finished_open: false });
  const copyChain = (c) => ({ samples: c.samples.map((s) => s.slice()), strokes: c.strokes.map((b) => b.slice()), closed: c.closed, finished_open: c.finished_open });

  Object.assign(cutter, {
    // The sweep, kept from one cut to the next as the gizmo keeps its
    // members (`m_draw_direction`, `m_draw_extension`, …).
    drawUi: { direction: 0, extension: 5, extAngleOn: false, extAngle: 0, angle: 0, depth: 3, smoothing: 0.2, view: [0, 0, -1] },

    freshDraw() {
      return { chain: emptyChain(), answer: null, asked: 0, capture: null, history: [], future: [], editing: false, points: null, reject: null, hoverEnd: null, hoverPoint: -1, surfaceGl: null, surfaceOf: null };
    },

    drawing() {
      const o = this.open;
      return Boolean(o && !this.dovetail() && o.surface === 'draw');
    },

    // Whether the line can be cut with: a cut surface, and neither side
    // empty (`can_perform_cut`, Draw).
    drawnReady() {
      const o = this.open;
      if (!this.drawing() || !o.draw) return false;
      const a = o.draw.answer;
      return Boolean(a && a.valid && !o.draw.capture);
    },

    // The drawn cut as the session takes it. Through all is hidden, so
    // off (`DrawThroughAllHidden`).
    drawArgs() {
      const o = this.open;
      const u = this.drawUi;
      return {
        drawn: {
          ...o.draw.chain,
          smoothing: u.smoothing,
          direction: u.direction,
          view: u.view,
          extension: u.extension,
          angle: u.angle,
          extension_angle: u.extAngleOn ? u.extAngle : null,
          through_all: false,
          depth: u.depth,
        },
      };
    },

    // The camera's direction into the part in the plane's frame, latched
    // when "View" is chosen and when a stroke ends (`latch_draw_view_dir`).
    latchView() {
      const o = this.open;
      const f = this.stage.camera.forward;
      const d = o.axes.map((axis) => M.dot(f, axis));
      this.drawUi.view = M.length(d) > 1e-9 ? M.normalize(d) : [0, 0, -1];
    },

    // A point of the plane's frame in the scene.
    planeToScene(p) {
      const o = this.open;
      return M.add(M.add(M.along(o.middle, o.axes[0], p[0]), M.scale(o.axes[1], p[1])), M.scale(o.axes[2], p[2]));
    },

    // ---- the session ----

    // The line as it is, changed by `op`, made ready by the session; the
    // last asked wins.
    async refreshDraw(op, extra = {}) {
      const o = this.open;
      const project = this.host.project();
      if (!o || !o.draw || !project) return null;
      const asked = ++o.draw.asked;
      let answer;
      try {
        answer = await this.host.call('cut_draw', {
          project: project.id, copy: o.copy, plane: this.frame(), ...this.drawArgs(), thickness: o.thickness, thickness_from: o.from, ...(op ? { op } : {}), ...extra,
        });
      } catch (error) {
        if (this.open === o) this.host.say(error.message, true);
        return null;
      }
      if (this.open !== o || !o.draw || o.draw.asked !== asked) return null;
      const { smoothing, ...chain } = answer.drawn;
      o.draw.chain = chain;
      o.draw.answer = answer;
      if (!o.draw.editing || o.draw.points === null || op === 'points') o.draw.points = answer.valid ? answer.path.map((s) => s.slice()) : null;
      if (!answer.valid) o.draw.editing = false;
      this.dropDrawSurface();
      this.dropPreview();
      o.previewStale = this.previewing();
      clearTimeout(this.previewTimer);
      if (this.previewing()) this.previewTimer = setTimeout(() => this.askPreview(), 60);
      this.changed();
      return answer;
    },

    // Where rays meet the copy, in the plane's frame (null for a miss).
    async drawHits(rays) {
      const o = this.open;
      const project = this.host.project();
      const answer = await this.host.call('cut_draw_hits', { project: project.id, copy: o.copy, plane: this.frame(), rays });
      o.draw.snap = answer.snap;
      return answer.hits;
    },

    rayAt(x, y) {
      const ray = this.stage.camera.ray(x, y);
      return [...ray.origin, ...ray.direction];
    },

    // ---- the line's undo (`push_draw_undo`, `draw_undo`, `draw_redo`) ----

    pushDraw() {
      const d = this.open.draw;
      d.history.push(copyChain(d.chain));
      if (d.history.length > 200) d.history.shift();
      d.future = [];
    },

    drawStep(which) {
      const d = this.open.draw;
      const [from, onto] = which === 'undo' ? [d.history, d.future] : [d.future, d.history];
      const chain = from.pop();
      if (!chain) return false;
      onto.push(copyChain(d.chain));
      d.chain = chain;
      d.editing = false;
      d.reject = null;
      this.refreshDraw(null);
      return true;
    },

    // "Surface": "Draw" starts with no line and no undo.
    startDraw() {
      const o = this.open;
      this.dropDrawSurface();
      o.draw = this.freshDraw();
      this.refreshDraw(null);
    },

    // ---- the pointer ----

    // Whether the copy is under a pixel.
    overCopy(x, y) {
      const frame = this.stage.matrices();
      const hit = this.stage.scene.pick(this.stage.camera, frame, x, y, { plates: false });
      const data = this.editor.data();
      return Boolean(hit && hit.placing !== undefined && data && data.placings[hit.placing] && data.placings[hit.placing].id === this.open.copy);
    },

    // The open line's end near a pixel: the back first when both are
    // (`draw_cut_chain_end_at_pixel`).
    endAt(x, y) {
      const c = this.open.draw.chain;
      if (!c.samples.length || c.closed) return null;
      const frame = this.stage.matrices();
      const far = (s) => {
        const p = this.editor.toView(frame, this.planeToScene(s));
        return Math.hypot(p[0] - x, p[1] - y);
      };
      const back = far(c.samples[c.samples.length - 1]);
      const front = far(c.samples[0]);
      if (back <= END_PICK_PX && back <= front) return 'back';
      if (front <= END_PICK_PX) return 'front';
      return null;
    },

    // The line's point under a pixel (`draw_point_at`).
    pointAt(x, y) {
      const pts = this.open.draw.points || [];
      const frame = this.stage.matrices();
      let best = -1;
      let near = POINT_PICK_PX;
      pts.forEach((s, i) => {
        const p = this.editor.toView(frame, this.planeToScene(s));
        const far = Math.hypot(p[0] - x, p[1] - y);
        if (far < near) { near = far; best = i; }
      });
      return best;
    },

    // The span of the line under a pixel (`draw_segment_at`).
    segmentAt(x, y) {
      const d = this.open.draw;
      const pts = d.points || [];
      const n = pts.length;
      if (n < 2) return -1;
      const frame = this.stage.matrices();
      const px = pts.map((s) => this.editor.toView(frame, this.planeToScene(s)));
      const spans = d.answer && d.answer.closed ? n : n - 1;
      let best = -1;
      let near = SEGMENT_PICK_PX * SEGMENT_PICK_PX;
      for (let i = 0; i < spans; i += 1) {
        const [a, b] = [px[i], px[(i + 1) % n]];
        const ab = [b[0] - a[0], b[1] - a[1]];
        const len2 = ab[0] * ab[0] + ab[1] * ab[1];
        if (len2 < 1e-9) continue;
        const t = Math.min(1, Math.max(0, ((x - a[0]) * ab[0] + (y - a[1]) * ab[1]) / len2));
        const d2 = (a[0] + t * ab[0] - x) ** 2 + (a[1] + t * ab[1] - y) ** 2;
        if (d2 < near) { near = d2; best = i; }
      }
      return best;
    },

    drawHover(x, y) {
      const d = this.open.draw;
      if (!d) return null;
      if (d.editing) {
        const at = this.pointAt(x, y);
        if (at !== d.hoverPoint) { d.hoverPoint = at; this.stage.frame(); }
        return at >= 0 ? { kind: 'cut', part: 'draw-point', id: `cut-draw-${at}` } : null;
      }
      const end = d.capture ? null : this.endAt(x, y);
      if (end !== d.hoverEnd) { d.hoverEnd = end; this.stage.frame(); }
      return null;
    },

    // A press (`draw_on_mouse`, LeftDown): on a handle while editing, a
    // drag moves it and Shift on a span adds a point; otherwise on the
    // model, a stroke from the line's end (or the first), refused with
    // its reason anywhere else.
    drawGrab(x, y, event) {
      const o = this.open;
      const d = o.draw;
      if (event.ctrlKey || event.metaKey || event.altKey) return null;
      if (d.editing) {
        if (event.shiftKey) {
          const seg = this.segmentAt(x, y);
          if (seg < 0) return null;
          return { kind: 'cut', part: 'draw-insert', x, y, tap: () => this.insertPoint(seg, x, y) };
        }
        const at = this.pointAt(x, y);
        if (at < 0) return null;
        return { kind: 'cut', part: 'draw-point', index: at, x, y, id: `cut-draw-${at}`, handle: { id: `cut-draw-${at}` }, tap: () => {} };
      }
      if (event.shiftKey || !this.overCopy(x, y)) return null;
      const c = d.chain;
      const at = !c.samples.length ? 'back' : c.closed ? null : this.endAt(x, y);
      if (!at) {
        d.reject = c.closed ? 'closed' : 'disjoint';
        this.panel();
        return null;
      }
      d.reject = null;
      d.hoverEnd = null;
      d.capture = { at, samples: [], last: [x, y], pending: Promise.resolve(), armed: false };
      this.captureRays([this.rayAt(x, y)]);
      return { kind: 'cut', part: 'draw', x, y, tap: () => {} };
    },

    // The rays' hits added to the stroke being drawn, in order.
    captureRays(rays) {
      const o = this.open;
      const cap = o.draw.capture;
      cap.pending = cap.pending.then(async () => {
        let hits;
        try {
          hits = await this.drawHits(rays);
        } catch (error) {
          return;
        }
        if (this.open !== o || o.draw.capture !== cap) return;
        for (const hit of hits) {
          if (hit) cap.samples.push(hit);
        }
        this.armCapture();
        this.stage.frame();
      });
    },

    // Whether letting go here would close the loop (`m_draw_snap_armed`).
    armCapture() {
      const d = this.open.draw;
      const cap = d.capture;
      const c = d.chain;
      const snap = d.snap || 1;
      cap.armed = false;
      if (!cap.samples.length) return;
      const last = cap.samples[cap.samples.length - 1];
      const dist = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
      if (c.samples.length) {
        const far = cap.at === 'back' ? c.samples[0] : c.samples[c.samples.length - 1];
        cap.armed = c.samples.length + cap.samples.length >= MIN_CHAIN && dist(last, far) <= snap;
      } else if (cap.samples.length >= MIN_CHAIN) {
        cap.armed = dist(last, cap.samples[0]) <= snap;
      }
    },

    // The pointer's way since the last hit, a pixel at a time
    // (`draw_interpolate_to`).
    drawDrag(grab, x, y) {
      const o = this.open;
      if (grab.part === 'draw-point') { this.movePoint(grab, x, y); return; }
      const cap = o.draw.capture;
      if (!cap) return;
      const [fx, fy] = cap.last;
      const steps = Math.min(Math.floor(Math.hypot(x - fx, y - fy)), MOST_RAYS);
      const rays = [];
      for (let k = 1; k <= steps; k += 1) {
        const t = k / (steps + 1);
        rays.push(this.rayAt(fx + (x - fx) * t, fy + (y - fy) * t));
      }
      rays.push(this.rayAt(x, y));
      cap.last = [x, y];
      this.captureRays(rays);
    },

    // The button let go: the stroke goes onto the line as one step, or
    // nothing when it does not carry it on.
    async drawDrop(grab, done) {
      const o = this.open;
      if (grab.part === 'draw-point') { await this.dropPoint(grab, done); return; }
      const d = o.draw;
      const cap = d.capture;
      if (!cap) return;
      await cap.pending;
      if (this.open !== o || d.capture !== cap) return;
      d.capture = null;
      this.latchView();
      if (!done || cap.samples.length < 2) { this.changed(); return; }
      this.pushDraw();
      const answer = await this.refreshDraw('append', { stroke: cap.samples, at: cap.at });
      if (answer && !answer.done) d.history.pop();
    },

    // ---- "Edit points" ----

    async movePoint(grab, x, y) {
      const d = this.open.draw;
      if (!grab.begun) {
        grab.begun = true;
        grab.points = d.points.map((s) => s.slice());
        this.held = grab;
      }
      grab.wanted = [x, y];
      if (grab.asking) return;
      grab.asking = true;
      while (grab.wanted) {
        const [wx, wy] = grab.wanted;
        grab.wanted = null;
        let hits;
        try {
          hits = await this.drawHits([this.rayAt(wx, wy)]);
        } catch (error) {
          break;
        }
        if (hits[0] && this.open && this.open.draw === d) {
          d.points[grab.index] = hits[0];
          this.stage.frame();
        }
      }
      grab.asking = false;
    },

    async dropPoint(grab, done) {
      const d = this.open.draw;
      this.held = null;
      if (!grab.begun) return;
      while (grab.asking) await new Promise((resolve) => setTimeout(resolve, 10));
      if (!done) { d.points = grab.points; this.stage.frame(); return; }
      this.pushDraw();
      await this.refreshDraw('points', { points: d.points });
    },

    // Shift with a click on a span: a point added there, put where the
    // pointer meets the model ("Insert cut line point").
    async insertPoint(seg, x, y) {
      const d = this.open.draw;
      const pts = d.points;
      const n = pts.length;
      const [a, b] = [pts[seg], pts[(seg + 1) % n]];
      let hit = null;
      try {
        hit = (await this.drawHits([this.rayAt(x, y)]))[0];
      } catch (error) {
        return;
      }
      const mid = hit || a.map((v, k) => (k < 6 ? (v + b[k]) / 2 : v));
      this.pushDraw();
      const at = (seg + 1) % n === 0 ? n : seg + 1;
      d.points.splice(at, 0, mid);
      await this.refreshDraw('points', { points: d.points });
    },

    // The right button on a handle takes it away ("Delete cut line
    // point"), while more than the least are left.
    drawRightClick(x, y) {
      const d = this.open.draw;
      if (!d || !d.editing) return false;
      const at = this.pointAt(x, y);
      if (at < 0 || d.points.length <= MIN_POINTS) return at >= 0;
      this.pushDraw();
      d.points.splice(at, 1);
      d.hoverPoint = -1;
      this.refreshDraw('points', { points: d.points });
      return true;
    },

    // ---- the panel's buttons ----

    // "Close loop": the ends joined across the surface as it is seen
    // (`close_draw_chain_on_surface`), or as they are when near.
    async closeLoop() {
      const o = this.open;
      const d = o.draw;
      const c = d.chain;
      if (c.closed || c.samples.length < MIN_CHAIN) return;
      const [a, b] = [c.samples[c.samples.length - 1], c.samples[0]];
      const gap = Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2]);
      let path = [];
      if (gap > (d.snap || 1)) {
        const frame = this.stage.matrices();
        const sa = this.editor.toView(frame, this.planeToScene(a));
        const sb = this.editor.toView(frame, this.planeToScene(b));
        const n = Math.min(400, Math.max(8, Math.floor(Math.hypot(sb[0] - sa[0], sb[1] - sa[1]) / 2)));
        const rays = [];
        for (let k = 1; k < n; k += 1) rays.push(this.rayAt(sa[0] + ((sb[0] - sa[0]) * k) / n, sa[1] + ((sb[1] - sa[1]) * k) / n));
        let hits;
        try {
          hits = await this.drawHits(rays);
        } catch (error) {
          return;
        }
        let misses = 0;
        for (const hit of hits) {
          if (!hit) { misses += 1; continue; }
          const last = path[path.length - 1];
          if (last && Math.hypot(hit[0] - last[0], hit[1] - last[1], hit[2] - last[2]) < 1e-6) continue;
          path.push(hit);
        }
        if (path.length < 2 || misses * 4 > n) { d.reject = 'nopath'; this.panel(); return; }
      }
      this.pushDraw();
      const answer = await this.refreshDraw('close', { path });
      if (answer && !answer.done) { d.history.pop(); d.reject = 'nopath'; this.panel(); } else d.reject = null;
    },

    async finishOpen() {
      this.pushDraw();
      this.open.draw.reject = null;
      const answer = await this.refreshDraw('finish_open');
      if (answer && !answer.done) this.open.draw.history.pop();
    },

    clearLine() {
      const d = this.open.draw;
      this.pushDraw();
      d.chain = emptyChain();
      d.editing = false;
      d.reject = null;
      this.refreshDraw(null);
    },

    // "Mirror cut": the plane's y turns over, so the line's points and
    // normals and the latched view do (CutRecipe.cpp `cut_mirror_local`).
    drawMirrored() {
      const o = this.open;
      if (!o.draw) return;
      const flip = (s) => s.map((v, k) => (k === 1 || k === 4 ? -v : v));
      o.draw.chain.samples = o.draw.chain.samples.map(flip);
      o.draw.history = o.draw.history.map((c) => ({ ...c, samples: c.samples.map(flip) }));
      o.draw.future = o.draw.future.map((c) => ({ ...c, samples: c.samples.map(flip) }));
      this.drawUi.view = [this.drawUi.view[0], -this.drawUi.view[1], this.drawUi.view[2]];
      o.draw.planeKey = null;
    },

    // Esc: out of "Edit points", or the line cleared.
    drawEscape() {
      const d = this.open.draw;
      if (!d) return false;
      if (d.editing) { d.editing = false; d.hoverPoint = -1; this.panel(); this.stage.frame(); return true; }
      if (!d.chain.samples.length) return false;
      this.clearLine();
      return true;
    },

    // ---- drawing ----

    dropDrawSurface() {
      const d = this.open && this.open.draw;
      if (d && d.surfaceGl) this.stage.renderer.drop(d.surfaceGl);
      if (d) { d.surfaceGl = null; d.surfaceOf = null; }
    },

    // The line, its ends, its handles and the surface it makes; the
    // halves once they are worked out (`render_draw_stroke`).
    drawDrawn(frame) {
      const o = this.open;
      const d = o.draw;
      if (!d) return;
      const r = this.stage.renderer;
      const gl = r.gl;
      const view = M.multiply(frame.projection, frame.view);
      const plane = M.chain(view, this.frame());
      this.drawHalves(frame);
      // The surface, a quarter there, so the part shows through it.
      const a = d.answer;
      if (a && a.valid && a.surface.length) {
        if (d.surfaceOf !== a) {
          this.dropDrawSurface();
          d.surfaceGl = r.triangles(a.surface);
          d.surfaceOf = a;
        }
        const colour = this.canPerform() || this.editor.busy ? [0.9, 0.9, 0.9, 0.25] : [1, 0.8, 0.8, 0.25];
        r.blend(true);
        gl.disable(gl.CULL_FACE);
        r.drawFlat(d.surfaceGl, plane, colour);
        r.blend(false);
      }
      // The ribbon: the line being drawn, else the finished one, else the
      // chain as it stands.
      let pts;
      let closed = false;
      if (d.editing && d.points) { pts = d.points; closed = Boolean(a && a.closed); } else if (d.capture) {
        const live = d.capture.at === 'front' ? [...d.capture.samples].reverse() : d.capture.samples;
        pts = d.capture.at === 'front' ? [...live, ...d.chain.samples] : [...d.chain.samples, ...live];
      } else if (a && a.valid) { pts = a.path; closed = a.closed; } else pts = d.chain.samples;
      gl.disable(gl.DEPTH_TEST);
      if (pts.length >= 2) {
        const ends = [];
        const spans = closed ? pts.length : pts.length - 1;
        for (let i = 0; i < spans; i += 1) {
          const [p, q] = [pts[i], pts[(i + 1) % pts.length]];
          ends.push(p[0], p[1], p[2], q[0], q[1], q[2]);
        }
        const lines = r.lines(ends);
        r.drawLines(lines, plane, RIBBON, 3);
        r.drop(lines);
      }
      gl.enable(gl.DEPTH_TEST);
      gl.clear(gl.DEPTH_BUFFER_BIT);
      const shapes = this.parts();
      const size = 0.5 * this.grip().size;
      // The open line's ends (`render_draw_chain_endpoints`).
      const c = d.chain;
      if (c.samples.length && !c.closed && !d.editing) {
        const cap = d.capture;
        const sphere = (s, end) => {
          const armed = cap && cap.armed && cap.at !== end;
          const hot = !cap && d.hoverEnd === end;
          const k = armed ? 1.4 : hot ? 1.25 : 1;
          r.drawMesh(shapes.sphere, frame, M.chain(M.translation(...this.planeToScene(s)), M.scaling(k * size, k * size, k * size)), { tint: armed ? END_ARMED : hot ? END_HOT : END });
        };
        sphere(c.samples[0], 'front');
        sphere(c.samples[c.samples.length - 1], 'back');
      }
      // The handles while editing (`render_draw_point_handles`).
      if (d.editing && d.points) {
        const k = 0.6 * size;
        d.points.forEach((s, i) => {
          const lit = d.hoverPoint === i || (this.held && this.held.part === 'draw-point' && this.held.index === i);
          r.drawMesh(shapes.sphere, frame, M.chain(M.translation(...this.planeToScene(s)), M.scaling(k, k, k)), { tint: lit ? HANDLE_HOT : HANDLE });
        });
      }
    },

    // ---- the panel ----

    drawRows(busy) {
      const { el } = this.host;
      const o = this.open;
      const d = o.draw;
      const u = this.drawUi;
      const c = d.chain;
      const a = d.answer;
      const rows = [];
      const redo = () => this.refreshDraw(null);
      const row = (label, ...cells) => el('div', { className: 'cut-row' }, el('span', { className: 'edit-label', textContent: label }), ...cells);
      // A slider and its number, as "Thickness" has them.
      const slider = (label, value, min, max, step, unitWord, tip, act, off, fieldMax = max) => {
        const digits = step < 0.1 ? 2 : step < 1 ? 1 : 0;
        const range = el('input', { type: 'range', min: String(min), max: String(max), step: String(step), value: String(Math.min(max, value)), title: tip, disabled: busy || off });
        range.setAttribute('aria-label', label);
        const number = el('input', { type: 'number', min: String(min), max: String(fieldMax), step: String(step), value: value.toFixed(digits), title: tip, disabled: busy || off });
        number.setAttribute('aria-label', label);
        const set = (v) => { if (Number.isFinite(v)) act(Math.min(fieldMax, Math.max(min, v))); else this.panel(); };
        range.addEventListener('input', () => { number.value = Number(range.value).toFixed(digits); });
        range.addEventListener('change', () => set(Number(range.value)));
        number.addEventListener('change', () => set(Number(number.value)));
        return el('span', { className: 'cut-slide' }, range, el('span', { className: 'unit' }, number, unitWord ? ` ${unitWord}` : ''));
      };
      // "Direction".
      const direction = el('select', { disabled: busy, title: t('Which way the cut surface reaches into the part. Surface normal follows the face the line was drawn on; the others are a single direction, i.e. a straight extrusion of the line.') });
      direction.setAttribute('aria-label', t('Direction'));
      [t('Surface normal'), t('View'), t('Axis X'), t('Axis Y'), t('Axis Z')].forEach((word, k) => direction.append(el('option', { value: String(k), textContent: word, selected: u.direction === k })));
      direction.addEventListener('change', () => {
        u.direction = Number(direction.value);
        if (u.direction === 1) this.latchView();
        redo();
      });
      rows.push(row(t('Direction'), el('span', { className: 'cut-line' }, direction)));
      const mm = t('mm');
      rows.push(row(t('Extension'), slider(t('Extension'), u.extension, 0, 50, 0.1, mm, t('How far the cut surface reaches past the line. An open line needs enough of this to reach across the part, or the cut will not separate it.'), (v) => { u.extension = v; redo(); })));
      // "Extension angle": off, the skirt carries on the band's slope.
      const extOn = el('input', { type: 'checkbox', checked: u.extAngleOn, disabled: busy });
      extOn.addEventListener('change', () => {
        u.extAngleOn = extOn.checked;
        if (u.extAngleOn && u.extAngle === 0) u.extAngle = u.angle;
        redo();
      });
      rows.push(row(t('Extension angle'), el('label', { className: 'edit-check', title: t("Off, the skirt outside the line continues the band's own slope, which is the default and keeps the cut surface smooth where it crosses the skin. Tick this to aim the skirt somewhere else instead.") }, extOn, ` ${t('Set')}`)));
      rows.push(row('', slider(t('Extension angle'), u.extAngle, -90, 90, 1, '°', t("Which way the skirt outside the drawn line leaves it. 0 lays it flat, level with the middle of the cut; higher lifts it away from the part; negative tucks it back under. With this off it continues the band, which is what the Angle sets."), (v) => { u.extAngle = v; if (u.extAngleOn) redo(); else this.panel(); }, !u.extAngleOn)));
      // "Angle": a loop's, or a line's along the surface normal.
      const angleUsable = c.closed || u.direction === 0;
      rows.push(row(t('Angle'), slider(t('Angle'), u.angle, -90, 90, 1, '°', t('The angle at which the outer shell projects in towards the flat middle of the cut. 0 makes a flat shelf around the core, 45 a chamfered lip the two halves key into, and 90 a straight wall with no lip. The SIGN is which way the lip leans: positive projects it away from the face you drew on, negative back towards you, so the same line can be given a lip on either side.'), (v) => { u.angle = v; redo(); }, !angleUsable)));
      rows.push(row(t('Depth'), slider(t('Depth'), u.depth, DEPTH_MIN, DEPTH_SLIDER, 0.1, mm, t('How far in the cut travels from the drawn line before it turns onto the flat middle. The deeper it goes, the smaller the flat core - too deep and there is no flat left at all.'), (v) => { u.depth = v; redo(); }, false, DEPTH_MAX)));
      rows.push(row(t('Smoothing'), slider(t('Smoothing'), u.smoothing, 0, 1, 0.01, '', t('Averages the line and its surface normals. A hand-drawn line picks up every facet it crossed; a little of this takes the kinks out of the cut surface.'), (v) => { u.smoothing = v; redo(); })));
      // "Edit points", "Close loop", "Cut along the line", "Clear line".
      const valid = Boolean(a && a.valid);
      const edit = el('input', { type: 'checkbox', checked: d.editing, disabled: busy || !valid });
      edit.addEventListener('change', () => {
        d.editing = edit.checked && valid;
        d.hoverPoint = -1;
        if (d.editing) d.points = a.path.map((s) => s.slice());
        this.panel();
        this.stage.frame();
      });
      rows.push(el('label', { className: 'edit-check', title: t("Turn the line's points into handles you can move. Drag a handle to move it along the surface, right-click one to delete it, and Shift+click a segment to add one. Each edit is a separate undo step.") }, edit, ` ${t('Edit points')}`));
      const button = (word, tip, off, act) => {
        const b = el('button', { type: 'button', textContent: word, title: tip, disabled: busy || off });
        b.addEventListener('click', act);
        return b;
      };
      rows.push(el('div', { className: 'cut-foot cut-start' },
        button(t('Close loop'), t('Join the two ends of the line along the surface, so the cut surface can be made'), c.closed || c.samples.length < MIN_CHAIN, () => this.closeLoop()),
        button(t('Cut along the line'), t('Use the line as it is, without closing it: the cut splits the part along it'), c.closed || c.finished_open || c.samples.length < MIN_CHAIN, () => this.finishOpen()),
        button(t('Clear line'), t('Remove the drawn line'), !c.samples.length, () => this.clearLine())));
      // What to do next.
      const hint = (text) => rows.push(el('p', { className: 'edit-hint', textContent: text }));
      const warn = (text) => rows.push(el('p', { className: 'edit-hint cut-warning', role: 'status', textContent: text }));
      if (d.editing) hint(t('Drag a handle to move it; right-click one to delete it; Shift+click a segment to add one.'));
      else if (!c.samples.length) hint(t('Drag on the model to start the cut line. You can rotate the view and carry on from either end until the line closes on itself.'));
      else if (c.finished_open) hint(t('Using the line as it is. Draw from an end to carry on; Esc clears it; Ctrl+Z takes back one stroke and Ctrl+Y puts it back.'));
      else if (!c.closed) hint(t('Start your next stroke at one of the marked ends. Rotate the view freely. Finish a stroke near the other end and the loop closes - or press "Cut along the line" to use it open.'));
      else hint(t('Closed. Esc clears the line; Ctrl+Z takes back one stroke and Ctrl+Y puts it back.'));
      if (d.reject === 'disjoint') warn(t('That stroke did not start at either end of the line, so it was not added. Start at one of the marked ends, or clear the line first.'));
      else if (d.reject === 'closed') warn(t('The line is already closed. Clear it, or press Ctrl+Z, before drawing a different one.'));
      else if (d.reject === 'nopath') warn(t('The two ends cannot be joined across the surface from this view. Turn the model so the gap between them is visible and try again, or draw the remaining part of the line by hand.'));
      if (!c.samples.length) warn(t('Draw a line on the model.'));
      else if (!c.closed && !c.finished_open) {
        warn(t('The line is not finished, so there is no cut surface yet. Carry on from an end until it comes within %1% mm of the other, or use it open.').replace('%1%', String(Math.round(a ? a.snap : 1))));
      } else if (a && !a.valid) {
        if (a.error) warn(a.error);
      } else if (a) {
        hint(a.closed ? t('Closed loop: the cut takes out the piece the line goes around.') : t('Open line: the cut splits the part along the line.'));
        if (a.empty[0] || a.empty[1]) warn(t('The stroke does not separate the part - increase Extension or draw the line right across it.'));
        if (a.folds) {
          warn(c.closed
            ? t('Depth reaches almost to the middle of the line, so there is no flat left to mate on. Reduce Depth, or raise the Angle.')
            : u.angle !== 0
              ? t('The line turns tighter than the cut surface reaches sideways, so the surface folds there. Reduce the Angle, the Extension or the Depth.')
              : t('The line turns tighter than the Extension reaches, so the cut surface folds there. Reduce Extension.'));
        }
        if (a.clipped) warn(t("The Extension reaches other parts of the model here, so it has been clipped to keep the cut on the line you drew. Shorten the line's gap to those parts or reduce Extension."));
        if (a.flips) warn(t('This loop has no consistent outside, so the Angle has been ignored. Draw a simpler loop, or use Direction = Axis or View.'));
      }
      if (o.previewStale) rows.push(el('p', { className: 'edit-hint cut-note', role: 'status', textContent: t('Cut face updating…') }));
      if (o.previewError) warn(o.previewError);
      return rows;
    },
  });
})(window.EdgeGL);
