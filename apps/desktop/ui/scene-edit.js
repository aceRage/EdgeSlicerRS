'use strict';

// Prepare as an editor: the tools that change what stands on the plates.
// What the reference offers here is its main toolbar (GLCanvas3D.cpp
// `_init_main_toolbar`), its Move, Rotate and Scale gizmos
// (Gizmos/GLGizmoMove.cpp, GLGizmoRotate.cpp, GLGizmoScale.cpp) with their
// number fields (Gizmos/GizmoObjectManipulation.cpp), dragging an object
// across the bed (GLCanvas3D.cpp `on_mouse`), and undo and redo.
//
// Nothing is changed here but what is seen while a drag lasts. Every edit
// is one call, `scene_edit`, and the session answers with the project and
// its scene as they then are; `undo` and `redo` answer the same. The 3D
// stage (gl/stage.js) hands the pointer over (`grab`, `drag`, `drop`,
// `hover`) and draws the handles (`draw`); objects.js lists what there is.
(function (GL) {
  const M = GL.math;

  // libslic3r/Color.hpp ColorRGB::X, Y, Z: the axes' own colours.
  const AXES = [[1, 60 / 255, 91 / 255, 1], [100 / 255, 200 / 255, 24 / 255, 1], [47 / 255, 136 / 255, 233 / 255, 1]];
  const PLAIN = [0.92, 0.92, 0.92, 1];
  // A handle taken up or under the pointer.
  const HOT = [1, 0.84, 0.3, 1];
  const UNIT = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
  // Takes +Z to each axis in turn, as gl/scene.js turns the bed's axes.
  const TURN = [
    [0, 0, -1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 0, -1, 0, 0, 1, 0, 0, 0, 0, 0, 1],
    M.identity(),
  ];
  // In pixels: how long a move arrow is, how large a handle, and how near
  // the pointer takes one up.
  const [ARROW, TIP, KNOB, GRIP] = [84, 13, 9, 9];
  // GLGizmoRotate.cpp: Offset, the snap regions (a third and two thirds of
  // the radius: eighths of a turn; the radius to a tenth beyond: 5 degrees).
  const RING_OFFSET = 5;
  // GLGizmoMove.hpp, GLGizmoScale.cpp `m_snap_step`: with Shift held.
  const [MOVE_STEP, SCALE_STEP] = [1, 0.05];
  // GLGizmoBase.cpp FLATTEN_COLOR, FLATTEN_HOVER_COLOR: the faces to lay
  // an object on, and the one under the pointer.
  const [FACE, FACE_HOT] = [[0.96, 0.93, 0.93, 0.5], [1, 1, 1, 0.75]];

  const TOOLS = {
    move: { name: 'Move', key: 'm', icon: 'M8,1.5v13M1.5,8h13M8,1.5l-2,2M8,1.5l2,2M8,14.5l-2-2M8,14.5l2-2M1.5,8l2-2M1.5,8l2,2M14.5,8l-2-2M14.5,8l-2,2' },
    rotate: { name: 'Rotate', key: 'r', icon: 'M13.5,8a5.5,5.5,0,1,1-1.6-3.9M13.5,1.5v3h-3' },
    scale: { name: 'Scale', key: 's', icon: 'M1.5,14.5h6v-6h-6ZM9.5,6.5l5-5M14.5,1.5h-4M14.5,1.5v4' },
    // GLGizmoFlatten.cpp: for one object at a time (`is_selection_supported`).
    flatten: { name: 'Lay on face', key: 'f', one: true, icon: 'M1.5,14.5h13M4.5,11.5l2-8l7,3l-3,5Z' },
  };
  const ICONS = {
    add: 'M1.5,12.5v-9a1,1,0,0,1,1-1h3l1.5,2h6.5a1,1,0,0,1,1,1v7a1,1,0,0,1-1,1h-11A1,1,0,0,1,1.5,12.5ZM8,7v4M6,9h4',
    addPlate: 'M1.5,11.5l6.5,3l6.5-3l-6.5-3ZM8,1.5v5M5.5,4h5',
    arrange: 'M1.5,1.5h5v5h-5ZM9.5,1.5h5v5h-5ZM1.5,9.5h5v5h-5ZM9.5,9.5h5v5h-5Z',
    splitObjects: 'M1.5,4.5h5v8h-5ZM9.5,2.5h5v5h-5ZM9.5,10.5h5v3h-5ZM8,1v14',
    splitParts: 'M1.5,3.5h13v9h-13ZM6.5,3.5v9M10.5,3.5v9',
    undo: 'M5,3.5l-3,3l3,3M2,6.5h7.5a4,4,0,0,1,0,8h-3.5',
    redo: 'M11,3.5l3,3l-3,3M14,6.5h-7.5a4,4,0,0,0,0,8h3.5',
  };

  const col = (m, c) => [m[4 * c], m[4 * c + 1], m[4 * c + 2]];
  const copyBox = (box) => (box ? [box[0].slice(), box[1].slice()] : null);
  // A 3 by 3 given as rows, as a matrix of the scene's kind.
  const linear = (r) => [r[0][0], r[1][0], r[2][0], 0, r[0][1], r[1][1], r[2][1], 0, r[0][2], r[1][2], r[2][2], 0, 0, 0, 0, 1];
  // `inner` done about `centre` instead of about the origin.
  const about = (centre, inner) => M.chain(M.translation(...centre), inner, M.translation(-centre[0], -centre[1], -centre[2]));
  // The box round a box (in an object's own coordinates) where a matrix puts it.
  const boxUnder = (matrix, local) => {
    let box = null;
    for (const corner of M.corners(local)) {
      const at = M.point(matrix, corner);
      box = M.union(box, [at, at]);
    }
    return box;
  };
  const degrees = (radians) => (radians * 180) / Math.PI;
  const radians = (value) => (value * Math.PI) / 180;
  const round = (value, digits = 2) => {
    const power = 10 ** digits;
    const rounded = Math.round(value * power) / power;
    return Object.is(rounded, -0) ? 0 : rounded;
  };

  // The turn of a matrix without its sizes, as three rows, and the sizes:
  // how long each of the object's own axes has become.
  // A mirrored object (its matrix turns it inside out) is taken as
  // mirrored along its own X and then turned, as the reference splits an
  // instance's transform into turn, size and mirror (`flip`).
  function turnOf(matrix) {
    const sizes = [0, 1, 2].map((c) => M.length(col(matrix, c)) || 1);
    const flip = M.determinant3(matrix) < 0;
    const cols = [0, 1, 2].map((c) => M.scale(col(matrix, c), (c === 0 && flip ? -1 : 1) / sizes[c]));
    return { sizes, flip, rows: [0, 1, 2].map((r) => [cols[0][r], cols[1][r], cols[2][r]]) };
  }

  // The angles about X, Y and Z, in that order of turning, that make a
  // turn: the reference's own way of writing an orientation
  // (libslic3r/Geometry.cpp, R = Rz * Ry * Rx).
  function anglesOf(rows) {
    const sy = Math.max(-1, Math.min(1, -rows[2][0]));
    const y = Math.asin(sy);
    if (Math.abs(sy) > 0.999999) return [Math.atan2(-rows[1][2], rows[1][1]), y, 0];
    return [Math.atan2(rows[2][1], rows[2][2]), y, Math.atan2(rows[1][0], rows[0][0])];
  }

  function turnFrom([x, y, z]) {
    const R = M.rows;
    return R.multiply(R.about(UNIT[2], z), R.multiply(R.about(UNIT[1], y), R.about(UNIT[0], x)));
  }

  const svg = (path) => {
    const node = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    node.setAttribute('viewBox', '0 0 16 16');
    node.setAttribute('aria-hidden', 'true');
    const line = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    line.setAttribute('d', path);
    node.append(line);
    return node;
  };

  const edit = {
    host: null,
    stage: null,
    // The tool that is open: 'move', 'rotate', 'scale' or null.
    tool: null,
    // The handle under the pointer, by its id.
    hot: null,
    // A drag that is going on.
    held: null,
    // An edit that is on its way to the session.
    busy: false,
    // The copies copied (Ctrl+C), by id.
    copied: [],
    // What the number fields keep between edits.
    fields: { uniform: true, frame: 'world', relative: [0, 0, 0], of: '' },
    shapes: null,

    init(host) {
      this.host = host;
      this.stage = host.stage;
      if (!this.stage.active) return;
      this.build();
      document.addEventListener('keydown', (event) => this.key(event));
      GL.edit = this;
    },

    // ---- what is laid over the stage ----

    build() {
      const { el } = this.host;
      const button = (id, label, tip, path, act) => {
        const node = el('button', { type: 'button', className: 'icon tool', id: `edit-${id}`, title: tip });
        node.setAttribute('aria-label', label);
        node.append(svg(path));
        node.addEventListener('click', () => { node.blur(); act(); });
        return node;
      };
      const gap = () => el('span', { className: 'toolbar-gap' });
      this.buttons = {
        add: button('add', 'Add', 'Add [Ctrl+I]', ICONS.add, () => this.add()),
        addPlate: button('add-plate', 'Add plate', 'Add plate', ICONS.addPlate, () => this.commit({ op: 'add_plate' })),
        arrange: button('arrange', 'Arrange all objects', 'Arrange all objects [A]\nArrange objects on selected plates [Shift+A]', ICONS.arrange, () => this.arrange(true)),
        splitObjects: button('split-objects', 'Split to objects', 'Split to objects', ICONS.splitObjects, () => this.split(true)),
        splitParts: button('split-parts', 'Split to parts', 'Split to parts', ICONS.splitParts, () => this.split(false)),
        undo: button('undo', 'Undo', 'Undo [Ctrl+Z]', ICONS.undo, () => this.step('undo')),
        redo: button('redo', 'Redo', 'Redo [Ctrl+Y]', ICONS.redo, () => this.step('redo')),
      };
      for (const [tool, { name, key, icon }] of Object.entries(TOOLS)) {
        this.buttons[tool] = button(tool, name, `${name} [${key.toUpperCase()}]`, icon, () => this.setTool(this.tool === tool ? null : tool));
        this.buttons[tool].setAttribute('aria-pressed', 'false');
      }
      const b = this.buttons;
      this.bar = el('div', { className: 'toolbar', id: 'edit-bar' },
        b.add, b.addPlate, gap(), b.arrange, gap(), b.splitObjects, b.splitParts, gap(), b.move, b.rotate, b.scale, b.flatten, gap(), b.undo, b.redo);
      this.bar.setAttribute('role', 'toolbar');
      this.bar.setAttribute('aria-label', 'Prepare');
      this.panel = el('div', { className: 'edit-panel overlay', id: 'edit-panel', hidden: true });
      this.warning = el('p', { className: 'edit-warning overlay', id: 'edit-warning', hidden: true });
      this.warning.setAttribute('role', 'status');
      this.host.stageNode.append(this.bar, this.panel, this.warning);
    },

    // ---- what there is ----

    data() {
      const stage = this.stage;
      const project = this.host.project();
      return stage.scene.data && project && stage.scene.data.project === project.id ? stage.scene.data : null;
    },

    editing() {
      return this.host.tab() === 'prepare' && Boolean(this.host.project());
    },

    // Whether parts, not objects, are selected.
    partMode() {
      return this.stage.parts().size > 0;
    },

    // The parts selected, each as the tools take a copy: its `matrix` from
    // its own coordinates to the scene's (the copy's times the part's own,
    // `C` and `P`), its own box (`local`) and the box round it in the scene.
    partItems() {
      const data = this.data();
      const parts = this.stage.parts();
      if (!data || !parts.size) return [];
      const copy = data.placings.find((placing) => this.stage.selection().has(placing.id));
      if (!copy) return [];
      const object = data.objects[copy.object];
      const items = [];
      (object.part_list || []).forEach((part, at) => {
        if (!parts.has(part.id) || !part.box) return;
        const matrix = M.multiply(copy.matrix, part.matrix);
        items.push({
          id: part.id, part, at, copy, object: copy.object, objectId: object.id,
          C: copy.matrix, P: part.matrix, matrix, local: part.box, box: boxUnder(matrix, part.box), printable: copy.printable,
        });
      });
      return items;
    },

    // The box round the parts selected, for the scene to mark.
    partBox() {
      const chosen = this.partMode() ? this.chosen() : null;
      return chosen ? chosen.box : null;
    },

    // The copies selected, with the box round them; or the parts selected,
    // as copies of their own (`partItems`).
    chosen() {
      const data = this.data();
      if (!data) return null;
      const selection = this.stage.selection();
      if (this.partMode()) {
        const held = this.held && this.held.starts && this.held.starts[0] && this.held.starts[0].copy.at !== undefined ? this.held.starts.map((start) => start.copy) : null;
        return this.gather(held || this.partItems());
      }
      return this.gather(data.placings.filter((placing) => selection.has(placing.id)));
    },

    // What the tools need of some copies (or parts): the box round them,
    // its middle, what a turn goes about, how far out the handles are.
    gather(copies) {
      let box = null;
      for (const copy of copies) box = M.union(box, copy.box);
      if (!copies.length || !box) return null;
      const centre = M.centre(box);
      // What a turn goes about: the middle of the one object's own box,
      // which a turn leaves where it is, so that turning back comes back
      // (the reference turns a single instance about itself); for
      // several, the middle of the box round them all.
      const local = copies.length === 1 ? this.localBox(copies[0]) : null;
      const pivot = local ? M.point(copies[0].matrix, M.centre(local)) : centre;
      return { copies, box, centre, pivot, radius: M.length(M.sub(box[1], box[0])) / 2 };
    },

    // An object's own box, from its mesh; null until the mesh is here. A
    // part's own box, for a part.
    localBox(placing) {
      if (placing.local) return placing.local;
      const mesh = this.stage.scene.meshOf(placing.object);
      return mesh && mesh.box ? mesh.box : null;
    },

    // ---- the session ----

    // Shows what the session answered an edit with.
    take(answer, { select = false, part = false } = {}) {
      const project = answer.project;
      const copy = [...this.stage.selection()][0];
      this.stage.adopt(project, answer.scene);
      if (select && answer.made && answer.made.length) this.stage.select(answer.made);
      if (part && answer.part && copy !== undefined) this.stage.selectParts(copy, [answer.part]);
      this.host.applied(project);
      this.host.say(answer.note || '', false);
      this.refresh();
    },

    // Makes one edit. False when it was refused, which is said.
    async commit(args, more = {}) {
      const project = this.host.project();
      if (!project || this.busy) return false;
      this.busy = true;
      try {
        this.take(await this.host.call('scene_edit', { project: project.id, ...args }), more);
        return true;
      } catch (error) {
        this.host.say(error.message, true);
        return false;
      } finally {
        this.busy = false;
        this.refresh();
      }
    },

    async step(which) {
      const project = this.host.project();
      if (!project || this.busy || !project.edit || !project.edit[which] || !this.editing()) return;
      this.busy = true;
      try {
        this.take(await this.host.call(which, { project: project.id }));
      } catch (error) {
        this.host.say(error.message, true);
      } finally {
        this.busy = false;
        this.refresh();
      }
    },

    // Sets where the copies selected stand: each by its matrix in the scene.
    // Parts are set within their object: the copy's matrix taken back off.
    place(name, copies) {
      if (copies.length && copies[0].local) {
        const back = M.invert(copies[0].C);
        return this.commit({ op: 'part_transform', object: copies[0].objectId, name, parts: copies.map((item) => ({ id: item.id, matrix: M.multiply(back, item.matrix) })) });
      }
      return this.commit({ op: 'transform', name, copies: copies.map((copy) => ({ id: copy.id, matrix: copy.matrix })) });
    },

    // While parts are dragged the scene draws them where they are taken.
    showMoving(items) {
      const scene = this.stage.scene;
      if (!items || !items.length || !items[0].local) { scene.moving = null; return; }
      const back = M.invert(items[0].C);
      scene.moving = { object: items[0].object, parts: new Map(items.map((item) => [item.at, M.chain(back, item.matrix, M.invert(item.P))])) };
    },

    // ---- the toolbar's own ----

    async add() {
      if (!this.editing()) return;
      const path = await this.host.pick('model', this.host.remembered.get('model') || '');
      if (!path) return;
      if (await this.commit({ op: 'add', path }, { select: true })) this.host.remembered.set('model', path);
    },

    // The one object what is selected is of, or null (Plater.cpp
    // `get_selected_object_idx`: splitting is for one object). Null while
    // parts are selected: "Split to objects" is not for a part
    // (GUI_ObjectList.cpp `is_splittable`).
    splitTarget() {
      if (this.partMode()) return null;
      const chosen = this.chosen();
      const data = this.data();
      if (!chosen || !data) return null;
      const objects = new Set(chosen.copies.map((copy) => copy.object));
      return objects.size === 1 ? data.objects[[...objects][0]] : null;
    },

    // "Split to objects" or "Split to parts" (Plater.cpp `split_object`,
    // `split_volume`); what is split to objects is selected after.
    split(toObjects) {
      const part = this.splitPart();
      if (!toObjects && part) { this.commit({ op: 'part_split', object: part.objectId, part: part.id }); return; }
      const target = this.splitTarget();
      if (!target || !(toObjects ? target.split.objects : target.split.parts)) return;
      this.commit({ op: toObjects ? 'split_objects' : 'split_parts', object: target.id }, { select: toObjects });
    },

    // The one part selected, when it can be split ("Split" of a part:
    // `ModelVolume::is_splittable`), else null.
    splitPart() {
      const items = this.partItems();
      return items.length === 1 && items[0].part.split ? items[0] : null;
    },

    arrange(all) {
      if (!this.editing() || !this.data() || !this.data().placings.length) return;
      this.commit(all ? { op: 'arrange', all: true } : { op: 'arrange' });
    },

    remove() {
      if (this.partMode()) {
        const items = this.partItems();
        if (items.length) this.commit({ op: 'part_delete', object: items[0].objectId, parts: items.map((item) => item.id) });
        return;
      }
      const ids = [...this.stage.selection()];
      if (ids.length) this.commit({ op: 'delete', copies: ids });
    },

    clone(ids, paste = false) {
      const data = this.data();
      if (!data) return;
      const there = new Set(data.placings.map((placing) => placing.id));
      const copies = ids.filter((id) => there.has(id));
      if (!copies.length) return;
      const project = this.host.project();
      // Pasted, they go on the plate shown; cloned, beside what they are of.
      const args = paste ? { name: 'Paste From Clipboard', plate: project.plate ?? undefined } : {};
      this.commit({ op: 'clone', copies, ...args }, { select: true });
    },

    // Whether a tool can be opened for what is selected. "Lay on face" is
    // for a whole object (GLGizmoFlatten.cpp `is_selection_supported`).
    usable(tool, chosen = this.chosen()) {
      return Boolean(chosen) && (!TOOLS[tool].one || (chosen.copies.length === 1 && !this.partMode()));
    },

    setTool(tool) {
      if (tool && !this.usable(tool)) return;
      this.tool = tool;
      this.hot = null;
      this.fields.of = '';
      this.refresh();
      this.stage.frame();
    },

    // Escape closes the tool that is open before it lets go of what is
    // selected. True when it did something.
    escape() {
      if (this.held) {
        this.revert(this.held);
        this.held.cancelled = true;
        this.stage.frame();
        return true;
      }
      if (this.tool) {
        this.setTool(null);
        return true;
      }
      return false;
    },

    // The menu of what is selected (GUI_Factories.cpp, the object menu).
    menu(x, y) {
      const data = this.data();
      const selection = this.stage.selection();
      if (!data || !selection.size) return;
      const copies = data.placings.filter((placing) => selection.has(placing.id));
      const printable = copies.every((copy) => copy.printable);
      // Plater.cpp `can_mirror`: what is selected is of one instance.
      const one = copies.length === 1 ? copies[0] : null;
      const mirror = (axis) => () => this.commit({ op: 'mirror', copies: [one.id], axis });
      const target = this.splitTarget();
      // "Add part", "Add negative part" and the rest (GUI_Factories.cpp
      // `append_menu_items_add_volume`), for one object: from a file
      // ("Load...") or a cube. The other shapes wait for the engine to
      // make them.
      const object = one ? data.objects[one.object] : null;
      const adding = [['Add part', 'part'], ['Add negative part', 'negative'], ['Add modifier', 'modifier'], ['Add support blocker', 'support_blocker'], ['Add support enforcer', 'support_enforcer']]
        .map(([label, kind]) => [label, null, {
          disabled: !object,
          items: [
            ['Load...', () => this.addPart(object, one, kind)],
            null,
            ['Cube', () => this.commit({ op: 'part_add', object: object.id, copy: one.id, kind, shape: 'Cube' }, { part: true }), { disabled: !this.host.settled() || !this.host.settled().printer }],
          ],
        }]);
      const items = [
        ['Split', null, {
          disabled: !target || !target.split.objects,
          title: 'Split the selected object',
          items: [
            ['To objects', () => this.split(true), { disabled: !target || !target.split.objects, title: 'Split the selected object into multiple objects' }],
            ['To parts', () => this.split(false), { disabled: !target || !target.split.parts, title: 'Split the selected object into multiple parts' }],
          ],
        }],
        ['Mirror', null, {
          disabled: !one,
          title: 'Mirror object',
          items: [
            ['Along X axis', mirror('x'), { title: 'Mirror along the X axis' }],
            ['Along Y axis', mirror('y'), { title: 'Mirror along the Y axis' }],
            ['Along Z axis', mirror('z'), { title: 'Mirror along the Z axis' }],
          ],
        }],
        ['Delete Del', () => this.remove()],
        null,
        ...adding,
        // GUI_Factories.cpp `append_menu_item_layers_editing`, and "Edit
        // Process Settings" (`append_menu_item_edit_process`): objsettings.js.
        [window.ObjectSettings.WORDS.heightRange, () => window.ObjectSettings.heightRange(object.id, copies.filter((copy) => copy.object === one.object).map((copy) => copy.id)), { disabled: !object }],
        null,
        ['Clone Ctrl+K', () => this.clone([...selection])],
        ['Printable', () => this.commit({ op: 'printable', copies: [...selection], value: !printable }), { checked: printable }],
        [window.ObjectSettings.WORDS.editProcess, () => window.ObjectSettings.editProcess(), { title: window.ObjectSettings.WORDS.editProcess }],
      ];
      if (copies.length === 1 && GL.objects) items.push(['Rename', () => GL.objects.rename(data.objects[copies[0].object].id)]);
      this.host.menu(x, y, items);
    },

    // "Load..." under "Add part" and the rest: a file's geometry as a part
    // of the object (GUI_ObjectList.cpp `load_subobject`).
    async addPart(object, copy, kind) {
      const path = await this.host.pick('model', this.host.remembered.get('model') || '');
      if (!path) return;
      if (await this.commit({ op: 'part_add', object: object.id, copy: copy.id, kind, path }, { part: true })) this.host.remembered.set('model', path);
    },

    // The menu of the parts selected (GUI_Factories.cpp
    // `create_bbl_part_menu`, the one the object list shows, and
    // `part_menu`). What the model cannot do yet is left out: "Edit
    // text", "Repair", "Simplify", "Separate", "Reload from disk" and the
    // rest (UI_PARITY.md, section 13).
    partMenu(x, y) {
      const items = this.partItems();
      const data = this.data();
      if (!items.length || !data) return;
      const object = items[0].objectId;
      const ids = items.map((item) => item.id);
      const box = this.partBox();
      const kinds = new Set(items.map((item) => item.part.kind));
      const project = this.host.project();
      const shift = (by) => () => this.place('Move Object', items.map((item) => ({ ...item, matrix: M.multiply(M.translation(...by), item.matrix) })));
      // The middle of the plate shown (`PartPlate::get_center_origin`).
      const plate = this.stage.scene.plate(project.plate);
      const middle = plate && plate.box ? [(plate.box[0] + plate.box[2]) / 2, (plate.box[1] + plate.box[3]) / 2] : null;
      const centre = box ? M.centre(box) : null;
      const centred = !middle || !centre || (Math.abs(centre[0] - middle[0]) < 1e-6 && Math.abs(centre[1] - middle[1]) < 1e-6);
      const mirror = (axis) => () => this.commit({ op: 'part_mirror', copy: items[0].copy.id, parts: ids, axis });
      const part = this.splitPart();
      // "Change type": the reference asks in a dialog ("Choose part type");
      // here the same choices are a sub-menu.
      const types = [['Part', 'part'], ['Negative Part', 'negative'], ['Modifier', 'modifier'], ['Support Blocker', 'support_blocker'], ['Support Enforcer', 'support_enforcer'], ['Precise Seam', 'precise_seam']];
      const filaments = this.filamentCount();
      const own = items.length === 1 ? items[0].part.own : undefined;
      const menu = [
        ['Delete Del', () => this.remove(), { title: 'Delete the selected object' }],
        ['Center', shift(centre && middle ? [middle[0] - centre[0], middle[1] - centre[1], 0] : [0, 0, 0]), { disabled: centred }],
        ['Drop', shift([0, 0, box ? -box[0][2] : 0]), { disabled: !box || Math.abs(box[0][2]) <= 0.001 }],
        ['Mirror', null, {
          title: 'Mirror object',
          items: [
            ['Along X axis', mirror('x'), { title: 'Mirror along the X axis' }],
            ['Along Y axis', mirror('y'), { title: 'Mirror along the Y axis' }],
            ['Along Z axis', mirror('z'), { title: 'Mirror along the Z axis' }],
          ],
        }],
        ['Split', null, {
          disabled: !part,
          title: 'Split the selected object',
          items: [
            ['To objects', null, { disabled: true, title: 'Split the selected object into multiple objects' }],
            ['To parts', () => this.split(false), { disabled: !part, title: 'Split the selected object into multiple parts' }],
          ],
        }],
        null,
        ['Change type', null, {
          disabled: items.length !== 1,
          items: types.map(([label, kind]) => [label, () => this.commit({ op: 'part_type', object, parts: ids, kind }), { checked: kinds.size === 1 && kinds.has(kind) }]),
        }],
      ];
      // "Change Filament", for parts and modifiers, when there is more
      // than one filament (`append_menu_item_change_filament`).
      if (filaments > 1 && [...kinds].every((kind) => kind === 'part' || kind === 'modifier')) {
        const names = (project.choices && project.choices.filaments) || [];
        const set = (filament) => () => this.commit({ op: 'part_filament', object, parts: ids, filament });
        menu.push([items.length === 1 ? 'Change Filament' : 'Set Filament for selected items', null, {
          title: 'Change Filament',
          items: [
            ['Default', set(0), { checked: own === null }],
            ...Array.from({ length: filaments }, (_, at) => [names[at] || `Filament ${at + 1}`, set(at + 1), { checked: own === at + 1 }]),
          ],
        }]);
      }
      this.host.menu(x, y, menu);
    },

    // How many filaments the project has to choose from.
    filamentCount() {
      const project = this.host.project();
      return Math.max(project.filament_count || 1, (project.colours || []).length, ((project.choices || {}).filaments || []).length, 1);
    },

    // The menu of a plate (GUI_Factories.cpp `create_plate_menu`): see
    // plates.js.
    plateMenu(x, y, plate) {
      if (GL.plateTools) GL.plateTools.menu(x, y, plate);
    },

    key(event) {
      if (!this.stage.active || !this.editing() || document.querySelector('dialog[open]')) return;
      if (event.target.matches('input, select, textarea') || event.altKey) return;
      const command = event.ctrlKey || event.metaKey;
      const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
      const take = (act) => { event.preventDefault(); act(); };
      if (command) {
        if (key === 'z' && !event.shiftKey) take(() => this.step('undo'));
        else if (key === 'y' || (key === 'z' && event.shiftKey)) take(() => this.step('redo'));
        else if (event.shiftKey) return;
        else if (key === 'i') take(() => this.add());
        // Copying, pasting and cloning are of objects; a part's own wait
        // for the model to copy parts.
        else if (this.partMode()) return;
        else if (key === 'c' && this.stage.selection().size) take(() => { this.copied = [...this.stage.selection()]; });
        else if (key === 'v' && this.copied.length) take(() => this.clone(this.copied, true));
        else if (key === 'k' && this.stage.selection().size) take(() => this.clone([...this.stage.selection()]));
        return;
      }
      if (key === 'Delete' || key === 'Backspace') { if (this.stage.selection().size) take(() => this.remove()); return; }
      if (key === 'a') { take(() => this.arrange(!event.shiftKey)); return; }
      for (const [tool, { key: letter }] of Object.entries(TOOLS)) {
        if (key === letter && !event.shiftKey) { take(() => this.setTool(this.tool === tool ? null : tool)); return; }
      }
      // The arrows move what is selected: 10 mm, or 1 mm with Shift
      // (KBShortcutsDialog.cpp, "Move selection 10 mm in positive X direction").
      const arrows = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, 1], ArrowDown: [0, -1] };
      if (arrows[key] && this.chosen()) {
        const by = event.shiftKey ? 1 : 10;
        take(() => this.nudge(arrows[key][0] * by, arrows[key][1] * by));
      }
    },

    // Moves what is selected across the bed by this much. A key pressed
    // again while the last move is on its way to the session adds to the
    // next one, so none is lost.
    async nudge(x, y) {
      this.owed = [(this.owed ? this.owed[0] : 0) + x, (this.owed ? this.owed[1] : 0) + y];
      while (this.owed && !this.busy) {
        const chosen = this.chosen();
        const shift = M.translation(this.owed[0], this.owed[1], 0);
        this.owed = null;
        if (!chosen) return;
        await this.place('Tool Move', chosen.copies.map((copy) => ({ ...copy, matrix: M.multiply(shift, copy.matrix) })));
      }
    },

    // ---- keeping the page in step ----

    // The stage shows a project, or the same one anew.
    shown() {
      this.refresh();
    },

    sceneChanged() {
      if (this.tool && !this.usable(this.tool)) this.tool = null;
      this.refresh();
      if (GL.objects) GL.objects.render();
    },

    selectionChanged() {
      if (this.tool && !this.usable(this.tool)) this.tool = null;
      this.fields.of = '';
      this.refresh();
      if (GL.objects) GL.objects.mark();
    },

    // The toolbar, the number fields and the warning, as things are now.
    refresh() {
      if (GL.plateTools) GL.plateTools.refresh();
      if (!this.bar) return;
      const project = this.host.project();
      const on = this.editing();
      this.bar.hidden = !on;
      this.warning.hidden = true;
      if (!on) {
        this.panel.hidden = true;
        return;
      }
      const data = this.data();
      const chosen = this.chosen();
      const history = project.edit || {};
      const b = this.buttons;
      const printer = Boolean(this.host.settled() && this.host.settled().printer);
      b.add.disabled = this.busy;
      b.addPlate.disabled = this.busy || project.empty || !printer;
      b.arrange.disabled = this.busy || !data || !data.placings.length || !printer;
      const target = this.splitTarget();
      b.splitObjects.disabled = this.busy || !target || !target.split.objects;
      b.splitParts.disabled = this.busy || !((target && target.split.parts) || this.splitPart());
      b.undo.disabled = this.busy || !history.undo;
      b.redo.disabled = this.busy || !history.redo;
      b.undo.title = `Undo [Ctrl+Z]${history.undo ? `\n${history.undo}` : ''}`;
      b.redo.title = `Redo [Ctrl+Y]${history.redo ? `\n${history.redo}` : ''}`;
      for (const tool of Object.keys(TOOLS)) {
        b[tool].disabled = !this.usable(tool, chosen);
        b[tool].setAttribute('aria-pressed', String(this.tool === tool));
      }
      if (this.tool === 'flatten' && chosen && chosen.copies.length === 1) this.wantFaces(chosen.copies[0]);
      // A plate with an object over its edge cannot be printed
      // (Plater.cpp `validate_current_plate`).
      const over = data && data.placings.some((placing) => placing.over && placing.plate === project.plate);
      if (over) {
        this.warning.textContent = 'An object is laid over the boundary of plate or exceeds the height limit.\nPlease solve the problem by moving it totally on or off the plate, and confirming that the height is within the build volume.';
        this.warning.hidden = false;
      }
      this.showFields();
    },

    // ---- the handles ----

    scale() {
      // Millimetres to a pixel where the camera looks.
      return 1 / this.stage.camera.zoom;
    },

    // Where a point of the scene is in the view, in pixels.
    toView(frame, point) {
      const at = M.point(M.multiply(frame.projection, frame.view), point);
      const [width, height] = this.stage.renderer.css;
      return [(at[0] * 0.5 + 0.5) * width, (1 - (at[1] * 0.5 + 0.5)) * height];
    },

    // The handles of the tool that is open, for what is selected: each
    // with what it is (`kind`), its id, and where it is.
    handles(chosen) {
      const mm = this.scale();
      if (this.tool === 'move') {
        // An arrow along X and one along Y. An object is not lifted off
        // the bed and the engine does not print one sunk under it, so
        // there is none along Z; a part has one, to move it up or down
        // within its object.
        return (this.partMode() ? [0, 1, 2] : [0, 1]).map((axis) => ({
          kind: 'move', id: `move-${axis}`, axis, from: chosen.centre, to: M.along(chosen.centre, UNIT[axis], ARROW * mm),
        }));
      }
      if (this.tool === 'rotate') {
        const held = this.held && this.held.kind === 'rotate' ? this.held : null;
        const centre = held ? held.centre : chosen.pivot;
        const radius = held ? held.radius : RING_OFFSET + chosen.radius;
        return [0, 1, 2].map((axis) => ({ kind: 'rotate', id: `rotate-${axis}`, axis, centre, radius }));
      }
      if (this.tool === 'scale') {
        const one = chosen.copies.length === 1 ? chosen.copies[0] : null;
        const local = one ? this.localBox(one) : null;
        if (one && local) {
          // One object: along each of its own axes, from the middle of
          // its box, and at the foot of the box's four corners for all
          // three at once (GLGizmoScale.cpp `update_grabbers_data`).
          const m = one.matrix;
          const c = M.centre(local);
          const h = [0, 1, 2].map((axis) => (local[1][axis] - local[0][axis]) / 2);
          const at = (x, y, z) => M.point(m, [c[0] + x * h[0], c[1] + y * h[1], c[2] + z * h[2]]);
          const pivot = at(0, 0, 0);
          return [
            ...[[0, -1, 0, 0], [0, 1, 0, 0], [1, 0, -1, 0], [1, 0, 1, 0]].map(([axis, x, y]) => ({ kind: 'scale', id: `scale-${axis}${x + y}`, axis, at: at(x, y, -1), pivot })),
            { kind: 'scale', id: 'scale-2', axis: 2, at: at(0, 0, 1), pivot },
            ...[[-1, -1], [1, -1], [1, 1], [-1, 1]].map(([x, y]) => ({ kind: 'scale', id: `scale-all${x}${y}`, axis: null, at: at(x, y, -1), pivot })),
          ];
        }
        // Several: all of them at once, about their middle.
        const [low, high] = chosen.box;
        return [[low[0], low[1]], [high[0], low[1]], [high[0], high[1]], [low[0], high[1]]].map(([x, y], at) => ({
          kind: 'scale', id: `scale-all${at}`, axis: null, at: [x, y, low[2]], pivot: chosen.centre,
        }));
      }
      return [];
    },

    // The handle at a pixel of the view, or null.
    handleAt(x, y, chosen) {
      if (this.tool === 'flatten') return this.faceAt(x, y, chosen);
      const frame = this.stage.matrices();
      let [best, least] = [null, GRIP];
      const near = (a, b) => {
        const [dx, dy] = [b[0] - a[0], b[1] - a[1]];
        const length = dx * dx + dy * dy;
        const t = length ? Math.max(0, Math.min(1, ((x - a[0]) * dx + (y - a[1]) * dy) / length)) : 0;
        return Math.hypot(x - a[0] - dx * t, y - a[1] - dy * t);
      };
      for (const handle of this.handles(chosen)) {
        let far = Infinity;
        if (handle.kind === 'move') {
          far = near(this.toView(frame, handle.from), this.toView(frame, handle.to));
        } else if (handle.kind === 'rotate') {
          const [u, w] = [UNIT[(handle.axis + 1) % 3], UNIT[(handle.axis + 2) % 3]];
          let before = null;
          for (let step = 0; step <= 72; step += 1) {
            const angle = (step * Math.PI) / 36;
            const point = this.toView(frame, M.along(M.along(handle.centre, u, Math.cos(angle) * handle.radius), w, Math.sin(angle) * handle.radius));
            if (before) far = Math.min(far, near(before, point));
            before = point;
          }
        } else {
          const point = this.toView(frame, handle.at);
          far = Math.hypot(x - point[0], y - point[1]) - KNOB / 2;
        }
        if (far < least) [best, least] = [handle, far];
      }
      return best;
    },

    hover(x, y) {
      const chosen = this.tool && !this.busy ? this.chosen() : null;
      const handle = chosen ? this.handleAt(x, y, chosen) : null;
      const hot = handle ? handle.id : null;
      if (hot === this.hot) return false;
      this.hot = hot;
      this.stage.canvas.classList.toggle('on-handle', Boolean(hot));
      return true;
    },

    // ---- dragging ----

    // What the left button takes hold of at a pixel: a handle of the tool
    // that is open, else the object there. Null where there is neither,
    // and the drag turns the view.
    grab(x, y, event) {
      if (this.busy || !this.data()) return null;
      const chosen = this.tool ? this.chosen() : null;
      const handle = chosen ? this.handleAt(x, y, chosen) : null;
      if (handle && handle.kind === 'face') {
        // A face is laid on the bed as the button goes down, as the
        // reference does (GLGizmoFlatten.cpp `on_mouse`); nothing is dragged.
        this.layOn(chosen.copies[0], handle.face);
        return { kind: 'face', cancelled: true, x, y };
      }
      if (handle) return { kind: handle.kind, handle, x, y };
      // With Ctrl or Shift a click adds to what is selected; it is no drag.
      if (event.ctrlKey || event.metaKey || event.shiftKey) return null;
      const frame = this.stage.matrices();
      const hit = this.stage.scene.pick(this.stage.camera, frame, x, y, { plates: false });
      if (!hit || hit.placing === undefined) return null;
      const placing = this.data().placings[hit.placing];
      // With Alt the part under the pointer is taken, and moved within
      // its object (GLCanvas3D.cpp `on_mouse`, the volume mode).
      if (event.altKey && (this.data().objects[placing.object].part_list || []).length > 1) {
        const at = this.stage.scene.pickPart(frame, x, y, hit.placing);
        const part = at === null ? null : this.data().objects[placing.object].part_list[at];
        if (part) return { kind: 'object', id: placing.id, part: part.id, x, y };
      }
      return { kind: 'object', id: placing.id, x, y };
    },

    // A drag begins: where everything selected stands is noted.
    begin(grab) {
      if (grab.kind === 'object' && grab.part !== undefined) {
        if (!this.stage.selection().has(grab.id) || !this.stage.parts().has(grab.part)) this.stage.selectParts(grab.id, [grab.part]);
      } else if (grab.kind === 'object' && (!this.stage.selection().has(grab.id) || this.partMode())) {
        this.stage.select([grab.id]);
      }
      const chosen = this.chosen();
      if (!chosen) return false;
      const camera = this.stage.camera;
      grab.starts = chosen.copies.map((copy) => ({ copy, matrix: copy.matrix.slice(), box: copyBox(copy.box), local: this.localBox(copy) }));
      grab.centre = chosen.pivot;
      grab.radius = RING_OFFSET + chosen.radius;
      if (grab.kind === 'object') {
        // Across the bed, at the height of the middle of what is held.
        const held = chosen.copies.find((copy) => copy.id === (grab.part !== undefined ? grab.part : grab.id)) || chosen.copies[0];
        grab.height = held.box ? (held.box[0][2] + held.box[1][2]) / 2 : 0;
        grab.from = camera.onGround(grab.x, grab.y, grab.height);
        if (!grab.from) return false;
      } else if (grab.kind === 'move') {
        grab.from = this.along(grab.handle.from, UNIT[grab.handle.axis], grab.x, grab.y);
      } else if (grab.kind === 'rotate') {
        const turn = this.around(grab.centre, grab.handle.axis, grab.x, grab.y);
        if (!turn) return false;
        grab.from = turn.angle;
      } else if (grab.kind === 'scale') {
        grab.pivot = grab.handle.pivot;
        grab.at = grab.handle.at;
        grab.sizes = grab.starts.length === 1 ? turnOf(grab.starts[0].matrix).sizes : null;
      }
      grab.begun = true;
      this.held = grab;
      return true;
    },

    // How far along a line (from `origin`, along the unit `axis`) the
    // point nearest the line of sight through a pixel is.
    along(origin, axis, x, y) {
      const ray = this.stage.camera.ray(x, y);
      const w = M.sub(origin, ray.origin);
      const b = M.dot(axis, ray.direction);
      const under = 1 - b * b;
      if (under < 1e-6) return 0;
      return (b * M.dot(ray.direction, w) - M.dot(axis, w)) / under;
    },

    // Where the line of sight through a pixel meets the plane through
    // `centre` square to an axis: the angle about the axis and how far out.
    around(centre, axis, x, y) {
      const ray = this.stage.camera.ray(x, y);
      const normal = UNIT[axis];
      const toward = M.dot(ray.direction, normal);
      if (Math.abs(toward) < 1e-4) return null;
      const point = M.along(ray.origin, ray.direction, M.dot(M.sub(centre, ray.origin), normal) / toward);
      const out = M.sub(point, centre);
      const [u, w] = [UNIT[(axis + 1) % 3], UNIT[(axis + 2) % 3]];
      return { angle: Math.atan2(M.dot(out, w), M.dot(out, u)), far: M.length(out) };
    },

    drag(grab, x, y, event) {
      if (grab.cancelled) return;
      if (!grab.begun && !this.begin(grab)) { grab.cancelled = true; return; }
      const set = (matrixOf) => {
        for (const start of grab.starts) {
          start.copy.matrix = matrixOf(start);
          // The box round it while the drag lasts: right for a move, near
          // enough for a turn; the session gives the true one after.
          start.copy.box = start.local ? boxUnder(start.copy.matrix, start.local) : start.box;
        }
      };
      if (grab.kind === 'object' || grab.kind === 'move') {
        let by;
        if (grab.kind === 'object') {
          const at = this.stage.camera.onGround(x, y, grab.height);
          if (!at) return;
          by = [at[0] - grab.from[0], at[1] - grab.from[1], 0];
        } else {
          let far = this.along(grab.handle.from, UNIT[grab.handle.axis], x, y) - grab.from;
          if (event.shiftKey) far = MOVE_STEP * Math.round(far / MOVE_STEP);
          by = M.scale(UNIT[grab.handle.axis], far);
        }
        const shift = M.translation(...by);
        for (const start of grab.starts) {
          start.copy.matrix = M.multiply(shift, start.matrix);
          start.copy.box = start.box ? [M.add(start.box[0], by), M.add(start.box[1], by)] : null;
        }
        grab.said = `${round(by[0])}, ${round(by[1])} mm`;
      } else if (grab.kind === 'rotate') {
        const turn = this.around(grab.centre, grab.handle.axis, x, y);
        if (!turn) return;
        let angle = turn.angle - grab.from;
        // GLGizmoRotate.cpp `on_dragging`: near the middle the turn goes
        // by eighths, just outside the ring by 5 degrees.
        const [r, eighth, fine] = [grab.radius, Math.PI / 4, Math.PI / 36];
        if (turn.far >= r / 3 && turn.far <= (2 * r) / 3) angle = eighth * Math.round(angle / eighth);
        else if (turn.far >= r && turn.far <= r * 1.1) angle = fine * Math.round(angle / fine);
        angle = Math.atan2(Math.sin(angle), Math.cos(angle));
        const spin = about(grab.centre, linear(M.rows.about(UNIT[grab.handle.axis], angle)));
        set((start) => M.multiply(spin, start.matrix));
        grab.angle = angle;
        grab.said = `${round(degrees(angle), 1)}°`;
      } else if (grab.kind === 'scale') {
        // GLGizmoScale.cpp `calc_ratio`: how far the pointer has gone from
        // the handle along the line from the middle through the handle.
        const ray = this.stage.camera.ray(x, y);
        const facing = this.stage.camera.forward;
        const toward = M.dot(ray.direction, facing);
        if (Math.abs(toward) < 1e-6) return;
        const point = M.along(ray.origin, ray.direction, M.dot(M.sub(grab.at, ray.origin), facing) / toward);
        const out = M.sub(grab.at, grab.pivot);
        const length = M.length(out);
        if (!length) return;
        let ratio = (length + M.dot(M.sub(point, grab.at), M.scale(out, 1 / length))) / length;
        if (event.shiftKey) ratio = SCALE_STEP * Math.round(ratio / SCALE_STEP);
        ratio = Math.max(ratio, 0.01);
        const axis = grab.handle.axis;
        if (grab.starts.length === 1 && grab.starts[0].local) {
          set((start) => this.sized(start.matrix, start.local, axis === null || this.fields.uniform ? [ratio, ratio, ratio] : UNIT[axis].map((on) => (on ? ratio : 1))));
        } else {
          const grow = about(grab.pivot, M.scaling(ratio, ratio, ratio));
          set((start) => M.multiply(grow, start.matrix));
        }
        grab.said = `${round(ratio * 100, 1)}%`;
      }
      this.showMoving(grab.starts.map((start) => start.copy));
      this.showFields(true);
    },

    // A matrix with each of the object's own axes `by` times as long, the
    // middle of the object's box staying where it is.
    sized(matrix, local, by) {
      const out = matrix.slice();
      for (let c = 0; c < 3; c += 1) for (let r = 0; r < 3; r += 1) out[4 * c + r] *= by[c];
      const middle = M.centre(local);
      const [was, now] = [M.point(matrix, middle), M.point(out, middle)];
      for (let r = 0; r < 3; r += 1) out[12 + r] += was[r] - now[r];
      return out;
    },

    revert(grab) {
      for (const start of grab.starts || []) {
        start.copy.matrix = start.matrix;
        start.copy.box = start.box;
      }
      this.stage.scene.moving = null;
    },

    // The button is let go: the edit is made, or with `done` false
    // everything goes back where it was.
    async drop(grab, done) {
      this.held = null;
      if (!grab.begun || grab.cancelled) { this.stage.frame(); return; }
      if (!done) {
        this.revert(grab);
        this.refresh();
        this.stage.frame();
        return;
      }
      const name = { object: 'Move Object', move: 'Gizmo-Move', rotate: 'Gizmo-Rotate', scale: 'Gizmo-Scale' }[grab.kind];
      if (grab.kind === 'rotate' && grab.angle) this.fields.relative[grab.handle.axis] += degrees(grab.angle);
      const kept = this.fields.of;
      const made = await this.place(name, grab.starts.map((start) => start.copy));
      this.stage.scene.moving = null;
      if (!made) this.revert(grab);
      else if (grab.kind === 'rotate') this.fields.of = kept;
      this.refresh();
      this.stage.frame();
    },

    // ---- drawing ----

    // What the handles are made of, once: a ring, a cone, a cube.
    parts() {
      if (this.shapes) return this.shapes;
      const r = this.stage.renderer;
      const ring = [];
      for (let step = 0; step < 72; step += 1) {
        const [a, b] = [(step * Math.PI) / 36, ((step + 1) * Math.PI) / 36];
        ring.push(Math.cos(a), Math.sin(a), 0, Math.cos(b), Math.sin(b), 0);
      }
      const cone = [];
      for (let side = 0; side < 16; side += 1) {
        const [a, b] = [(side * Math.PI) / 8, ((side + 1) * Math.PI) / 8];
        const [ax, ay, bx, by] = [Math.cos(a) * 0.32, Math.sin(a) * 0.32, Math.cos(b) * 0.32, Math.sin(b) * 0.32];
        cone.push(ax, ay, 0, bx, by, 0, 0, 0, 1, 0, 0, 0, bx, by, 0, ax, ay, 0);
      }
      const cube = [];
      for (let axis = 0; axis < 3; axis += 1) {
        for (const side of [-0.5, 0.5]) {
          const [u, v] = [(axis + 1) % 3, (axis + 2) % 3];
          const corner = (a, b) => { const p = [0, 0, 0]; p[axis] = side; p[u] = a; p[v] = b; return p; };
          cube.push(...corner(-0.5, -0.5), ...corner(0.5, -0.5), ...corner(0.5, 0.5), ...corner(-0.5, -0.5), ...corner(0.5, 0.5), ...corner(-0.5, 0.5));
        }
      }
      this.shapes = { ring: r.lines(ring), cone: r.triangles(cone), cube: r.triangles(cube), line: r.lines([0, 0, 0, 0, 0, 1]) };
      return this.shapes;
    },

    // The handles of the tool that is open, over everything else.
    draw(frame) {
      // The plate's own tools stand beside the plate shown.
      if (GL.plateTools) GL.plateTools.place(frame);
      const chosen = this.tool && this.editing() ? this.chosen() : null;
      if (!chosen) return;
      const r = this.stage.renderer;
      const gl = r.gl;
      if (this.tool === 'flatten') { this.drawFaces(frame, chosen); return; }
      const parts = this.parts();
      const matrix = M.multiply(frame.projection, frame.view);
      const mm = this.scale();
      const colour = (handle, plain) => (handle.id === this.hot || (this.held && this.held.handle === handle) || (this.held && this.held.handle && this.held.handle.id === handle.id) ? HOT : plain);
      gl.disable(gl.DEPTH_TEST);
      r.blend(true);
      for (const handle of this.handles(chosen)) {
        if (handle.kind === 'move') {
          const tint = colour(handle, AXES[handle.axis]);
          const length = ARROW * mm;
          r.drawLines(parts.line, M.chain(matrix, M.translation(...handle.from), TURN[handle.axis], M.scaling(1, 1, length - TIP * mm)), tint, 3);
          r.drawFlat(parts.cone, M.chain(matrix, M.translation(...M.along(handle.from, UNIT[handle.axis], length - TIP * mm)), TURN[handle.axis], M.scaling(TIP * mm, TIP * mm, TIP * mm)), tint);
        } else if (handle.kind === 'rotate') {
          const hot = colour(handle, null);
          // The ring of another axis makes way while one is turned.
          if (this.held && this.held.kind === 'rotate' && !hot) continue;
          r.drawLines(parts.ring, M.chain(matrix, M.translation(...handle.centre), TURN[handle.axis], M.scaling(handle.radius, handle.radius, handle.radius)), hot || AXES[handle.axis], hot ? 3 : 2);
        } else {
          const size = KNOB * mm;
          r.drawFlat(parts.cube, M.chain(matrix, M.translation(...handle.at), M.scaling(size, size, size)), colour(handle, handle.axis === null ? PLAIN : AXES[handle.axis]));
        }
      }
      r.blend(false);
      gl.enable(gl.DEPTH_TEST);
    },

    // ---- Lay on face ----

    // The faces of the copy selected, asked for once for each place and
    // shape it has (`scene_faces`), and made ready to draw.
    async wantFaces(copy) {
      const data = this.data();
      if (!data) return;
      const key = `${data.project}|${copy.id}|${data.objects[copy.object].mesh}|${copy.matrix.join(',')}`;
      if (this.faces && this.faces.key === key) return;
      this.dropFaces();
      const held = { key, list: null, meshes: [] };
      this.faces = held;
      try {
        const answer = await this.host.call('scene_faces', { project: data.project, copy: copy.id });
        if (this.faces !== held) return;
        const r = this.stage.renderer;
        held.list = answer.faces;
        // Each outline is convex, so a fan of triangles fills it.
        held.meshes = answer.faces.map((face) => {
          const fan = [];
          for (let at = 1; at + 1 < face.drawn.length; at += 1) fan.push(...face.drawn[0], ...face.drawn[at], ...face.drawn[at + 1]);
          return r.triangles(fan);
        });
      } catch (error) {
        if (this.faces !== held) return;
        held.list = [];
        this.host.say(error.message, true);
      }
      this.showFields();
      this.stage.frame();
    },

    dropFaces() {
      if (this.faces) for (const mesh of this.faces.meshes) this.stage.renderer.drop(mesh);
      this.faces = null;
    },

    // The face under a pixel, as a handle; null where there is none.
    faceAt(x, y, chosen) {
      const faces = this.faces;
      if (!faces || !faces.meshes.length) return null;
      const r = this.stage.renderer;
      const frame = this.stage.matrices();
      const matrix = chosen.copies[0].matrix;
      // Drawn alone and over everything, as the reference's gizmo picks its
      // faces in front of the object (`set_raycaster_gizmos_on_top`).
      const hit = r.pick(x, y, frame.projection, (projection) => {
        const m = M.multiply(M.multiply(projection, frame.view), matrix);
        faces.meshes.forEach((mesh, at) => r.drawFlat(mesh, m, GL.Renderer.pickColour(at + 1)));
      });
      if (!hit || hit > faces.list.length) return null;
      return { kind: 'face', id: `face-${hit - 1}`, face: faces.list[hit - 1] };
    },

    drawFaces(frame, chosen) {
      const faces = this.faces;
      if (!faces || !faces.meshes.length) return;
      const r = this.stage.renderer;
      const gl = r.gl;
      const m = M.multiply(M.multiply(frame.projection, frame.view), chosen.copies[0].matrix);
      // Over the object, each face hiding those behind it.
      gl.clear(gl.DEPTH_BUFFER_BIT);
      r.blend(true);
      faces.meshes.forEach((mesh, at) => r.drawFlat(mesh, m, this.hot === `face-${at}` ? FACE_HOT : FACE));
      r.blend(false);
    },

    layOn(copy, face) {
      this.hot = null;
      this.commit({ op: 'lay_on_face', copy: copy.id, normal: face.normal });
    },

    // ---- the number fields ----

    // What the fields say of what is selected.
    measured(chosen) {
      const one = chosen.copies.length === 1 ? chosen.copies[0] : null;
      const local = one ? this.localBox(one) : null;
      const world = [0, 1, 2].map((axis) => chosen.box[1][axis] - chosen.box[0][axis]);
      const out = { one, local, position: [chosen.centre[0], chosen.centre[1], chosen.box[0][2]], world, scale: [100, 100, 100], size: world, angles: null, turned: false };
      if (one) {
        // A part's scale and turn are its own, within its object, as the
        // reference gives a volume's (GizmoObjectManipulation.cpp
        // `update_settings_value`); its size is as it is printed.
        const { sizes, rows } = turnOf(one.P || one.matrix);
        out.scale = sizes.map((size) => size * 100);
        out.angles = anglesOf(rows).map(degrees);
        out.turned = out.angles.some((angle) => Math.abs(angle) > 1e-4);
        const printed = turnOf(one.matrix).sizes;
        if (local && this.fields.frame === 'object') out.size = [0, 1, 2].map((axis) => (local[1][axis] - local[0][axis]) * printed[axis]);
      }
      return out;
    },

    // The number fields of the tool that is open
    // (GizmoObjectManipulation.cpp `do_render_move_window`,
    // `do_render_rotate_window`, `do_render_scale_input_window`). `live`:
    // only the numbers are set, for a drag that is going on.
    showFields(live = false) {
      const chosen = this.tool && this.editing() ? this.chosen() : null;
      this.panel.hidden = !chosen;
      if (!chosen) return;
      const { el } = this.host;
      const of = `${this.tool}|${[...this.stage.selection()].join(',')}|${[...this.stage.parts()].join(',')}|${this.fields.frame}`;
      if (this.fields.of !== of) {
        this.fields.of = of;
        this.fields.relative = [0, 0, 0];
        live = false;
      }
      if (this.tool === 'flatten') {
        // GLGizmoFlatten.cpp `on_render_input_window`, its convex hull mode.
        const faces = this.faces && this.faces.list;
        const hint = !faces ? '…' : faces.length ? 'Click a highlighted face to lay the object on it.' : 'No resting faces found on the convex hull.';
        this.panel.replaceChildren(el('h3', { textContent: 'Lay on face' }), el('p', { className: 'edit-hint', textContent: hint }));
        return;
      }
      const m = this.measured(chosen);
      const row = (label, values, { unit, key, off = [], digits = 2, min = null }) => {
        const cells = values.map((value, axis) => {
          const input = el('input', { type: 'number', step: 'any', value: String(round(value, digits)), disabled: this.busy || off.includes(axis) });
          input.dataset.field = `${key}-${axis}`;
          input.setAttribute('aria-label', `${label} ${'XYZ'[axis]}`);
          if (min !== null) input.min = String(min);
          input.addEventListener('change', () => this.typed(key, axis, Number(input.value)));
          return el('span', { className: 'unit' }, input, unit ? ` ${unit}` : '');
        });
        return [el('span', { className: 'edit-label', textContent: label }), ...cells];
      };
      if (live) {
        // Only the numbers, so that a field keeps the keys while it is typed in.
        const set = (key, values, digits = 2) => values.forEach((value, axis) => {
          const input = this.panel.querySelector(`[data-field="${key}-${axis}"]`);
          if (input && document.activeElement !== input) input.value = String(round(value, digits));
        });
        set('position', m.position);
        set('scale', m.scale);
        set('size', m.size);
        if (m.angles) set('absolute', m.angles);
        if (this.held && this.held.kind === 'rotate') set('relative', this.fields.relative.map((value, axis) => value + (axis === this.held.handle.axis ? degrees(this.held.angle || 0) : 0)));
        return;
      }
      const head = [el('span'), ...['X', 'Y', 'Z'].map((axis) => el('b', { className: `gl-${axis.toLowerCase()}`, textContent: axis }))];
      const frame = () => {
        const pick = el('select', { title: 'Coordinate system used for transform actions.' });
        pick.setAttribute('aria-label', 'Coordinate system used for transform actions.');
        pick.append(new Option('World', 'world'), new Option('Object', 'object'));
        pick.value = this.fields.frame;
        pick.disabled = !m.one;
        pick.addEventListener('change', () => { this.fields.frame = pick.value; this.fields.of = ''; this.showFields(); });
        return pick;
      };
      const reset = (tip, act, enabled) => {
        const node = el('button', { type: 'button', className: 'icon tool', title: tip, disabled: !enabled || this.busy, textContent: '↺' });
        node.setAttribute('aria-label', tip);
        node.addEventListener('click', act);
        return node;
      };
      const grid = el('div', { className: 'edit-grid' });
      let foot = [];
      if (this.tool === 'move') {
        const object = this.fields.frame === 'object' && m.one;
        grid.append(frame(), ...head.slice(1), el('span'),
          // In the object's own coordinates the fields are how far to go
          // along its own axes; Z is the lowest point, which stays on the
          // bed. A part's Z can be typed: it moves within its object.
          ...row('Position', object ? [0, 0, 0] : m.position, { unit: 'mm', key: object ? 'along' : 'position', off: this.partMode() ? [] : [2] }), el('span'));
      } else if (this.tool === 'rotate') {
        grid.append(el('span', { className: 'edit-label', textContent: 'World', title: 'Coordinate system used for transform actions.' }), ...head.slice(1), el('span'),
          ...row('Relative', this.fields.relative, { unit: '°', key: 'relative' }),
          reset('Reset current rotation to the value when open the rotation tool.', () => this.typed('relative-reset'), this.fields.relative.some((angle) => Math.abs(angle) > 1e-9)));
        if (m.angles) {
          grid.append(...row('Absolute', m.angles, { unit: '°', key: 'absolute' }),
            reset('Reset current rotation to real zeros.', () => this.typed('absolute-reset'), m.turned));
        }
      } else {
        // In the world's coordinates an object that is turned is sized
        // as a whole, as the reference has it.
        const whole = !m.one || (this.fields.frame === 'world' && m.turned);
        const uniform = el('input', { type: 'checkbox', checked: this.fields.uniform || whole, disabled: whole });
        uniform.addEventListener('change', () => { this.fields.uniform = uniform.checked; });
        grid.append(frame(), ...head.slice(1), el('span'),
          ...row('Scale', m.scale, { unit: '%', key: 'scale', min: 0.01 }),
          reset('Reset', () => this.typed('scale-reset'), m.one && m.scale.some((value) => Math.abs(value - 100) > 1e-6)),
          ...row('Size', m.size, { unit: 'mm', key: 'size', min: 0.001 }), el('span'));
        foot = [el('label', { className: 'edit-check' }, uniform, ' uniform scale')];
      }
      // GizmoObjectManipulation.cpp `update_settings_value`: one part is a
      // volume, several a group.
      const title = !this.partMode() ? 'Object Operations' : chosen.copies.length === 1 ? 'Volume Operations' : 'Group Operations';
      this.panel.replaceChildren(el('h3', { textContent: title }), grid, ...foot);
    },

    // A number typed into a field, or one of the reset buttons.
    typed(key, axis, value) {
      const chosen = this.chosen();
      if (!chosen || this.busy) return;
      const m = this.measured(chosen);
      const all = (matrixOf) => chosen.copies.map((copy) => ({ ...copy, matrix: matrixOf(copy) }));
      const spin = (rows) => about(chosen.pivot, linear(rows));
      const keep = () => { const of = this.fields.of; return () => { this.fields.of = of; this.showFields(); }; };
      if (key === 'position' || key === 'along') {
        if (!Number.isFinite(value)) { this.showFields(); return; }
        // Along the world's axis to where the number says, or that far
        // along the object's own.
        const by = key === 'position' ? M.scale(UNIT[axis], value - m.position[axis]) : M.scale(M.normalize(col(m.one.matrix, axis)), value);
        if (!this.partMode()) by[2] = 0;
        const shift = M.translation(...by);
        this.place('Set Position', all((copy) => M.multiply(shift, copy.matrix)));
      } else if (key === 'relative' || key === 'relative-reset') {
        const was = this.fields.relative.slice();
        const back = keep();
        let turn;
        if (key === 'relative') {
          if (!Number.isFinite(value)) { this.showFields(); return; }
          turn = spin(M.rows.about(UNIT[axis], radians(value - was[axis])));
          this.fields.relative[axis] = value;
        } else {
          // Back to how it was when the tool was opened: the turns taken
          // back, the last first.
          turn = spin([2, 1, 0].reduce((rows, a) => M.rows.multiply(rows, M.rows.about(UNIT[a], radians(-was[a]))), M.rows.identity()));
          this.fields.relative = [0, 0, 0];
        }
        const kept = this.fields.relative.slice();
        this.place('Set Orientation', all((copy) => M.multiply(turn, copy.matrix))).then((made) => {
          this.fields.relative = made ? kept : was;
          back();
        });
      } else if (key === 'absolute' || key === 'absolute-reset') {
        if (!m.one || (key === 'absolute' && !Number.isFinite(value))) { this.showFields(); return; }
        const angles = key === 'absolute' ? m.angles.map((angle, a) => radians(a === axis ? value : angle)) : [0, 0, 0];
        const rows = turnFrom(angles);
        // A part's own turn is set within its object.
        const own = m.one.P || m.one.matrix;
        const { sizes, flip } = turnOf(own);
        const next = own.slice();
        for (let c = 0; c < 3; c += 1) for (let r = 0; r < 3; r += 1) next[4 * c + r] = rows[r][c] * sizes[c] * (c === 0 && flip ? -1 : 1);
        // About the middle of what is selected, which stays where it is.
        const local = m.local || [[0, 0, 0], [0, 0, 0]];
        const [was, now] = [M.point(own, M.centre(local)), M.point(next, M.centre(local))];
        for (let r = 0; r < 3; r += 1) next[12 + r] += was[r] - now[r];
        const matrix = m.one.C ? M.multiply(m.one.C, next) : next;
        this.place('Set Orientation', [{ ...m.one, matrix }]);
      } else if (key === 'scale' || key === 'size' || key === 'scale-reset') {
        let by;
        if (key === 'scale-reset') {
          by = m.scale.map((scale) => 100 / scale);
        } else {
          if (!(value > 0)) { this.showFields(); return; }
          const ratio = value / (key === 'scale' ? m.scale[axis] : m.size[axis]);
          const whole = this.fields.uniform || !m.one || (this.fields.frame === 'world' && m.turned);
          by = whole ? [ratio, ratio, ratio] : UNIT[axis].map((on) => (on ? ratio : 1));
        }
        if (m.one && m.local) {
          this.place('Set Scale', [{ ...m.one, matrix: this.sized(m.one.matrix, m.local, by) }]);
        } else {
          const grow = about(chosen.centre, M.scaling(by[0], by[0], by[0]));
          this.place('Set Scale', all((copy) => M.multiply(grow, copy.matrix)));
        }
      }
    },
  };

  GL.editor = edit;
})(window.EdgeGL);
