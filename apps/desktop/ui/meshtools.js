'use strict';

// A part's mesh operations, from the object and part menus
// (GUI_Factories.cpp): "Repair/Remesh" (GUI_ObjectList.cpp
// `repair_by_remesh`, RemeshDialog.cpp), "Simplify Model"
// (Gizmos/GLGizmoSimplify.cpp), and
// the object list's warning of a mesh's errors (GUI_ObjectList.cpp
// `get_mesh_errors_info`).
//
// "Simplify" is a panel over the stage, as the reference's gizmo is one
// that is not on the toolbar (`on_is_selectable` is false): it opens for
// one part, an object of one part selected whole being that part, and
// shows the part simplified in its place while the settings change, the
// session working it out (`mesh_simplify`, the one before called off) and
// telling how far it has got (`mesh_progress`). "Apply" puts the result in
// as one step of the history ("Simplify <name>", `mesh_apply`), "Cancel"
// closes it. An object of one part with a million triangles or more is
// pointed out when it is first seen, with "Simplify model" to open it
// (`add_simplify_suggestion_notification`).
//
// Where this differs, on purpose (UI_PARITY.md, section 13, "Simplify"):
// "Detail level" starts at "Medium" and simplifies with Medium's error;
// the reference's slider shows "Medium" (`static int reduction = 2`) while
// its first preview uses Extra low's (`max_error = 1.`).
(function (GL) {
  const M = GL.math;

  // GLGizmoSimplify.cpp: the errors of "Detail level", Extra high to Extra low.
  const LEVELS = [[1e-3, 'Extra high'], [1e-2, 'High'], [0.1, 'Medium'], [0.5, 'Low'], [1, 'Extra low']];
  // `GuiCfg::max_char_in_name`.
  const NAME_CHARS = 30;
  // `add_simplify_suggestion_notification`: `triangles_to_suggest_simplify`.
  const BIG = 1000000;
  // How often the progress is asked for while the session works.
  const POLL_MS = 120;

  // The float sums of `Configuration::fix_count_by_ratio`.
  const f = Math.fround;
  const countByRatio = (count, ratio) => {
    if (ratio <= 0) return count;
    if (ratio >= 100) return 0;
    return Math.round(f(f(f(count) * f(100 - ratio)) / 100));
  };

  const tools = {
    // What "Simplify" keeps between openings, as the gizmo keeps its
    // configuration (`m_configuration`) and the slider's place.
    config: { useCount: false, ratio: 50, level: 2, wireframe: false },
    open: null,
    seen: new Map(),

    init() {
      if (this.host || !GL.edit) return Boolean(this.host);
      this.editor = GL.edit;
      this.host = GL.edit.host;
      this.stage = GL.edit.stage;
      return true;
    },

    active() {
      return Boolean(this.open);
    },

    // ---- the menus ----

    // "Simplify Model" (`append_menu_item_simplify`), for the object menu
    // (`can_simplify`: an object selected, the tool not open).
    objectItems(object) {
      return [
        this.repairItem(object, []),
        [t('Simplify Model'), () => this.start(object, null), { disabled: !object || this.active() }],
      ];
    },

    // "Repair/Remesh" (`append_menu_item_fix_through_netfabb`), for an
    // object (every printed part of it) or the parts selected.
    repairItem(object, parts) {
      return [t('Repair/Remesh'), () => this.repair(object, parts), { title: t('Rebuild the selected parts as watertight meshes (detail below ~0.1mm may be lost)'), disabled: !object || this.repairing }];
    },

    // The same in the part menu, for the one part selected.
    partItems(object, parts) {
      return [
        this.repairItem(object, parts),
        [t('Simplify Model'), () => this.start(object, parts.length === 1 ? parts[0] : -1), { disabled: this.active() }],
      ];
    },

    // ---- the object list's warning ----

    // The warning icon and its tooltip (`get_mesh_errors_info`), or null.
    warning(errors) {
      if (!errors) return null;
      const lines = [];
      if (errors.repaired) lines.push(fmt(tn('%1$d error repaired', '%1$d errors repaired', errors.repaired), errors.repaired));
      if (errors.open_edges) {
        lines.push(`${t('Remaining errors')}:`);
        lines.push(`\t${fmt(tn('%1$d non-manifold edge', '%1$d non-manifold edges', errors.open_edges), errors.open_edges)}`);
      }
      const tip = lines.join('\n');
      const mark = document.createElement('span');
      mark.className = 'obj-warning';
      mark.textContent = '⚠';
      mark.title = tip;
      mark.setAttribute('role', 'img');
      mark.setAttribute('aria-label', tip);
      return mark;
    },

    // ---- "Simplify" ----

    data() {
      return this.editor.data();
    },

    target() {
      const o = this.open;
      const data = this.data();
      if (!o || !data) return null;
      const at = data.objects.findIndex((object) => object.id === o.object);
      if (at < 0) return null;
      const object = data.objects[at];
      const part = (object.part_list || []).find((each) => each.id === o.part);
      if (!part) return null;
      const placing = data.placings.find((each) => each.id === o.copy) || data.placings.find((each) => each.object === at);
      return placing ? { object, part, placing } : null;
    },

    // Opens it on `object`'s part `part` (null: its one part; -1: several
    // were selected). The reference says why it cannot otherwise.
    start(objectId, part) {
      if (!this.init()) return;
      const data = this.data();
      const object = data && data.objects.find((each) => each.id === objectId);
      if (!object) return;
      const list = object.part_list || [];
      const id = part === null ? (list.length === 1 ? list[0].id : null) : part;
      if (id === null || id === -1) {
        this.host.say(t('Simplification is currently only allowed when a single part is selected'), true);
        return;
      }
      this.editor.setTool(null);
      this.close();
      const copy = [...this.stage.selection()][0];
      this.open = { object: object.id, part: id, copy, asked: 0, id: null, result: null, triangles: null, running: false, progress: 0, cancelling: false, error: null };
      this.forget(object.id);
      this.process();
      this.panel();
    },

    close() {
      const o = this.open;
      if (!o) return;
      if (o.running) this.call('mesh_cancel', {});
      this.drop(o);
      this.open = null;
      if (this.panelNode) this.panelNode.hidden = true;
      if (this.stage.scene) this.stage.scene.replacing = null;
      this.stage.frame();
    },

    drop(o) {
      const r = this.stage.renderer;
      if (o.result) { r.drop(o.result.mesh); if (o.result.lines) r.drop(o.result.lines); }
      o.result = null;
    },

    call(name, args) {
      return this.host.call(name, { project: this.host.project().id, ...args });
    },

    // What is selected changed, or the scene: the tool closes when its
    // part is gone or another object is selected
    // (`on_render_input_window`: another volume starts afresh).
    follow() {
      const o = this.open;
      if (!o) { this.suggest(); return; }
      const there = this.target();
      if (!there) { this.close(); return; }
      const copy = [...this.stage.selection()][0];
      const data = this.data();
      const chosen = data.placings.find((each) => each.id === copy);
      if (chosen && data.objects[chosen.object].id !== o.object) { this.close(); return; }
      this.stage.scene.replacing = this.replacing();
      this.panel();
      this.suggest();
    },

    // The part not drawn while its preview is (scene.js).
    replacing() {
      const there = this.target();
      if (!there || !this.open.result) return null;
      return { copy: there.placing.id, part: there.part.triangles };
    },

    // `GLGizmoSimplify::process`: the result for the settings now, the one
    // under way called off by the session.
    async process() {
      const o = this.open;
      const there = this.target();
      if (!o || !there) return;
      const c = this.config;
      const original = there.part.triangles ? there.part.triangles[1] : there.object.triangles;
      const args = { object: o.object, part: o.part };
      if (c.useCount) args.wanted = countByRatio(original, c.ratio);
      else args.max_error = LEVELS[c.level][0];
      o.asked += 1;
      const asked = o.asked;
      o.running = true;
      o.progress = 0;
      this.panel();
      this.poll(o);
      let answer;
      try {
        answer = await this.call('mesh_simplify', args);
      } catch (error) {
        if (this.open !== o || asked !== o.asked) return;
        o.running = false;
        o.cancelling = false;
        if (!/stopped/.test(error.message)) o.error = error.message;
        this.panel();
        return;
      }
      if (this.open !== o || asked !== o.asked) return;
      let bytes;
      try {
        bytes = await this.stage.bytes('mesh_result', { id: answer.id });
      } catch (error) {
        return;
      }
      if (this.open !== o || asked !== o.asked) return;
      this.drop(o);
      const r = this.stage.renderer;
      o.result = { mesh: r.meshFromBytes(bytes), lines: r.lines(this.edges(bytes)) };
      o.id = answer.id;
      o.triangles = answer.triangles;
      o.running = false;
      o.error = null;
      this.stage.scene.replacing = this.replacing();
      this.panel();
      this.stage.frame();
    },

    async poll(o) {
      while (this.open === o && o.running) {
        await new Promise((done) => setTimeout(done, POLL_MS));
        if (this.open !== o || !o.running) return;
        try {
          const answer = await this.host.call('mesh_progress', {});
          if (answer.running && this.open === o && o.running) {
            o.progress = answer.progress;
            this.panel();
          }
        } catch (error) {
          return;
        }
      }
    },

    // Each edge of a mesh as it is sent (api/scene.rs `mesh_bytes`) once,
    // for "Show wireframe".
    edges(bytes) {
      const view = new DataView(bytes);
      const [count, triangles] = [8, 12].map((at) => view.getUint32(at, true));
      const positions = new Float32Array(bytes, 24, count * 3);
      const corners = new Uint32Array(bytes, 24 + count * 12, triangles * 3);
      const seen = new Set();
      const ends = [];
      for (let t = 0; t < triangles; t += 1) {
        for (let k = 0; k < 3; k += 1) {
          const a = corners[t * 3 + k];
          const b = corners[t * 3 + ((k + 1) % 3)];
          const key = a < b ? a * 4294967296 + b : b * 4294967296 + a;
          if (seen.has(key)) continue;
          seen.add(key);
          ends.push(positions[a * 3], positions[a * 3 + 1], positions[a * 3 + 2], positions[b * 3], positions[b * 3 + 1], positions[b * 3 + 2]);
        }
      }
      return ends;
    },

    // `apply_simplify`.
    async apply() {
      const o = this.open;
      if (!o || o.running || !o.id || this.editor.busy) return;
      this.editor.busy = true;
      try {
        const answer = await this.call('mesh_apply', { id: o.id });
        this.close();
        this.editor.take(answer);
      } catch (error) {
        this.host.say(error.message, true);
      } finally {
        this.editor.busy = false;
        this.editor.refresh();
      }
    },

    // `on_render`: the result where the part stands, in its colour, with
    // its edges over it when "Show wireframe" is ticked.
    draw(frame) {
      const o = this.open;
      const there = this.target();
      if (!o || !o.result || !there) return;
      const r = this.stage.renderer;
      const matrix = M.multiply(there.placing.matrix, there.part.matrix);
      r.drawMesh(o.result.mesh, frame, matrix, { palette: this.stage.scene.palette, pass: 0 });
      if (this.config.wireframe) {
        const gl = r.gl;
        gl.enable(gl.DEPTH_TEST);
        gl.depthFunc(gl.LEQUAL);
        r.drawLines(o.result.lines, M.chain(frame.projection, frame.view, matrix), [1, 1, 1, 1], 1);
        gl.depthFunc(gl.LESS);
      }
    },

    // The panel (`on_render_input_window`).
    panel() {
      const { el } = this.host;
      if (!this.panelNode) {
        this.panelNode = el('div', { className: 'edit-panel overlay cut-panel simplify-panel', id: 'simplify-panel', hidden: true });
        this.panelNode.setAttribute('role', 'group');
        this.panelNode.setAttribute('aria-label', t('Simplify'));
        this.host.stageNode.append(this.panelNode);
      }
      const node = this.panelNode;
      const o = this.open;
      const there = this.target();
      if (!o || !there) { node.hidden = true; return; }
      node.hidden = false;
      const c = this.config;
      const original = there.part.triangles ? there.part.triangles[1] : there.object.triangles;
      let name = there.part.name || there.object.name || '';
      if (name.length > NAME_CHARS) name = `${name.slice(0, NAME_CHARS - 3)}...`;
      // With "Detail level", the ratio and count are the preview's.
      if (!c.useCount && o.triangles !== null) c.shownRatio = (1 - f(o.triangles / original)) * 100;
      const ratio = c.useCount ? c.ratio : (c.shownRatio ?? c.ratio);
      const wanted = c.useCount ? countByRatio(original, c.ratio) : (o.triangles ?? countByRatio(original, c.ratio));
      const fixed = (value) => (value > 10 ? value.toFixed(0) : value > 1 ? value.toFixed(1) : value.toFixed(2));
      const rows = [el('h3', { textContent: t('Simplify') })];
      rows.push(el('div', { className: 'cut-row' }, el('span', { className: 'cut-part', textContent: `${t('Mesh name')}:` }), el('span', { className: 'cut-line quiet', textContent: name, title: there.part.name || '' })));
      rows.push(el('div', { className: 'cut-row' }, el('span', { className: 'cut-part', textContent: `${t('Triangles')}:` }), el('span', { className: 'cut-line quiet', textContent: String(original) })));
      rows.push(el('hr'));
      const choice = (useCount, label) => {
        const radio = el('input', { type: 'radio', name: 'simplify-by', checked: c.useCount === useCount });
        radio.addEventListener('change', () => { c.useCount = useCount; this.process(); });
        return el('label', { className: 'simplify-choice' }, radio, label);
      };
      // "Detail level": five steps, named.
      const level = el('input', { type: 'range', min: 0, max: 4, step: 1, value: c.level, disabled: c.useCount });
      level.setAttribute('aria-label', t('Detail level'));
      const levelName = el('span', { className: 'simplify-value', textContent: t(LEVELS[c.level][1]) });
      level.addEventListener('input', () => { c.level = Number(level.value); levelName.textContent = t(LEVELS[c.level][1]); });
      level.addEventListener('change', () => this.process());
      rows.push(el('div', { className: 'cut-row' }, choice(false, t('Detail level')), el('span', { className: 'cut-line' }, level, levelName)));
      // "Decimate ratio": a slider and the number, then the count it gives.
      const slider = el('input', { type: 'range', min: 0, max: 100, step: 0.01, value: ratio, disabled: !c.useCount });
      slider.setAttribute('aria-label', t('Decimate ratio'));
      const number = el('input', { type: 'number', min: 0, max: 100, step: 0.05, value: Number(ratio).toFixed(2), disabled: !c.useCount, className: 'simplify-number' });
      number.setAttribute('aria-label', t('Decimate ratio'));
      const shown = el('span', { className: 'simplify-value', textContent: `${fixed(Number(ratio))} %` });
      const count = el('div', { className: 'simplify-count quiet', textContent: fmt(t('%d triangles'), wanted) });
      const setRatio = (value, run) => {
        let v = Number(value);
        if (!Number.isFinite(v)) return;
        if (v < 0) v = 0.01;
        if (v > 100) v = 100;
        c.ratio = v;
        shown.textContent = `${fixed(v)} %`;
        count.textContent = fmt(t('%d triangles'), countByRatio(original, v));
        if (run) this.process();
      };
      slider.addEventListener('input', () => { number.value = Number(slider.value).toFixed(2); setRatio(slider.value, false); });
      slider.addEventListener('change', () => setRatio(slider.value, true));
      number.addEventListener('change', () => { slider.value = number.value; setRatio(number.value, true); });
      rows.push(el('div', { className: 'cut-row' }, choice(true, t('Decimate ratio')), el('span', { className: 'cut-line' }, slider, shown, number)));
      rows.push(count);
      const wire = el('input', { type: 'checkbox', checked: c.wireframe });
      wire.addEventListener('change', () => { c.wireframe = wire.checked; this.stage.frame(); });
      rows.push(el('label', { className: 'edit-check' }, wire, ` ${t('Show wireframe')}`));
      if (o.error) rows.push(el('p', { className: 'cut-note', textContent: o.error }));
      // The progress while it works.
      const bar = el('div', { className: 'simplify-progress', hidden: !o.running });
      const fill = el('span');
      fill.style.width = `${o.progress}%`;
      bar.append(fill);
      const percent = el('span', { className: 'quiet', textContent: o.running ? `${o.progress}%` : '' });
      const apply = el('button', { type: 'button', className: 'primary', textContent: t('Apply'), disabled: o.running || !o.id || this.editor.busy });
      if (o.running) apply.title = t("Can't apply when processing preview.");
      apply.addEventListener('click', () => this.apply());
      const cancel = el('button', { type: 'button', textContent: t('Cancel'), disabled: o.cancelling });
      if (o.cancelling) cancel.title = t('Operation already cancelling. Please wait a few seconds.');
      cancel.addEventListener('click', () => this.close());
      rows.push(el('div', { className: 'cut-foot simplify-foot' }, bar, percent, apply, cancel));
      node.replaceChildren(...rows);
    },

    // ---- "Repair/Remesh" ----

    // The options as last chosen (RemeshDialog.cpp `load_from_config`):
    // the voxel size the part's own until one is chosen.
    remeshOptions(auto) {
      let kept = {};
      try { kept = JSON.parse(this.host.remembered.get('remesh') || '{}') || {}; } catch { kept = {}; }
      const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);
      const number = (v, or) => (typeof v === 'number' && Number.isFinite(v) ? v : or);
      return {
        voxel_size: clamp(number(kept.voxel_size, auto), 0.02, 5),
        keep_bottom_flat: kept.keep_bottom_flat ?? true,
        bottom_margin: clamp(number(kept.bottom_margin, 0), 0, 50),
        preserve_sharp_edges: kept.preserve_sharp_edges ?? true,
        sharp_feature_angle: clamp(number(kept.sharp_feature_angle, 45), 5, 175),
      };
    },

    // `repair_by_remesh`: the dialog, then the parts rebuilt as one step.
    async repair(objectId, parts) {
      if (!this.init() || this.repairing) return;
      const project = this.host.project();
      let info;
      try {
        info = await this.host.call('mesh_repair_info', { project: project.id, object: objectId, parts });
      } catch (error) {
        this.host.say(error.message, true);
        return;
      }
      if (!info.parts) return;
      const options = await this.remeshDialog(info);
      if (!options) return;
      this.close();
      this.repairing = true;
      const done = { running: true };
      const report = async () => {
        while (done.running) {
          try {
            const answer = await this.host.call('mesh_progress', {});
            if (done.running && answer.running) this.host.say(`${t('Repair by remeshing')}: ${answer.progress}%`, false);
          } catch (error) { /* said when it ends */ }
          await new Promise((wait) => setTimeout(wait, POLL_MS * 2));
        }
      };
      this.host.say(`${t('Repair by remeshing')}…`, false);
      report();
      try {
        const answer = await this.host.call('mesh_repair', { project: project.id, object: objectId, parts, options });
        done.running = false;
        this.editor.take(answer);
      } catch (error) {
        done.running = false;
        this.host.say(error.message, true);
      } finally {
        this.repairing = false;
        this.editor.refresh();
      }
    },

    // RemeshDialog.cpp: answers the options, or null for "Cancel".
    remeshDialog(info) {
      const { el } = this.host;
      const o = this.remeshOptions(info.auto_voxel);
      const dialog = el('dialog', { className: 'remesh-dialog' });
      dialog.setAttribute('aria-label', t('Repair by remeshing'));
      const form = el('form', { method: 'dialog' });
      const input = (value, unit, tip) => {
        // `num_str`: three figures.
        const field = el('input', { type: 'number', step: 'any', value: String(Number(value.toPrecision(3))), className: 'remesh-number' });
        const node = el('span', { className: 'unit remesh-unit', title: tip }, field, el('span', { textContent: t(unit) }));
        return [field, node];
      };
      const flatTip = t('Voxel remeshing reconstructs the surface on a fixed grid, which leaves a flat base rippled and rounds off its outline by up to a voxel - the first layer is then no longer flush with the bed. With this on, the part is cut just above its base, only the upper piece is remeshed, and the untouched bottom slab is joined back on, so the base comes through exactly as it went in.\n\nParts with no flat base are remeshed whole, as before.');
      const sharpTip = t('The distance field averages both faces together within about a voxel of a hard edge, so corners come back filleted. With this on, the edges of the original part are found first, and any rebuilt vertex that lands within a voxel of one is pulled back onto it.');
      const voxelTip = fmt(t('How coarse the rebuild is. The mesh is re-created as the surface of its distance field on a grid of this spacing, so detail smaller than one voxel is lost and a smaller value means more triangles and a slower repair.\n\nThe value for this part chosen automatically is %s mm.'), String(Number(info.auto_voxel.toPrecision(3))));
      const [voxel, voxelNode] = input(o.voxel_size, 'mm', voxelTip);
      // The part's own size, untouched, stays each part's own.
      const untouched = o.voxel_size === info.auto_voxel ? voxel.value : null;
      const flat = el('input', { type: 'checkbox', checked: o.keep_bottom_flat, title: flatTip });
      const [margin, marginNode] = input(o.bottom_margin > 0 ? o.bottom_margin : 2.5 * o.voxel_size, 'mm', t('How much of the original base is kept untouched. Leave it at a couple of voxels: too thin and the cut runs through the rounding the remesh would have added anyway, too thick and a sloped wall picks up a visible seam where the two pieces meet.'));
      const sharp = el('input', { type: 'checkbox', checked: o.preserve_sharp_edges, title: sharpTip });
      const [angle, angleNode] = input(o.sharp_feature_angle, 'deg', t('Two faces meeting at more than this angle count as a hard edge. Lower values protect gentler creases as well; higher values protect only the crispest corners.'));
      const preview = el('p', { className: 'remesh-preview' });
      const read = (field, or) => { const v = Number(field.value); return field.value.trim() && Number.isFinite(v) ? v : or; };
      // `update_preview`: about two triangles a voxel of surface, to two
      // figures.
      const update = () => {
        margin.disabled = !flat.checked;
        angle.disabled = !sharp.checked;
        const size = Math.min(Math.max(read(voxel, o.voxel_size), 0.02), 5);
        if (!info.triangles || info.area <= 0 || size <= 0) {
          preview.textContent = fmt(t('Triangles: %llu'), info.triangles);
          return;
        }
        let estimate = 2 * info.area / (size * size);
        if (estimate >= 10) {
          const magnitude = 10 ** (Math.floor(Math.log10(estimate)) - 1);
          estimate = Math.round(estimate / magnitude) * magnitude;
        }
        preview.textContent = fmt(t('Triangles: %llu -> about %llu'), info.triangles, Math.round(estimate));
      };
      for (const node of [voxel, margin, angle]) node.addEventListener('input', update);
      flat.addEventListener('change', update);
      sharp.addEventListener('change', update);
      const row = (label, control, tip) => el('label', { className: 'remesh-row', title: tip || '' }, el('span', { textContent: `${t(label)}:` }), control);
      const ok = el('button', { type: 'submit', value: 'ok', className: 'primary', textContent: t('Remesh') });
      const cancel = el('button', { type: 'submit', value: 'cancel', formNoValidate: true, textContent: t('Cancel') });
      form.append(
        el('h2', { textContent: t('Repair by remeshing') }),
        el('div', { className: 'remesh-grid' },
          row('Voxel size', voxelNode, voxelTip),
          row('Keep the bottom flat', flat, flatTip),
          row('Bottom slab height', marginNode),
          row('Preserve sharp edges', sharp, sharpTip),
          row('Feature angle', angleNode)),
        preview,
        el('p', { className: 'quiet remesh-note', textContent: t('Painted supports, seams and colours are cleared by a remesh.') }),
        el('div', { className: 'row end' }, ok, cancel),
      );
      dialog.append(form);
      document.body.append(dialog);
      update();
      return new Promise((resolve) => {
        dialog.addEventListener('close', () => {
          const auto = untouched !== null && voxel.value === untouched;
          const chosen = dialog.returnValue === 'ok' ? {
            voxel_size: auto ? 0 : Math.min(Math.max(read(voxel, o.voxel_size), 0.02), 5),
            keep_bottom_flat: flat.checked,
            bottom_margin: Math.min(Math.max(read(margin, o.bottom_margin), 0), 50),
            preserve_sharp_edges: sharp.checked,
            sharp_feature_angle: Math.min(Math.max(read(angle, o.sharp_feature_angle), 5), 175),
          } : null;
          // `save_to_config`, on "Remesh"; an untouched size is not kept,
          // so the next part is offered its own again.
          if (chosen) {
            const kept = { ...chosen };
            if (auto) delete kept.voxel_size;
            this.host.remembered.set('remesh', JSON.stringify(kept));
          }
          dialog.remove();
          resolve(chosen);
        }, { once: true });
        dialog.returnValue = 'cancel';
        dialog.showModal();
        voxel.focus();
      });
    },

    // ---- the suggestion for big meshes ----

    // `add_simplify_suggestion_notification`: an object of one part with a
    // million triangles or more, pointed out once when it is first seen.
    suggest() {
      if (!this.init()) return;
      const project = this.host.project();
      const data = this.data();
      if (!project || !data) return;
      const key = project.id;
      const seen = this.seen.get(key) || new Set();
      const fresh = this.seen.has(key) ? data.objects.filter((object) => !seen.has(object.id)) : data.objects;
      this.seen.set(key, seen);
      for (const object of data.objects) seen.add(object.id);
      for (const object of fresh) {
        if ((object.part_list || []).length !== 1 || object.triangles < BIG) continue;
        this.notices = (this.notices || []).filter((notice) => notice.object !== object.id);
        this.notices.push({ project: key, object: object.id, name: object.name || '' });
      }
      this.showNotices();
    },

    forget(object) {
      this.notices = (this.notices || []).filter((notice) => notice.object !== object);
      this.showNotices();
    },

    showNotices() {
      const { el } = this.host;
      if (!this.noticeNode) {
        this.noticeNode = el('div', { className: 'simplify-notices overlay', hidden: true });
        this.host.stageNode.append(this.noticeNode);
      }
      const project = this.host.project();
      const here = (this.notices || []).filter((notice) => project && notice.project === project.id);
      this.noticeNode.hidden = !here.length;
      this.noticeNode.replaceChildren(...here.map((notice) => {
        const text = fmt(t("Processing model '%1%' with more than 1M triangles could be slow. It is highly recommended to simplify the model."), notice.name);
        const link = el('button', { type: 'button', className: 'link', textContent: t('Simplify model') });
        link.addEventListener('click', () => {
          const data = this.data();
          const at = data.objects.findIndex((object) => object.id === notice.object);
          const placing = data.placings.find((each) => each.object === at);
          this.forget(notice.object);
          if (!placing) return;
          this.stage.select([placing.id]);
          this.start(notice.object, null);
        });
        const shut = el('button', { type: 'button', className: 'icon tool', textContent: '✕', title: t('Close') });
        shut.addEventListener('click', () => this.forget(notice.object));
        return el('div', { className: 'simplify-notice', role: 'status' }, el('span', { textContent: `${text} ` }), link, shut);
      }));
    },
  };

  GL.meshTools = tools;
})(window.EdgeGL);
