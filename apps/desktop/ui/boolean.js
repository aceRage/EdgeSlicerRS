'use strict';

// "Mesh Boolean" [B]: two parts of one object made one by a union, a
// difference or an intersection (Gizmos/GLGizmoMeshBoolean.cpp). The
// parts are picked here, in the page: by a click on them in the scene (the
// first click picks the first part, then the second), with each slot's
// "Select" to say which a click picks, its ✕ to let it go, and "Pick from
// list" for a part a click cannot reach (one inside another). The first
// part's box is drawn white, the second's teal, as the reference draws
// them (`on_render`). The button named for the operation is one
// `scene_edit` (`mesh_boolean`); the part made is selected after, which
// closes the tool, as the reference selects it in the object list.
//
// Where this differs, on purpose (UI_PARITY.md, section 13, "Mesh
// Boolean"): only printed parts can be picked, and the tool is for an
// object with two of them; "Delete input" starts unticked each time the
// tool opens (the reference means to reset it, `on_set_state`, but sets
// local variables of the same names instead).
(function (GL) {
  const M = GL.math;

  // GLGizmoMeshBoolean.cpp `on_render`: the first part's box white, the
  // second's teal (0, 150, 136).
  const SOURCE = [1, 1, 1, 1];
  const TOOL = [0, 150 / 255, 136 / 255, 1];
  const OPERATIONS = [['union', t('Union')], ['difference', t('Difference')], ['intersection', t('Intersection')]];

  // The box round a box (in a part's own coordinates) where a matrix puts it.
  const boxUnder = (matrix, local) => {
    let box = null;
    for (const corner of M.corners(local)) {
      const at = M.point(matrix, corner);
      box = M.union(box, [at, at]);
    }
    return box;
  };

  const mesher = {
    // What is open: the copy, the operation, which slot a click fills,
    // the two parts (by number) and each operation's "Delete input".
    open: null,

    init(host, editor) {
      this.host = host;
      this.stage = host.stage;
      this.editor = editor;
    },

    active() {
      return Boolean(this.open);
    },

    // `on_set_state`, On: nothing picked, "Union", the first slot next.
    start(placing) {
      this.open = { copy: placing.id, object: placing.object, operation: 'union', selecting: 'source', source: null, tool: null, deleteInput: { difference: false, intersection: false } };
      this.stage.frame();
    },

    close() {
      this.open = null;
      this.dropBoxes();
      if (this.panelNode) this.panelNode.hidden = true;
    },

    // What is selected changed, or the scene: another copy starts afresh
    // (`on_render`, a new object), and a part that is gone is let go.
    follow(placing) {
      const o = this.open;
      if (!o) return;
      if (!placing) { this.close(); return; }
      if (placing.id !== o.copy) { this.start(placing); return; }
      o.object = placing.object;
      const ids = new Set(this.printed().map((part) => part.id));
      if (o.source !== null && !ids.has(o.source)) o.source = null;
      if (o.tool !== null && !ids.has(o.tool)) o.tool = null;
    },

    // ---- the parts ----

    data() {
      return this.editor.data();
    },

    placing() {
      const data = this.data();
      return data && this.open ? data.placings.find((placing) => placing.id === this.open.copy) : null;
    },

    // The object's parts, each with its place in the object's list.
    partList() {
      const data = this.data();
      const placing = this.placing();
      return data && placing ? (data.objects[placing.object].part_list || []).map((part, at) => ({ ...part, at })) : [];
    },

    // The parts that can be picked: printed parts.
    printed() {
      return this.partList().filter((part) => part.kind === 'part');
    },

    part(id) {
      return id === null ? null : this.printed().find((part) => part.id === id) || null;
    },

    // `set_src_volume`, `set_tool_volume`: a part picked for one slot is
    // let go from the other.
    set(slot, id) {
      const o = this.open;
      o[slot] = id;
      const other = slot === 'source' ? 'tool' : 'source';
      if (o[other] === id) o[other] = null;
    },

    // The printed part under a pixel, nearest the eye (`gizmo_event`, its
    // ray cast on every part of the object), or null.
    partAt(x, y) {
      const placing = this.placing();
      const scene = this.stage.scene;
      const mesh = placing && scene.meshOf(placing.object);
      if (!mesh) return null;
      const parts = this.printed();
      const r = this.stage.renderer;
      const frame = this.stage.matrices();
      const hit = r.pick(x, y, frame.projection, (projection) => {
        const matrix = M.multiply(M.multiply(projection, frame.view), placing.matrix);
        parts.forEach((part, at) => r.drawFlat(mesh, matrix, GL.Renderer.pickColour(at + 1), undefined, part.triangles));
      });
      return hit ? parts[hit - 1] || null : null;
    },

    // The left button with the tool open: a click on a part picks it for
    // the slot being filled, the first then the second; nothing else is
    // done with the press, as the reference's gizmo takes it whole.
    grab(x, y) {
      if (!this.open || this.editor.busy) return null;
      const part = this.partAt(x, y);
      // Off the object's printed parts the view turns, as with no tool.
      if (!part) return null;
      return {
        kind: 'boolean',
        cancelled: true,
        tap: () => {
          if (!this.open) return;
          if (this.open.selecting === 'tool') {
            this.set('tool', part.id);
          } else {
            this.set('source', part.id);
            this.open.selecting = 'tool';
          }
          this.editor.refresh();
        },
      };
    },

    // ---- the boxes ----

    boxes() {
      const o = this.open;
      const placing = this.placing();
      if (!o || !placing) return [];
      const out = [];
      for (const [slot, colour] of [['source', SOURCE], ['tool', TOOL]]) {
        const part = this.part(o[slot]);
        if (part && part.box) out.push({ box: boxUnder(M.multiply(placing.matrix, part.matrix), part.box), colour });
      }
      return out;
    },

    dropBoxes() {
      if (this.lines) for (const held of this.lines.list) this.stage.renderer.drop(held);
      this.lines = null;
    },

    // Each box drawn as the selection's is (Selection.cpp
    // `render_bounding_box`: a fifth of each edge at each corner).
    draw(frame) {
      const boxes = this.boxes();
      const key = JSON.stringify(boxes);
      const r = this.stage.renderer;
      if (!this.lines || this.lines.key !== key) {
        this.dropBoxes();
        const list = boxes.map(({ box }) => {
          const ends = [];
          for (const corner of M.corners(box)) {
            for (let axis = 0; axis < 3; axis += 1) {
              const other = corner.slice();
              const far = corner[axis] === box[0][axis] ? box[1][axis] : box[0][axis];
              other[axis] = corner[axis] + (far - corner[axis]) * 0.2;
              ends.push(...corner, ...other);
            }
          }
          return r.lines(ends);
        });
        this.lines = { key, list };
      }
      const gl = r.gl;
      gl.disable(gl.DEPTH_TEST);
      const matrix = M.multiply(frame.projection, frame.view);
      this.lines.list.forEach((lines, at) => r.drawLines(lines, matrix, boxes[at].colour, 2));
      gl.enable(gl.DEPTH_TEST);
    },

    // ---- the panel ----

    canApply() {
      const o = this.open;
      return Boolean(o && this.part(o.source) && this.part(o.tool) && !this.editor.busy);
    },

    // `run_boolean`: the result in the first part's place; the second
    // deleted for a union, and for the others when "Delete input" is
    // ticked.
    async apply() {
      if (!this.canApply()) return;
      const o = this.open;
      const data = this.data();
      const object = data.objects[this.placing().object].id;
      const operation = o.operation;
      const args = {
        op: 'mesh_boolean', object, source: o.source, tool: o.tool, operation,
        delete_input: operation === 'union' ? true : o.deleteInput[operation],
      };
      const done = await this.editor.commit(args, { part: true });
      // Done: nothing is picked, the first slot is next
      // (`generate_new_volume`); the part made is selected, which closes
      // the tool. Refused: the warning is said, the picks stay.
      if (done && this.open) {
        this.open.source = null;
        this.open.tool = null;
        this.open.selecting = 'source';
        this.editor.refresh();
      }
    },

    panel() {
      const { el } = this.host;
      if (!this.panelNode) {
        this.panelNode = el('div', { className: 'edit-panel overlay cut-panel boolean-panel', id: 'boolean-panel', hidden: true });
        this.panelNode.setAttribute('role', 'group');
        this.panelNode.setAttribute('aria-label', t('Mesh Boolean'));
        this.host.stageNode.append(this.panelNode);
      }
      const node = this.panelNode;
      const o = this.open;
      if (!o) { node.hidden = true; return; }
      node.hidden = false;
      const busy = this.editor.busy;
      const rows = [el('h3', { textContent: t('Mesh Boolean') })];
      // "Union", "Difference", "Intersection": two joined buttons, as
      // STYLE.md has a choice of one of a few.
      const modes = el('div', { className: 'joined segmented boolean-modes' });
      modes.setAttribute('role', 'group');
      modes.setAttribute('aria-label', t('Mesh Boolean'));
      for (const [value, label] of OPERATIONS) {
        const button = el('button', { type: 'button', textContent: label, disabled: busy });
        button.setAttribute('aria-pressed', String(o.operation === value));
        button.addEventListener('click', () => { o.operation = value; this.panel(); });
        modes.append(button);
      }
      rows.push(modes);
      const difference = o.operation === 'difference';
      const printed = this.printed();
      // A slot: its caption, "Select" (or "1 selected"), which is pressed
      // while a click fills it, the part's name with ✕, and "Pick from
      // list".
      const slot = (key, caption, colour) => {
        const part = this.part(o[key]);
        const select = el('button', { type: 'button', className: o.selecting === key ? 'cut-armed' : '', textContent: part ? `1 ${t('selected')}` : t('Select'), disabled: busy });
        select.setAttribute('aria-pressed', String(o.selecting === key));
        select.addEventListener('click', () => { o.selecting = key; this.panel(); });
        const line = [select];
        if (part) {
          const clear = el('button', { type: 'button', className: 'icon tool', textContent: '✕', disabled: busy });
          clear.setAttribute('aria-label', `${caption}: ${part.name || ''}`);
          clear.title = caption;
          clear.addEventListener('click', () => { o[key] = null; this.editor.refresh(); });
          line.push(el('span', { className: 'boolean-name', textContent: part.name || '', title: part.name || '' }), clear);
        }
        const list = el('select', { disabled: busy });
        list.setAttribute('aria-label', `${caption}: ${t('Pick from list')}`);
        list.append(new Option(t('Pick from list'), ''));
        for (const each of printed) list.append(new Option(each.name || `${each.at + 1}`, String(each.id)));
        list.value = part ? String(part.id) : '';
        list.addEventListener('change', () => {
          if (!list.value) return;
          this.set(key, Number(list.value));
          this.editor.refresh();
        });
        line.push(list);
        // Marked in its box's colour.
        const marker = el('span', { className: 'cut-marker' });
        marker.style.setProperty('--swatch', `rgb(${colour.slice(0, 3).map((c) => Math.round(c * 255)).join(', ')})`);
        return el('div', { className: 'cut-row' },
          el('span', { className: 'cut-part' }, marker, caption),
          el('span', { className: 'cut-line' }, ...line));
      };
      rows.push(slot('source', difference ? t('Subtract from') : t('Part 1'), SOURCE));
      rows.push(slot('tool', difference ? t('Subtract with') : t('Part 2'), TOOL));
      // "Delete input", for the difference and the intersection each.
      if (o.operation !== 'union') {
        const box = el('input', { type: 'checkbox', checked: o.deleteInput[o.operation], disabled: busy });
        box.addEventListener('change', () => { o.deleteInput[o.operation] = box.checked; });
        rows.push(el('label', { className: 'edit-check' }, box, ` ${t('Delete input')}`));
      }
      const label = OPERATIONS.find(([value]) => value === o.operation)[1];
      const apply = el('button', { type: 'button', className: 'primary', textContent: label, disabled: !this.canApply() });
      apply.addEventListener('click', () => this.apply());
      rows.push(el('div', { className: 'cut-foot' }, apply));
      node.replaceChildren(...rows);
    },
  };

  GL.mesher = mesher;
})(window.EdgeGL);
