'use strict';

// "Cut"'s surfaces (Gizmos/GLGizmoCut.cpp `render_curved_surface_inputs`,
// `render_side_visibility_inputs`, `curved_on_mouse`): "Surface" flat or
// "Curved", the curved one a grid of control points over the plane bent by
// dragging them (their neighbours following within "Bend radius", less the
// further they are with "Falloff"), smoothed, reset, snapped onto the part
// by the right button; and each half shown, ghosted or hidden. While the
// surface is bent the session works the halves out by its booleans
// (`cut_preview`), the last one asked for stopping the one before, and
// they are drawn in place of the part. The surface's sums are the
// reference's CurvedCut.cpp, as `edge_engine::curved` has them. Added to
// `GL.cutter` (cut.js).
(function (GL) {
  const M = GL.math;
  const cutter = GL.cutter;
  const sub = M.sub;

  const MIN = 2;
  const MAX = 15;
  // `curved_cut_default_grid`: a point every 10 mm, at least five a side.
  const SPACING = 10;
  const LEAST = 5;
  // The handles' colours: the grip's grey, yellow under the pointer.
  const HANDLE = [0.75, 0.75, 0.75, 1];
  const HANDLE_HOT = [1, 1, 0, 1];
  // `curved_cut_side_alpha`: a ghost a quarter there.
  const GHOST = 0.25;
  const UPPER = [0, 1, 1];
  const LOWER = [1, 0, 1];

  const catmullRom = (p0, p1, p2, p3, t) => {
    const t2 = t * t;
    const t3 = t2 * t;
    return 0.5 * ((2 * p1) + (-p0 + p2) * t + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t2 + (-p0 + 3 * p1 - 3 * p2 + p3) * t3);
  };

  // The sheet's sums (CurvedCut.cpp `CurvedCutSheet`).
  const Sheet = {
    flat(nx, ny, halfU, halfV) {
      return { nx, ny, half_u: halfU, half_v: halfV, z: new Array(nx * ny).fill(0) };
    },
    isFlat(s) { return s.z.every((z) => z === 0); },
    at(s, i, j) { return s.z[Math.min(Math.max(j, 0), s.ny - 1) * s.nx + Math.min(Math.max(i, 0), s.nx - 1)]; },
    evaluate(s, u, v) {
      if (s.nx < 2 || s.ny < 2 || Sheet.isFlat(s)) return 0;
      const fu = Math.min(Math.max(u, 0), 1) * (s.nx - 1);
      const fv = Math.min(Math.max(v, 0), 1) * (s.ny - 1);
      const iu = Math.min(Math.max(Math.floor(fu), 0), s.nx - 2);
      const iv = Math.min(Math.max(Math.floor(fv), 0), s.ny - 2);
      const [tu, tv] = [fu - iu, fv - iv];
      const blend = (p, t, count) => (count === 2 ? p[1] + (p[2] - p[1]) * t : catmullRom(p[0], p[1], p[2], p[3], t));
      const column = [0, 1, 2, 3].map((k) => {
        const j = iv - 1 + k;
        return blend([Sheet.at(s, iu - 1, j), Sheet.at(s, iu, j), Sheet.at(s, iu + 1, j), Sheet.at(s, iu + 2, j)], tu, s.nx);
      });
      return blend(column, tv, s.ny);
    },
    height(s, x, y) { return Sheet.evaluate(s, 0.5 * (x / s.half_u + 1), 0.5 * (y / s.half_v + 1)); },
    controlXY(s, i, j) {
      return [(2 * (s.nx < 2 ? 0.5 : i / (s.nx - 1)) - 1) * s.half_u, (2 * (s.ny < 2 ? 0.5 : j / (s.ny - 1)) - 1) * s.half_v];
    },
    // `falloff_weight`.
    weight(d, r) {
      if (r <= 0 || d >= r) return 0;
      if (d <= 0) return 1;
      const t = 1 - (d * d) / (r * r);
      return t * t;
    },
    // `grab`: the points within `radius` of `centre` moved by `delta`.
    grab(s, centre, radius, delta, falloff) {
      if (radius <= 0 || delta === 0) return;
      for (let j = 0; j < s.ny; j += 1) {
        for (let i = 0; i < s.nx; i += 1) {
          const [x, y] = Sheet.controlXY(s, i, j);
          const d = Math.hypot(x - centre[0], y - centre[1]);
          if (d >= radius) continue;
          s.z[j * s.nx + i] += delta * (falloff ? Sheet.weight(d, radius) : 1);
        }
      }
    },
    // `smooth`: each point towards its neighbours' mean.
    smooth(s, strength) {
      if ((s.nx < 3 && s.ny < 3) || strength <= 0) return;
      const src = s.z.slice();
      const at = (i, j) => src[Math.min(Math.max(j, 0), s.ny - 1) * s.nx + Math.min(Math.max(i, 0), s.nx - 1)];
      const [useU, useV] = [s.nx >= 3, s.ny >= 3];
      for (let j = 0; j < s.ny; j += 1) {
        for (let i = 0; i < s.nx; i += 1) {
          const sum = (useU ? at(i - 1, j) + at(i + 1, j) : 0) + (useV ? at(i, j - 1) + at(i, j + 1) : 0);
          const mean = sum / (2 * (Number(useU) + Number(useV)));
          s.z[j * s.nx + i] = src[j * s.nx + i] * (1 - strength) + mean * strength;
        }
      }
    },
    // `set_grid`: the surface sampled at a grid of another size.
    regrid(s, nx, ny) {
      nx = Math.min(Math.max(nx, MIN), MAX);
      ny = Math.min(Math.max(ny, MIN), MAX);
      const out = Sheet.flat(nx, ny, s.half_u, s.half_v);
      if (!Sheet.isFlat(s)) {
        for (let j = 0; j < ny; j += 1) for (let i = 0; i < nx; i += 1) out.z[j * nx + i] = Sheet.evaluate(s, i / (nx - 1), j / (ny - 1));
      }
      return out;
    },
    // `set_half_size` with `resample`: the same surface over another
    // extent.
    resized(s, halfU, halfV) {
      const out = { ...s, half_u: Math.max(halfU, 1e-6), half_v: Math.max(halfV, 1e-6), z: s.z.slice() };
      if (Sheet.isFlat(s)) return out;
      for (let j = 0; j < s.ny; j += 1) {
        for (let i = 0; i < s.nx; i += 1) {
          const [x, y] = Sheet.controlXY(out, i, j);
          out.z[j * s.nx + i] = Sheet.height(s, x, y);
        }
      }
      return out;
    },
    // The plane flipped about its x: rows reversed, heights turned
    // (`flip_about_u`); mirrored: rows reversed, heights kept.
    flipped(s, turnHeights) {
      const out = { ...s, z: s.z.slice() };
      for (let j = 0; j < s.ny; j += 1) for (let i = 0; i < s.nx; i += 1) out.z[j * s.nx + i] = (turnHeights ? -1 : 1) * s.z[(s.ny - 1 - j) * s.nx + i];
      return out;
    },
  };

  Object.assign(cutter, {
    // "Show halves": each `visible`, `ghost` or `hidden`, kept from one cut
    // to the next.
    visibility: { upper: 'visible', lower: 'visible' },
    // The curved surface's own choices, kept from one cut to the next.
    curvedUi: { square: true, falloff: true, userGrid: false },

    // Whether the surface is curved and bent: what the cut and the
    // preview are made by.
    bent() {
      const o = this.open;
      return Boolean(o && !this.dovetail() && o.surface === 'curved' && o.sheet && !Sheet.isFlat(o.sheet));
    },

    curvedArgs() {
      return this.bent() ? { sheet: this.open.sheet } : {};
    },

    // "Surface": flat or curved.
    setSurface(surface) {
      const o = this.open;
      if (!o || surface === o.surface) return;
      this.remember();
      o.surface = surface;
      if (surface === 'curved') {
        o.sheet = null;
        this.fitSheet(true);
        o.fitKey = `${this.frame().join(',')}|`;
      }
      this.dropPreview();
      this.changed();
    },

    // The sheet over the part's section, a little beyond it
    // (`fit_curved_sheet_to_section`, `curved_cut_fit_projection_extent`):
    // its control points every 10 mm unless the grid was set by hand; a
    // bent sheet keeps its shape over the new extent.
    fitSheet(fresh) {
      const o = this.open;
      const reach = o.section && o.section.reach;
      const radius = o.radius;
      const extent = (low, high) => {
        const far = reach ? Math.max(Math.abs(low), Math.abs(high)) : radius;
        return Math.ceil((far + Math.max(0.15 * far, 5)) * 2 - 1e-9) * 0.5;
      };
      const halfU = extent(reach ? reach[0][0] : 0, reach ? reach[1][0] : 0);
      const halfV = extent(reach ? reach[0][1] : 0, reach ? reach[1][1] : 0);
      const count = (half) => Math.min(Math.max(Math.round((2 * half) / SPACING) + 1, LEAST), MAX);
      if (fresh || !o.sheet) {
        o.sheet = Sheet.flat(count(halfU), count(halfV), halfU, halfV);
        this.curvedUi.userGrid = false;
      } else {
        let sheet = Sheet.resized(o.sheet, halfU, halfV);
        if (!this.curvedUi.userGrid) sheet = Sheet.regrid(sheet, count(halfU), count(halfV));
        o.sheet = sheet;
      }
      o.bendRadius = this.defaultBend();
    },

    // `default_curved_bend_radius`: one and a half times the grid's
    // spacing.
    defaultBend() {
      const s = this.open.sheet;
      const spacing = Math.min((2 * s.half_u) / Math.max(1, s.nx - 1), (2 * s.half_v) / Math.max(1, s.ny - 1));
      return Math.max(1, 1.5 * spacing);
    },

    // A control point's place in the scene.
    controlAt(i, j) {
      const o = this.open;
      const s = o.sheet;
      const [x, y] = Sheet.controlXY(s, i, j);
      return M.add(M.add(M.along(o.middle, o.axes[0], x), M.scale(o.axes[1], y)), M.scale(o.axes[2], s.z[j * s.nx + i]));
    },

    // The control point under a pixel.
    controlUnder(x, y) {
      const o = this.open;
      if (!o || o.surface !== 'curved' || !o.sheet || this.dovetail() || o.editing) return -1;
      const frame = this.stage.matrices();
      const px = 0.6 * GL.cutterGripPx;
      let best = -1;
      let near = Infinity;
      for (let j = 0; j < o.sheet.ny; j += 1) {
        for (let i = 0; i < o.sheet.nx; i += 1) {
          const p = this.editor.toView(frame, this.controlAt(i, j));
          const far = Math.hypot(x - p[0], y - p[1]);
          if (far < px && far < near) { best = j * o.sheet.nx + i; near = far; }
        }
      }
      return best;
    },

    // A press on a handle: a drag bends the surface along the plane's
    // normal ("Curved cut surface").
    curvedGrab(x, y) {
      const at = this.controlUnder(x, y);
      if (at < 0) return null;
      return { kind: 'cut', part: 'control', index: at, x, y, id: `cut-control-${at}`, handle: { id: `cut-control-${at}` }, tap: () => {} };
    },

    curvedDrag(grab, x, y) {
      const o = this.open;
      const s = o.sheet;
      const [i, j] = [grab.index % s.nx, Math.floor(grab.index / s.nx)];
      if (!grab.begun) {
        grab.begun = true;
        this.remember();
        grab.grid = s.z.slice();
        grab.origin = this.controlAt(i, j);
        grab.start = this.editor.along(grab.origin, o.axes[2], grab.x, grab.y);
        this.held = grab;
      }
      const delta = this.editor.along(grab.origin, o.axes[2], x, y) - grab.start;
      s.z = grab.grid.slice();
      Sheet.grab(s, Sheet.controlXY(s, i, j), o.bendRadius, delta, this.curvedUi.falloff);
      this.sheetChanged(true);
    },

    curvedDrop(grab, done) {
      const o = this.open;
      this.held = null;
      if (grab.begun && !done) { o.sheet.z = grab.grid; o.history.pop(); }
      this.sheetChanged(false);
    },

    // The right button on a handle snaps it onto the part along the
    // plane's normal; with Shift its neighbours follow
    // (`curved_snap_apply`).
    async curvedSnap(x, y, event) {
      const o = this.open;
      const at = this.controlUnder(x, y);
      if (at < 0) return false;
      const s = o.sheet;
      const [i, j] = [at % s.nx, Math.floor(at / s.nx)];
      const [cx, cy] = Sheet.controlXY(s, i, j);
      const project = this.host.project();
      let answer;
      try {
        answer = await this.host.call('cut_snap', { project: project.id, copy: o.copy, plane: this.frame(), point: [cx, cy, s.z[at]] });
      } catch (error) {
        this.host.say(error.message, true);
        return true;
      }
      if (this.open !== o || answer.distance === null) return true;
      this.remember();
      if (event && event.shiftKey) Sheet.grab(o.sheet, [cx, cy], o.bendRadius, answer.distance, this.curvedUi.falloff);
      else o.sheet.z[at] += answer.distance;
      this.sheetChanged(false);
      return true;
    },

    // The surface changed: drawn again, and its halves worked out anew
    // once it is still for a moment.
    sheetChanged(dragging) {
      const o = this.open;
      o.sheetMesh = null;
      clearTimeout(this.previewTimer);
      this.previewTimer = setTimeout(() => this.askPreview(), dragging ? 250 : 60);
      o.previewStale = this.bent();
      this.panel();
      this.stage.frame();
    },

    // ---- the preview ----

    async askPreview() {
      const o = this.open;
      if (!o) return;
      if (!this.bent()) { this.dropPreview(); o.previewStale = false; this.panel(); this.stage.frame(); return; }
      const project = this.host.project();
      const asked = JSON.stringify([this.frame(), o.sheet, o.thickness, o.from]);
      if (o.preview && o.preview.key === asked) { o.previewStale = false; this.panel(); return; }
      o.previewStale = true;
      this.panel();
      const args = { project: project.id, copy: o.copy, plane: this.frame(), sheet: o.sheet, thickness: o.thickness, thickness_from: o.from };
      let answer;
      try {
        answer = await this.host.call('cut_preview', args);
      } catch (error) {
        // A later one took its place: nothing to say.
        if (this.open === o && !/stopped/.test(error.message)) {
          o.previewError = error.message;
          o.previewStale = false;
          this.panel();
        }
        return;
      }
      if (this.open !== o || JSON.stringify([this.frame(), o.sheet, o.thickness, o.from]) !== asked) return;
      let upper;
      let lower;
      try {
        upper = answer.triangles[0] ? await this.stage.bytes('cut_preview_mesh', { id: answer.id, side: 'upper' }) : null;
        lower = answer.triangles[1] ? await this.stage.bytes('cut_preview_mesh', { id: answer.id, side: 'lower' }) : null;
      } catch (error) {
        return;
      }
      if (this.open !== o) return;
      this.dropPreview();
      const r = this.stage.renderer;
      o.preview = { key: asked, upper: upper && r.meshFromBytes(upper), lower: lower && r.meshFromBytes(lower), empty: [!answer.triangles[0], !answer.triangles[1]] };
      o.previewStale = false;
      o.previewError = null;
      this.changed();
    },

    dropPreview() {
      const o = this.open;
      if (!o || !o.preview) return;
      const r = this.stage.renderer;
      if (o.preview.upper) r.drop(o.preview.upper);
      if (o.preview.lower) r.drop(o.preview.lower);
      o.preview = null;
    },

    // ---- drawing ----

    // The halves' alphas (`curved_cut_side_alpha`): 1, a ghost's, or none.
    sideAlpha(side) {
      const v = this.visibility[side];
      return v === 'hidden' ? -1 : v === 'ghost' ? GHOST : 1;
    },

    // The bent surface, its handles, and the halves as the booleans made
    // them, in place of the part.
    drawCurved(frame, view) {
      const o = this.open;
      const r = this.stage.renderer;
      const gl = r.gl;
      if (o.preview && this.bent()) {
        for (const [side, mesh, colour] of [['upper', o.preview.upper, UPPER], ['lower', o.preview.lower, LOWER]]) {
          const alpha = this.sideAlpha(side);
          if (!mesh || alpha < 0) continue;
          r.blend(alpha < 1);
          r.drawMesh(mesh, frame, M.identity(), { tint: [...colour, 1], alpha });
        }
        r.blend(false);
      }
      if (!o.sheetMesh) {
        // The surface, sampled 32 a side (`sample_sheet`), in the plane's
        // own coordinates.
        const s = o.sheet;
        const n = 32;
        const points = [];
        const point = (i, j) => {
          const x = (2 * (i / (n - 1)) - 1) * s.half_u;
          const y = (2 * (j / (n - 1)) - 1) * s.half_v;
          return [x, y, Sheet.height(s, x, y)];
        };
        for (let j = 0; j + 1 < n; j += 1) {
          for (let i = 0; i + 1 < n; i += 1) {
            const [a, b, c, d] = [point(i, j), point(i + 1, j), point(i + 1, j + 1), point(i, j + 1)];
            points.push(...a, ...b, ...c, ...a, ...c, ...d);
          }
        }
        if (this.sheetGl) r.drop(this.sheetGl);
        this.sheetGl = r.triangles(points);
        o.sheetMesh = true;
      }
      const colour = this.canPerform() || this.editor.busy ? [0.9, 0.9, 0.9, 0.5] : [1, 0.8, 0.8, 0.5];
      r.blend(true);
      gl.disable(gl.CULL_FACE);
      r.drawFlat(this.sheetGl, M.chain(view, this.frame()), colour);
      r.blend(false);
      // The handles, over everything.
      gl.clear(gl.DEPTH_BUFFER_BIT);
      const size = 0.6 * GL.cutterGripPx * this.mm();
      const hot = this.editor.hot;
      const held = this.held && this.held.part === 'control' ? this.held.id : null;
      for (let j = 0; j < o.sheet.ny; j += 1) {
        for (let i = 0; i < o.sheet.nx; i += 1) {
          const id = `cut-control-${j * o.sheet.nx + i}`;
          const lit = hot === id || held === id;
          r.drawMesh(this.parts().sphere, frame, M.chain(M.translation(...this.controlAt(i, j)), M.scaling(size, size, size)), { tint: lit ? HANDLE_HOT : HANDLE });
        }
      }
    },

    // ---- the panel ----

    surfaceRows(busy) {
      const { el } = this.host;
      const o = this.open;
      const rows = [];
      const joined = (label, choices, now, act) => {
        const group = el('div', { className: 'joined segmented' });
        group.setAttribute('role', 'group');
        group.setAttribute('aria-label', label);
        for (const [value, word] of choices) {
          const button = el('button', { type: 'button', textContent: word, disabled: busy });
          button.setAttribute('aria-pressed', String(now === value));
          button.addEventListener('click', () => act(value));
          group.append(button);
        }
        return el('div', { className: 'cut-row' }, el('span', { className: 'edit-label', textContent: label }), el('span', { className: 'cut-line' }, group));
      };
      rows.push(joined(t('Surface'), [['flat', t('Flat')], ['curved', t('Curved')]], o.surface, (value) => this.setSurface(value)));
      // "Show halves".
      rows.push(el('span', { className: 'edit-label cut-group', textContent: t('Show halves') }));
      const sides = [['visible', t('Visible')], ['ghost', t('Ghost')], ['hidden', t('Hidden')]];
      for (const [side, word] of [['upper', t('Upper')], ['lower', t('Lower')]]) {
        rows.push(joined(word, sides, this.visibility[side], (value) => { this.visibility[side] = value; this.changed(); }));
      }
      rows.push(el('p', { className: 'edit-hint', textContent: t('Ghost draws a half translucent so the cut surface and the other half show through; Hidden leaves it out of the preview entirely. Neither changes the cut.') }));
      if (o.surface !== 'curved' || !o.sheet) return rows;
      const s = o.sheet;
      const whole = (label, value, min, max, act, tip) => {
        const input = el('input', { type: 'number', min: String(min), max: String(max), step: '1', value: String(value), disabled: busy, title: tip || '' });
        input.setAttribute('aria-label', label);
        input.addEventListener('change', () => { const v = Math.round(Number(input.value)); if (Number.isFinite(v)) act(Math.min(max, Math.max(min, v))); else this.panel(); });
        return el('span', { className: 'unit cut-count' }, input);
      };
      const grid = (nx, ny) => {
        this.remember();
        this.curvedUi.userGrid = true;
        o.sheet = Sheet.regrid(o.sheet, nx, ny);
        o.bendRadius = this.defaultBend();
        this.sheetChanged(false);
      };
      // "Control points": one count, or across and down.
      const cells = this.curvedUi.square
        ? [whole(t('Control points'), Math.max(s.nx, s.ny, 3), 3, MAX, (n) => grid(n, n)), ` × ${Math.max(s.nx, s.ny, 3)}`]
        : [whole(t('Control points'), s.nx, MIN, MAX, (n) => grid(n, s.ny), t('Control points across the sheet (along X). 2 makes every row a single straight line.')),
          ' × ',
          whole(t('Control points'), s.ny, MIN, MAX, (n) => grid(s.nx, n), t('Control points down the sheet (along Y). 2 makes every column a single straight line, grabbable from either end - a ruled bend.'))];
      rows.push(el('div', { className: 'cut-row' }, el('span', { className: 'edit-label', textContent: t('Control points') }), el('span', { className: 'cut-line' }, ...cells)));
      const tick = (label, on, act, tip) => {
        const input = el('input', { type: 'checkbox', checked: on, disabled: busy });
        input.addEventListener('change', () => act(input.checked));
        return el('label', { className: 'edit-check', title: tip || '' }, input, ` ${label}`);
      };
      rows.push(tick(t('Square'), this.curvedUi.square, (on) => {
        this.curvedUi.square = on;
        if (on && s.nx !== s.ny) grid(Math.max(s.nx, s.ny), Math.max(s.nx, s.ny));
        else this.panel();
      }, t('Keep the same number of control points across and down. Turn it off for a ruled sheet, e.g. 10 x 2.')));
      // "Bend radius" and "Falloff".
      const bend = el('input', { type: 'number', min: '1', max: '200', step: '0.5', value: (Math.round(o.bendRadius * 10) / 10).toFixed(1), disabled: busy, title: t('Neighbouring control points within this distance follow a dragged handle, less the further they are. Small: bend one point; large: bend the whole sheet.') });
      bend.setAttribute('aria-label', t('Bend radius'));
      bend.addEventListener('change', () => { const v = Number(bend.value); if (Number.isFinite(v)) o.bendRadius = Math.min(200, Math.max(1, v)); this.panel(); });
      rows.push(el('div', { className: 'cut-row' }, el('span', { className: 'edit-label', textContent: t('Bend radius') }), el('span', { className: 'cut-line' }, el('span', { className: 'unit' }, bend, ` ${t('mm')}`))));
      rows.push(tick(t('Falloff'), this.curvedUi.falloff, (on) => { this.curvedUi.falloff = on; }));
      // "Smooth" and "Reset surface".
      const smooth = el('button', { type: 'button', textContent: t('Smooth'), title: t('One smoothing pass over the control grid'), disabled: busy });
      smooth.addEventListener('click', () => { this.remember(); Sheet.smooth(o.sheet, 0.5); this.sheetChanged(false); });
      const reset = el('button', { type: 'button', textContent: t('Reset surface'), title: t('Flatten the surface back to the cut plane'), disabled: busy || Sheet.isFlat(s) });
      reset.addEventListener('click', () => { this.remember(); o.sheet.z = o.sheet.z.map(() => 0); this.sheetChanged(false); });
      rows.push(el('div', { className: 'cut-foot cut-start' }, smooth, reset));
      rows.push(el('p', { className: 'edit-hint', textContent: t('Drag a handle to bend the cut surface. With every handle at zero the cut is exactly the flat plane cut.') }));
      rows.push(el('p', { className: 'edit-hint', textContent: t('Right-click-drag a handle to snap it onto the model surface; hold Shift to carry its neighbours with it. The surface it finds may be on the far side of the part - set that half to Ghost or Hidden above to see it.') }));
      if (o.previewStale) rows.push(el('p', { className: 'edit-hint cut-note', role: 'status', textContent: t('Cut face updating…') }));
      if (o.previewError) rows.push(el('p', { className: 'edit-hint cut-warning', role: 'status', textContent: o.previewError }));
      const empty = o.preview && this.bent() ? o.preview.empty : [false, false];
      if (empty[0]) rows.push(el('p', { className: 'edit-hint cut-note', textContent: t('The surface does not cross the part on the upper side; that side would be empty.') }));
      if (empty[1]) rows.push(el('p', { className: 'edit-hint cut-note', textContent: t('The surface does not cross the part on the lower side; that side would be empty.') }));
      return rows;
    },

    sheetFlipped(turnHeights) {
      const o = this.open;
      if (o && o.sheet) { o.sheet = Sheet.flipped(o.sheet, turnHeights); o.sheetMesh = null; }
    },
  });

  GL.CutSheet = Sheet;
})(window.EdgeGL);
