'use strict';

// The painting tools of Prepare: "Support Painting" [L], "Seam painting"
// [P], "Paint-on fuzzy skin" [H] and "Color Painting" [N]
// (Gizmos/GLGizmoPainterBase.cpp, GLGizmoFdmSupports.cpp, GLGizmoSeam.cpp,
// GLGizmoFuzzySkin.cpp, GLGizmoMmuSegmentation.cpp).
//
// The paint is the session's (api/paint.rs): the page sends the lines of
// sight under the pointer (`paint_event`) and draws what comes back, the
// painted pieces of the object's triangles a block at a time, over the
// object's own triangles (`paint_mesh`). Letting go of the button makes
// the stroke one step of the undo history, as the reference takes its
// snapshot on the button's release.
(function (GL) {
  const M = GL.math;

  // GLGizmoPainterBase.hpp: the brush's size and the smart fill's angle,
  // their limits and their steps on the wheel; the height range's.
  const RADIUS = { min: 0.4, max: 8, step: 0.2, start: 1 };
  const HEIGHT = { min: 0.1, max: 8, step: 0.2, start: 0.2 };
  const ANGLE = { min: 0, max: 90, step: 1, start: 30 };
  // TriangleSelectorPatch::GapAreaMin, GapAreaMax, GapAreaStep.
  const GAP = { min: 0, max: 5, step: 0.2, start: 0 };
  // TriangleSelectorGUI::enforcers_color, blockers_color; GLVolume::NEUTRAL_COLOR;
  // the stabilizer points' (GLGizmoFdmSupports.cpp STABILIZER_POINT_COLOR).
  const ENFORCER = [0.5, 1, 0.5, 1];
  const BLOCKER = [1, 0.5, 0.5, 1];
  const STABILIZER = [0.62, 0.36, 0.9, 1];
  const NEUTRAL = [0.8, 0.8, 0.8, 1];
  // GLGizmoPainterBase.hpp: the sphere cursor, held with the left button,
  // the right, or not held.
  const CURSOR = { left: [0, 0, 1, 0.25], right: [1, 0, 0, 0.25], hover: [0, 0, 0, 0.25] };
  // The mark the session gives a piece a fill has marked (paint.rs MARKED).
  const MARKED = 128;

  const ICON = {
    circle: 'M8,1.5a6.5,6.5,0,1,0,0.01,0Z',
    sphere: 'M8,1.5a6.5,6.5,0,1,0,0.01,0ZM1.5,8c0,1.8,2.9,3,6.5,3s6.5-1.2,6.5-3',
    triangle: 'M8,2L14.5,13.5H1.5Z',
    height: 'M1.5,5.5h13M1.5,10.5h13M8,1.5v4M8,10.5v4M6,3.5l2-2l2,2M6,12.5l2,2l2-2',
    fill: 'M3,7.5l5-5l5,5l-5,5ZM13.5,10.5c0,1.5-1,2.5-1,2.5s-1-1-1-2.5l1-1.5Z',
    gap: 'M1.5,1.5h13v13h-13ZM6,6h4v4h-4Z',
  };

  // What each tool offers (each gizmo's `on_render_input_window`), its
  // left and right buttons' states (`get_left_button_state_type`,
  // `get_right_button_state_type`; null: the right button is not the
  // tool's), its keys for the tools (`on_key_down_select_tool_type`), the
  // name of the size, and the undo step of a stroke
  // (`handle_snapshot_action_name`).
  const KINDS = {
    supports: {
      name: t('Support Painting'), size: t('Pen size'), shape: t('Tool type'),
      tools: ['circle', 'sphere', 'fill', 'gap'],
      keys: { c: 'circle', s: 'sphere', f: 'fill', g: 'gap' },
      left: 1, right: 2,
      // GLGizmoFdmSupports.cpp writes these without `_L`.
      step: (erase, right) => (erase ? 'Unselect all' : right ? 'Block supports' : 'Enforce supports'),
      rows: [[t('Left mouse button'), t('Enforce supports')], [t('Right mouse button'), t('Block supports')], [t('Shift+') + t('Left mouse button'), t('Erase')], [t('Ctrl+') + t('Mouse wheel'), t('Pen size')], [t('Alt+') + t('Mouse wheel'), t('Section view')]],
    },
    seam: {
      name: t('Seam painting'), size: t('Brush size'), shape: t('Brush shape'),
      tools: ['circle', 'sphere'],
      keys: { c: 'circle', s: 'sphere' },
      left: 1, right: 2,
      step: (erase, right) => (erase ? 'Remove selection' : right ? 'Block seam' : 'Enforce seam'),
      rows: [[t('Left mouse button'), t('Enforce seam')], [t('Right mouse button'), t('Block seam')], [t('Shift+') + t('Left mouse button'), t('Erase')], [t('Ctrl+') + t('Mouse wheel'), t('Brush size')], [t('Alt+') + t('Mouse wheel'), t('Section view')]],
    },
    fuzzy_skin: {
      name: t('Paint-on fuzzy skin'), size: t('Brush size'), shape: t('Tool type'),
      tools: ['circle', 'sphere', 'triangle', 'fill'],
      keys: {},
      left: 1, right: 0,
      step: (erase) => (erase ? 'Remove fuzzy skin' : 'Add fuzzy skin'),
      rows: [[t('Left mouse button'), t('Add fuzzy skin')], [t('Shift+') + t('Left mouse button'), t('Remove fuzzy skin')], [t('Ctrl+') + t('Mouse wheel'), t('Brush size')], [t('Alt+') + t('Mouse wheel'), t('Section view')]],
    },
    color: {
      name: t('Color Painting'), size: t('Pen size'), shape: t('Tool type'),
      tools: ['circle', 'sphere', 'triangle', 'height', 'fill', 'gap'],
      keys: { c: 'circle', s: 'sphere', t: 'triangle', h: 'height', f: 'fill', g: 'gap' },
      left: null, right: null,
      // The filament's number in the step's name: the reference formats
      // it into the message (`handle_snapshot_action_name`).
      step: (erase, right, filament) => (erase ? 'Remove painted color' : fmt('Painted using: Filament %1%', filament)),
      rows: [[t('Left mouse button'), t('Paint')], [t('Shift+') + t('Left mouse button'), t('Erase')], [t('Key 1~9'), t('Choose filament')], [t('Ctrl+') + t('Mouse wheel'), t('Pen size')], [t('Alt+') + t('Mouse wheel'), t('Section view')]],
    },
  };
  // The tools' own names (their tooltips in the reference).
  const TOOL_NAMES = { circle: t('Circle'), sphere: t('Sphere'), triangle: t('Triangle'), height: t('Height Range'), fill: t('Fill'), gap: t('Gap Fill') };

  const paint = {
    host: null,
    stage: null,
    // The tool open: { kind, project, copy, object (its index in the
    // scene), parts, base, blocks, … }, or null.
    open: null,
    // What each kind's panel was left at, for next time.
    kept: {},
    // A call on its way, and the next pointer event waiting for it.
    busy: false,
    // The stroke going on: its state, its name, where the pointer last was.
    stroke: null,
    // Where the pointer is, and what it is over.
    pointer: null,
    hit: null,
    band: null,

    init(host) {
      this.host = host;
      this.stage = host.stage;
      GL.paint = this;
    },

    kinds() { return KINDS; },

    active() { return Boolean(this.open); },

    settings(kind) {
      if (!this.kept[kind]) {
        this.kept[kind] = {
          tool: KINDS[kind].tools[0], radius: RADIUS.start, height: HEIGHT.start, angle: ANGLE.start, gap: GAP.start,
          edge: true, overhangs: false, threshold: 30, highlight: true, vertical: false, horizontal: false, filament: 1, clip: 0, clipNormal: null,
        };
      }
      return this.kept[kind];
    },

    // ---- opening and closing ----

    // Opens the tool `kind` on a copy of an object.
    async start(kind, copy) {
      await this.finish();
      const project = this.host.project();
      const data = this.host.data();
      if (!project || !data) return false;
      const placing = data.placings.find((one) => one.id === copy);
      if (!placing) return false;
      const filaments = this.host.filamentCount();
      const open = { kind, project: project.id, copy, object: placing.object, objectId: data.objects[placing.object].id, parts: [], base: null, blocks: new Map(), palette: null, ready: false, states: [] };
      this.open = open;
      this.hit = null;
      this.band = null;
      this.panel();
      try {
        const opened = await this.host.call('paint_open', { project: project.id, copy, kind, filaments });
        if (this.open !== open) return false;
        open.parts = opened.parts;
        open.max = opened.max_state;
        this.host.say(`${KINDS[kind].name}: ${Math.round(opened.ms)} ms`, false);
        const bytes = await this.host.bytes('paint_mesh', { project: project.id });
        if (this.open !== open) return false;
        const count = new DataView(bytes).getUint32(0, true);
        open.base = this.stage.renderer.mesh(bytes.slice(8), count * 3);
        open.palette = this.stage.renderer.palette(1);
        this.colours();
        open.ready = true;
        open.revision = project.edit ? project.edit.revision : null;
        this.stage.scene.painting = { object: placing.object, copy };
        await this.send({ op: 'refresh' });
      } catch (error) {
        this.host.say(error.message, true);
        if (this.open === open) this.close();
        return false;
      }
      this.panel();
      this.stage.frame();
      return true;
    },

    // Closes the tool open, if any.
    async finish() {
      if (!this.open) return;
      this.close();
    },

    close() {
      const open = this.open;
      if (!open) return;
      this.open = null;
      this.stroke = null;
      this.waiting = [];
      const r = this.stage.renderer;
      r.drop(open.base);
      for (const block of open.blocks.values()) r.drop(block);
      if (open.palette) r.drop(open.palette);
      this.dropShapes();
      this.stage.scene.painting = null;
      this.cursorNode().hidden = true;
      if (this.panelNode) this.panelNode.hidden = true;
      this.host.call('paint_close', {}).catch(() => {});
      // The object's mesh as it is now painted, which was not asked for while
      // the tool was open.
      this.stage.wantMissing();
      this.stage.frame();
    },

    // The tool goes when what it is on goes or is no longer what is
    // selected alone.
    keep() {
      const open = this.open;
      if (!open) return;
      const project = this.host.project();
      const data = this.host.data();
      const placing = data && data.placings.find((one) => one.id === open.copy);
      if (!project || project.id !== open.project || !placing || this.host.tab() !== 'prepare') {
        this.host.toolClosed();
        this.close();
        return;
      }
      open.object = placing.object;
      if (this.stage.scene.painting) this.stage.scene.painting.object = placing.object;
      // The model changed otherwise (an undo, a redo): the paint is read
      // from it again (GLGizmoPainterBase.cpp `on_load`).
      // The project as the stage now has it (an edit's answer reaches the
      // stage before the rest of the page).
      const shown = (this.stage.model && this.stage.model.project) || project;
      const revision = shown.edit ? shown.edit.revision : null;
      if (open.ready && revision !== open.revision) {
        open.revision = revision;
        this.queue(() => this.send({ op: 'refresh' }).then(() => this.panel()));
      }
    },

    // The colours of the states, as the reference draws them, and of the
    // object's own triangles.
    colours() {
      const open = this.open;
      if (!open || !open.palette) return;
      const project = this.host.project();
      const colours = new Uint8Array(256 * 4);
      const filament = (n) => [...GL.Scene.filamentColour(project.colours, Math.max(n, 1)), 1];
      const of = (state) => {
        if (open.kind === 'color') return filament(state);
        return [NEUTRAL, ENFORCER, BLOCKER, STABILIZER][state] || NEUTRAL;
      };
      // TriangleSelectorGUI::get_seed_fill_color: 1.25 times as light.
      const lighter = (colour) => [...colour.slice(0, 3).map((c) => Math.min(c * 1.25, 1)), 1];
      for (let mark = 0; mark < 256; mark += 1) {
        const colour = mark < MARKED ? of(mark) : lighter(of(mark - MARKED));
        colours.set(colour.map((part) => Math.round(part * 255)), mark * 4);
      }
      this.stage.renderer.setPalette(open.palette, colours);
    },

    // ---- the session ----

    // The copy's matrix, the camera's place, and the section view, which
    // every event carries.
    view() {
      const open = this.open;
      const data = this.host.data();
      const placing = data.placings.find((one) => one.id === open.copy);
      return { project: open.project, matrix: placing.matrix, eye: this.stage.camera.position, clip: this.clipPlane() };
    },

    rayAt(x, y) {
      const ray = this.stage.camera.ray(x, y);
      return [...ray.origin, ...ray.direction];
    },

    // Sends an event; the answer's blocks replace the page's.
    async send(args) {
      const open = this.open;
      if (!open) return null;
      const s = this.settings(open.kind);
      const tool = this.toolOf();
      const bytes = await this.host.bytes('paint_event', {
        ...this.view(), tool, radius: s.radius, height: s.height,
        angle: open.kind === 'color' && tool === 'bucket' && !s.edge ? -1 : s.angle,
        highlight: open.kind === 'supports' && s.overhangs ? Math.min(s.threshold + 1, 90) : 0,
        ...args,
      });
      if (this.open !== open) return null;
      return this.take(bytes);
    },

    // The tool as the session names it: the colour gizmo's "Fill" is the
    // bucket fill, with "Edge detection" its angle (GLGizmoMmuSegmentation.cpp
    // BUCKET_FILL); the others' "Fill" is the smart fill.
    toolOf() {
      const open = this.open;
      const tool = this.settings(open.kind).tool;
      if (tool === 'fill') return open.kind === 'color' ? 'bucket' : 'fill';
      if (tool === 'gap') return 'gap';
      return tool;
    },

    take(bytes) {
      const open = this.open;
      const view = new DataView(bytes);
      const length = view.getUint32(0, true);
      const answer = JSON.parse(new TextDecoder().decode(new Uint8Array(bytes, 4, length)));
      let at = 4 + length;
      while (at % 4) at += 1;
      const r = this.stage.renderer;
      if (answer.all) {
        for (const block of open.blocks.values()) r.drop(block);
        open.blocks.clear();
      }
      while (at + 12 <= bytes.byteLength) {
        const [part, chunk, count] = [view.getUint32(at, true), view.getUint32(at + 4, true), view.getUint32(at + 8, true)];
        at += 12;
        const key = `${part}:${chunk}`;
        r.drop(open.blocks.get(key));
        open.blocks.delete(key);
        if (count) open.blocks.set(key, r.mesh(bytes.slice(at, at + count * 48), count * 3));
        at += count * 48;
      }
      this.hit = answer.hit;
      if (answer.band !== undefined) this.setBand(answer.band);
      open.states = answer.states || [];
      open.note = answer.note || '';
      if (answer.edited) {
        // The stroke is a step of the history: the project and its scene
        // as they now are, as any edit answers. The paint here is what the
        // model now holds.
        open.revision = answer.edited.project.edit ? answer.edited.project.edit.revision : null;
        GL.editor.take(answer.edited);
      }
      this.stage.frame();
      return answer;
    },

    // One event at a time: what comes while one is on its way waits its
    // turn, in order. A hover or a drag is `passing`: one waiting is
    // replaced by the next (a drag reaches back to where the last one
    // ended), never one that is not (a press, a commit, a tool changed).
    async queue(job, passing = false) {
      const waiting = this.waiting || (this.waiting = []);
      const last = waiting[waiting.length - 1];
      if (passing && last && last.passing) waiting[waiting.length - 1] = { job, passing };
      else waiting.push({ job, passing });
      if (this.busy) return;
      this.busy = true;
      try {
        while (waiting.length) {
          const next = waiting.shift();
          try {
            await next.job();
          } catch (error) {
            this.host.say(error.message, true);
          }
        }
      } finally {
        this.busy = false;
      }
    },

    // ---- the pointer ----

    hover(x, y) {
      if (!this.open || !this.open.ready) return false;
      this.pointer = [x, y];
      this.placeCursor();
      if (this.stroke) return true;
      const tool = this.toolOf();
      this.queue(() => this.send({ op: 'hover', ray: this.rayAt(x, y) }).then(() => this.placeCursor()), true);
      if (tool === 'gap') return true;
      return true;
    },

    // The button goes down on the stage. True when the tool takes it: over
    // the object, with a button the tool paints with, Ctrl not held
    // (`GLGizmoPainterBase::on_mouse`).
    press(x, y, event) {
      const open = this.open;
      if (!open || !open.ready || event.ctrlKey || event.metaKey) return false;
      const kind = KINDS[open.kind];
      const right = event.button === 2;
      if (event.button !== 0 && !right) return false;
      const s = this.settings(open.kind);
      if (s.tool === 'gap') return false;
      // The right button with Shift is the right button
      // (`gizmo_event(RightDown, …, false, …)`).
      const erase = event.shiftKey && !right;
      let state;
      if (erase) state = 0;
      else if (open.kind === 'color') {
        if (right) return false;
        state = s.filament;
      } else {
        state = right ? kind.right : kind.left;
      }
      if (state === null) return false;
      // Over the object: its triangles drawn alone for the pixel under the
      // pointer, the section view's cut taken away.
      if (!this.over(x, y)) return false;
      this.stroke = { state, right, name: kind.step(erase, right, s.filament), from: [x, y], first: [x, y], painted: false };
      this.placeCursor();
      this.queue(() => this.send({ op: 'paint', rays: [this.rayAt(x, y)], state }).then((answer) => {
        if (this.stroke && answer && answer.hit) this.stroke.painted = true;
      }));
      return true;
    },

    // Whether the object painted is under a pixel of the view.
    over(x, y) {
      const open = this.open;
      const data = this.host.data();
      const placing = data && data.placings.find((one) => one.id === open.copy);
      if (!placing || !open.base) return false;
      const r = this.stage.renderer;
      const frame = this.stage.matrices();
      const plane = this.clipPlane();
      return r.pick(x, y, frame.projection, (projection) => {
        r.drawMesh(open.base, { view: frame.view, projection }, placing.matrix, { tint: GL.Renderer.pickColour(1), clip: plane ? [...plane.normal, plane.offset] : null, emission: 10 });
      }) !== 0;
    },

    // The pointer's way from where it last painted to (x, y), a pixel at a
    // time, the newest first (`get_projected_mouse_positions`), held to a
    // line across or down with "Horizontal" or "Vertical".
    drag(x, y) {
      const stroke = this.stroke;
      if (!stroke || !this.open) return;
      const s = this.settings(this.open.kind);
      if (['circle', 'sphere', 'triangle', 'height'].includes(s.tool)) {
        if (s.vertical) x = stroke.first[0];
        else if (s.horizontal) y = stroke.first[1];
      }
      this.pointer = [x, y];
      this.placeCursor();
      this.queue(() => {
        const [fx, fy] = stroke.from;
        const steps = Math.floor(Math.hypot(x - fx, y - fy));
        const rays = [this.rayAt(x, y)];
        for (let step = 1; step <= steps; step += 1) {
          const k = step / (steps + 1);
          rays.push(this.rayAt(x + (fx - x) * k, y + (fy - y) * k));
        }
        if (steps > 0) rays.push(this.rayAt(fx, fy));
        stroke.from = [x, y];
        return this.send({ op: 'paint', rays, state: stroke.state }).then((answer) => {
          if (answer && answer.hit) stroke.painted = true;
        });
      }, true);
    },

    // The button is let go: the stroke goes into the model as one step.
    release() {
      const stroke = this.stroke;
      if (!stroke) return;
      this.stroke = null;
      this.queue(() => this.send({ op: 'commit', name: stroke.name, state: stroke.state }).then(() => this.panel()));
    },

    // Ctrl with the wheel sizes the brush (or the angle, the height, the
    // gap's area), Alt with the wheel moves the section view
    // (`GLGizmoPainterBase::gizmo_event`, `MouseWheelUp` / `Down`). True
    // when taken.
    wheel(event) {
      const open = this.open;
      if (!open || !open.ready) return false;
      const up = event.deltaY < 0;
      const s = this.settings(open.kind);
      if (event.ctrlKey || event.metaKey) {
        const step = (limits, key) => { s[key] = Math.round(Math.min(limits.max, Math.max(limits.min, s[key] + (up ? limits.step : -limits.step))) * 100) / 100; };
        if (s.tool === 'height') step(HEIGHT, 'height');
        else if (s.tool === 'circle' || s.tool === 'sphere') step(RADIUS, 'radius');
        else if (s.tool === 'fill') {
          step(ANGLE, 'angle');
          if (this.pointer) this.queue(() => this.send({ op: 'hover', ray: this.rayAt(...this.pointer), force: true }));
        } else if (s.tool === 'gap') {
          step(GAP, 'gap');
          this.gapChanged();
        } else return true;
        this.panel();
        this.placeCursor();
        this.stage.frame();
        return true;
      }
      if (event.altKey) {
        this.setClip(Math.min(1, Math.max(0, s.clip + (up ? 0.01 : -0.01))));
        return true;
      }
      return false;
    },

    // ---- keys ----

    // A key while the tool is open: the tool's own first
    // (`GLGizmosManager::on_key`). True when taken.
    key(event) {
      const open = this.open;
      if (!open || event.ctrlKey || event.metaKey || event.altKey) return false;
      const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
      const kind = KINDS[open.kind];
      if (kind.keys[key] && !event.shiftKey) { this.setTool(kind.keys[key]); return true; }
      // The colour gizmo's filaments by their numbers: 1 to 9, and 1 then 0
      // to 6 within half a second for 10 to 16.
      if (open.kind === 'color' && /^[0-9]$/.test(key)) {
        const digit = Number(key);
        const now = performance.now();
        let filament = digit;
        if (this.lastOne && now - this.lastOne < 500 && digit <= 6) filament = 10 + digit;
        this.lastOne = digit === 1 ? now : 0;
        if (filament >= 1 && filament <= this.host.filamentCount()) {
          this.settings('color').filament = filament;
          this.panel();
        }
        return true;
      }
      return false;
    },

    // ---- the panel ----

    setTool(tool) {
      const open = this.open;
      if (!open) return;
      const s = this.settings(open.kind);
      if (s.tool === tool) return;
      const wasGap = s.tool === 'gap';
      s.tool = tool;
      // The marks of a fill go with the tool (`seed_fill_unselect_all_triangles`).
      this.queue(() => this.send({ op: 'unmark' }));
      if (wasGap || tool === 'gap') this.gapChanged();
      this.setBand([]);
      this.panel();
      this.placeCursor();
    },

    gapChanged() {
      const open = this.open;
      if (!open) return;
      const s = this.settings(open.kind);
      this.queue(() => this.send({ op: 'gap', area: s.tool === 'gap' ? s.gap : null }));
    },

    panelNode: null,

    // The tool's panel, under the toolbar where the other tools' fields
    // are.
    panel() {
      const open = this.open;
      const { el } = this.host;
      if (!this.panelNode) {
        this.panelNode = el('div', { className: 'edit-panel overlay paint-panel', id: 'paint-panel', hidden: true });
        this.host.stageNode.append(this.panelNode);
      }
      const node = this.panelNode;
      if (!open) { node.hidden = true; return; }
      node.hidden = false;
      const kind = KINDS[open.kind];
      const s = this.settings(open.kind);
      const rows = [el('h3', { textContent: kind.name })];
      const field = (label, limits, key, unit, changed) => {
        const range = el('input', { type: 'range', min: String(limits.min), max: String(limits.max), step: String(limits.step / 4), value: String(s[key]) });
        range.setAttribute('aria-label', label);
        const number = el('input', { type: 'number', min: String(limits.min), max: String(limits.max), step: String(limits.step), value: String(Math.round(s[key] * 100) / 100) });
        number.setAttribute('aria-label', label);
        const set = (value) => {
          if (!Number.isFinite(value)) return;
          s[key] = Math.min(limits.max, Math.max(limits.min, value));
          range.value = String(s[key]);
          number.value = String(Math.round(s[key] * 100) / 100);
          this.placeCursor();
          if (changed) changed();
          this.stage.frame();
        };
        range.addEventListener('input', () => set(Number(range.value)));
        number.addEventListener('change', () => set(Number(number.value)));
        return el('div', { className: 'paint-row' }, el('span', { className: 'edit-label', textContent: label }), el('span', { className: 'paint-slide' }, range, el('span', { className: 'unit' }, number, unit ? ` ${unit}` : '')));
      };
      const check = (label, key, changed, title = '') => {
        const box = el('input', { type: 'checkbox', checked: Boolean(s[key]) });
        box.addEventListener('change', () => { s[key] = box.checked; if (changed) changed(); this.stage.frame(); });
        const label_ = el('label', { className: 'edit-check', title }, box, ` ${label}`);
        return label_;
      };
      // The colour gizmo's filaments, by colour and number.
      if (open.kind === 'color') {
        const count = this.host.filamentCount();
        const project = this.host.project();
        const swatches = el('div', { className: 'paint-filaments', role: 'radiogroup' });
        swatches.setAttribute('aria-label', t('Filaments'));
        for (let filament = 1; filament <= count; filament += 1) {
          const colour = GL.Scene.filamentColour(project.colours, filament).map((c) => Math.round(c * 255));
          const button = el('button', { type: 'button', className: 'paint-filament', textContent: String(filament) });
          button.style.setProperty('--swatch', `rgb(${colour.join(',')})`);
          // Black or white figures, whichever reads on the colour.
          const light = 0.2126 * colour[0] + 0.7152 * colour[1] + 0.0722 * colour[2] > 140;
          button.classList.toggle('on-light', light);
          button.setAttribute('aria-pressed', String(s.filament === filament));
          button.title = filament <= 9 ? `${t('Shortcut Key ')}${filament}` : fmt(t('Filament %d'), filament);
          button.addEventListener('click', () => { s.filament = filament; this.panel(); });
          swatches.append(button);
        }
        rows.push(el('div', { className: 'paint-row' }, el('span', { className: 'edit-label', textContent: t('Filaments') }), swatches));
        rows.push(el('p', { className: 'edit-hint', textContent: `${t('Selected filament')} ${s.filament} ${fmt(t('/ %d'), count)}` }));
      }
      // The tools, as icon buttons.
      const tools = el('div', { className: 'paint-tools', role: 'radiogroup' });
      tools.setAttribute('aria-label', kind.shape);
      for (const tool of kind.tools) {
        const button = el('button', { type: 'button', className: 'icon tool', title: TOOL_NAMES[tool] });
        button.setAttribute('aria-label', TOOL_NAMES[tool]);
        button.setAttribute('aria-pressed', String(s.tool === tool));
        button.append(svg(ICON[tool === 'fill' ? 'fill' : tool]));
        button.addEventListener('click', () => this.setTool(tool));
        tools.append(button);
      }
      rows.push(el('div', { className: 'paint-row' }, el('span', { className: 'edit-label', textContent: kind.shape }), tools));
      if (open.kind === 'supports') {
        rows.push(check(t('On overhangs only'), 'overhangs', null, fmt(t('Allows painting only on facets selected by: "%1%"'), t('Threshold Angle'))));
      }
      if (s.tool === 'circle' || s.tool === 'sphere') {
        rows.push(field(kind.size, RADIUS, 'radius', 'mm'));
      } else if (s.tool === 'height') {
        rows.push(field(t('Height range'), HEIGHT, 'height', 'mm'));
      } else if (s.tool === 'fill') {
        if (open.kind === 'color') rows.push(check(t('Edge detection'), 'edge', () => this.panel()));
        if (open.kind !== 'color' || s.edge) rows.push(field(t('Smart fill angle'), ANGLE, 'angle', '°', () => this.queue(() => this.send({ op: 'unmark' }))));
      } else if (s.tool === 'gap') {
        rows.push(field(t('Gap area'), GAP, 'gap', 'mm²', () => this.gapChanged()));
      }
      if (open.kind === 'supports') {
        rows.push(field(t('Threshold Angle'), { min: 0, max: 90, step: 1 }, 'threshold', '°'));
      }
      // "Vertical" and "Horizontal" hold a drag to a line: the colour
      // gizmo's for every tool, the support gizmo's for its brushes, the
      // seam gizmo's "Vertical" alone.
      if (open.kind === 'color' || (open.kind === 'supports' && ['circle', 'sphere'].includes(s.tool))) {
        rows.push(el('div', { className: 'paint-checks' },
          check(t('Vertical'), 'vertical', () => { if (s.vertical) s.horizontal = false; this.panel(); }),
          check(t('Horizontal'), 'horizontal', () => { if (s.horizontal) s.vertical = false; this.panel(); })));
      } else if (open.kind === 'seam') {
        rows.push(check(t('Vertical'), 'vertical'));
      }
      // "Section view", or "Reset direction" once it cuts.
      if (s.tool !== 'gap') {
        const clip = { min: 0, max: 1, step: 0.01 };
        const row = field(t('Section view'), clip, 'clip', '', () => this.setClip(s.clip));
        if (s.clip > 0) {
          const reset = el('button', { type: 'button', className: 'quiet', textContent: t('Reset direction') });
          reset.addEventListener('click', () => { s.clipNormal = null; this.setClip(s.clip); });
          row.firstChild.replaceWith(reset);
        }
        rows.push(row);
      }
      // What the buttons and keys do (`show_tooltip_information`).
      const help = el('div', { className: 'paint-help' });
      for (const [keys, what] of kind.rows) help.append(el('span', { className: 'paint-key', textContent: keys }), el('span', { textContent: what }));
      rows.push(help);
      const foot = el('div', { className: 'paint-foot' });
      if (s.tool === 'gap') {
        const perform = el('button', { type: 'button', textContent: t('Perform') });
        perform.addEventListener('click', () => this.queue(() => this.send({ op: 'gap_fill', area: s.gap, name: open.kind === 'color' ? 'Gap fill' : 'Reset selection' })));
        foot.append(perform);
      }
      const erase = el('button', { type: 'button', textContent: t('Erase all painting') });
      erase.disabled = !open.states.length;
      erase.addEventListener('click', () => this.queue(() => this.send({ op: 'erase_all', name: 'Reset selection' }).then(() => this.panel())));
      foot.append(erase);
      rows.push(foot);
      // What the engine does with it (the engine's own notes, as a slice
      // gives them).
      const note = this.note();
      if (note) rows.push(el('p', { className: 'edit-hint paint-note', textContent: note }));
      const focused = node.contains(document.activeElement) ? document.activeElement.getAttribute('aria-label') : null;
      node.replaceChildren(...rows);
      if (focused) {
        const again = [...node.querySelectorAll('[aria-label]')].find((one) => one.getAttribute('aria-label') === focused);
        if (again) again.focus();
      }
    },

    // What the engine says of the paint when it slices it, where it does
    // not follow it (the session gives its note: painted supports and
    // fuzzy skin, until the engine follows them).
    note() {
      return this.open ? this.open.note || '' : '';
    },

    // ---- the section view ----

    // The plane that cuts the object at `ratio` of its depth seen from the
    // camera (ObjectClipper::set_position_by_ratio): its normal toward the
    // camera, kept while the ratio moves, from the far side of the copy's
    // bounding sphere (ratio 1) to its near side (ratio 0, nothing cut).
    setClip(ratio) {
      const open = this.open;
      if (!open) return;
      const s = this.settings(open.kind);
      s.clip = ratio;
      if (ratio > 0 && !s.clipNormal) s.clipNormal = M.scale(this.stage.camera.forward, -1);
      if (ratio === 0) s.clipNormal = null;
      this.panel();
      this.stage.frame();
    },

    clipPlane() {
      const open = this.open;
      if (!open) return null;
      const s = this.settings(open.kind);
      if (!(s.clip > 0) || !s.clipNormal) return null;
      const data = this.host.data();
      const placing = data.placings.find((one) => one.id === open.copy);
      if (!placing || !placing.box) return null;
      const centre = M.centre(placing.box);
      const radius = M.length(M.sub(placing.box[1], placing.box[0])) / 2;
      const offset = M.dot(s.clipNormal, centre) + radius - s.clip * 2 * radius;
      return { normal: s.clipNormal, offset };
    },

    // ---- drawing ----

    draw(frame) {
      const open = this.open;
      if (!open || !open.base) return;
      const data = this.host.data();
      const placing = data && data.placings.find((one) => one.id === open.copy);
      if (!placing) return;
      const r = this.stage.renderer;
      const gl = r.gl;
      const s = this.settings(open.kind);
      const plane = this.clipPlane();
      const clip = plane ? [...plane.normal, plane.offset] : null;
      // TriangleSelectorPatch: the object's own triangles, grey but for the
      // colour gizmo, where each part is its filament's colour.
      const slope = open.kind === 'supports' && s.threshold > 0 ? -Math.cos((Math.min(s.threshold + 1, 90) * Math.PI) / 180) : null;
      r.drawMesh(open.base, frame, placing.matrix, { palette: open.palette, tint: open.kind === 'color' ? [0, 0, 0, -1] : NEUTRAL, clip, slope });
      // The paint over them, a hair in front.
      gl.enable(gl.POLYGON_OFFSET_FILL);
      gl.polygonOffset(-1, -2);
      for (const block of open.blocks.values()) r.drawMesh(block, frame, placing.matrix, { palette: open.palette, clip });
      gl.disable(gl.POLYGON_OFFSET_FILL);
      this.drawCursor(frame);
    },

    shapes: null,

    dropShapes() {
      if (!this.shapes) return;
      const r = this.stage.renderer;
      r.drop(this.shapes.sphere);
      if (this.shapes.band) r.drop(this.shapes.band);
      this.shapes = null;
    },

    sphere() {
      if (!this.shapes) this.shapes = { sphere: null, band: null };
      if (!this.shapes.sphere) {
        // A ball of 1 mm, 16 by 32.
        const points = [];
        const at = (i, j) => {
          const [theta, phi] = [(Math.PI * i) / 16, (2 * Math.PI * j) / 32];
          const p = [Math.sin(theta) * Math.cos(phi), Math.sin(theta) * Math.sin(phi), Math.cos(theta)];
          return [p, p];
        };
        for (let i = 0; i < 16; i += 1) {
          for (let j = 0; j < 32; j += 1) points.push(at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j), at(i + 1, j + 1), at(i, j + 1));
        }
        this.shapes.sphere = this.stage.renderer.meshFromPoints(points);
      }
      return this.shapes.sphere;
    },

    setBand(segments) {
      if (!this.shapes) this.shapes = { sphere: null, band: null };
      if (this.shapes.band) this.stage.renderer.drop(this.shapes.band);
      this.shapes.band = segments && segments.length ? this.stage.renderer.lines(segments.flat(2)) : null;
    },

    // The cursor (`render_cursor`): a sphere at the point under the
    // pointer, the height range's band, or for the circle a ring round the
    // pointer (`cursorNode`).
    drawCursor(frame) {
      const open = this.open;
      const s = this.settings(open.kind);
      const r = this.stage.renderer;
      if (s.tool === 'sphere' && this.hit) {
        const colour = this.stroke ? (this.stroke.state === 0 ? CURSOR.hover : this.stroke.right ? CURSOR.right : CURSOR.left) : CURSOR.hover;
        r.blend(true);
        r.drawMesh(this.sphere(), frame, M.chain(M.translation(...this.hit.point), M.scaling(s.radius, s.radius, s.radius)), { tint: colour });
        r.blend(false);
      }
      if (s.tool === 'height' && this.shapes && this.shapes.band) {
        r.drawLines(this.shapes.band, M.multiply(frame.projection, frame.view), [1, 1, 1, 1], 1.5);
      }
    },

    cursorNode() {
      if (!this.cursor) {
        this.cursor = this.host.el('div', { className: 'paint-cursor', hidden: true });
        this.host.stageNode.append(this.cursor);
      }
      return this.cursor;
    },

    // The circle cursor: a ring of the brush's size round the pointer
    // (`render_cursor_circle`: the radius times the zoom), shown while the
    // pointer is over the object.
    placeCursor() {
      const node = this.cursorNode();
      const open = this.open;
      const s = open ? this.settings(open.kind) : null;
      if (!open || !this.pointer || s.tool !== 'circle' || (!this.hit && !this.stroke)) {
        node.hidden = true;
        return;
      }
      const radius = s.radius * this.stage.camera.zoom;
      const box = this.stage.canvas.getBoundingClientRect();
      const stage = this.host.stageNode.getBoundingClientRect();
      node.hidden = false;
      node.style.width = `${2 * radius}px`;
      node.style.height = `${2 * radius}px`;
      node.style.left = `${box.left - stage.left + this.pointer[0] - radius}px`;
      node.style.top = `${box.top - stage.top + this.pointer[1] - radius}px`;
    },

    leave() {
      this.pointer = null;
      this.hit = null;
      this.placeCursor();
      this.stage.frame();
    },
  };

  function svg(path) {
    const node = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    node.setAttribute('viewBox', '0 0 16 16');
    node.setAttribute('aria-hidden', 'true');
    const line = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    line.setAttribute('d', path);
    node.append(line);
    return node;
  }

  GL.painter = paint;
})(window.EdgeGL);
