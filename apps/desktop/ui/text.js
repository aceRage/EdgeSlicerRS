'use strict';

// "Emboss" [T]: text made into a part of an object, or an object of its
// own (Gizmos/GLGizmoEmboss.cpp with Jobs/EmbossJob.cpp). As the
// reference's tool, it works on a text part that exists: opened with a
// text part selected, it edits it; opened with an object selected, it
// first adds a text where the line of sight through the middle of the
// object's box meets it (`start_create_volume_without_position`), or
// below the object when it meets nothing; opened with nothing selected,
// it makes an object of a text on the plate (`CreateObjectJob`). "Add
// part" / "Add negative part" / "Add modifier" ▸ "Text" in the object
// menu add one where the menu was opened (`create_volume` at the mouse).
//
// The panel is the reference's: the text, "Font" with italic and bold,
// "Height", "Depth", "Advanced" (alignment, gaps, boldness, skew, the
// distance from the surface, the rotation and "Set text to face camera")
// and "Operation" ("Join", "Cut", "Modifier"). Every change remakes the
// part in the session at once, as the reference's jobs do; the changes
// made while the tool stays open on one text are one step of the
// history ("Emboss attribute change"). The fonts are the session's
// (`text_fonts`): their names, and each drawn in itself for the list.
//
// Not here yet, and so not offered: "Use surface" and "Per glyph" (shown,
// off), the curve, the style list, inline shapes, dragging the text over
// the surface and the 3D handles.
(function (GL) {
  const M = GL.math;

  // How long the panel waits after a change before the part is made again.
  const SETTLE_MS = 250;
  // GLGizmoEmboss.cpp `limits`.
  const LIMITS = { size: [0.1, 1000], depth: [0.01, 1e4], skew: [-1, 1], boldness: [-0.5, 0.5], angle: [-180, 180] };

  const texter = {
    open: null,
    fonts: null,
    previews: new Map(),

    init(host, editor) {
      this.host = host;
      this.editor = editor;
      this.stage = host.stage;
    },

    active() {
      return Boolean(this.open);
    },

    data() {
      return this.editor.data();
    },

    call(name, args) {
      return this.host.call(name, { project: this.host.project().id, ...args });
    },

    async loadFonts() {
      if (this.fonts) return this.fonts;
      this.fonts = await this.host.call('text_fonts', {});
      this.faces = new Map();
      for (const family of this.fonts.families) {
        for (const face of family.faces) this.faces.set(face.id, { ...face, family });
      }
      return this.fonts;
    },

    // ---- opening ----

    // The tool opened from the toolbar or by T: what is selected decides.
    async start() {
      const editor = this.editor;
      const items = editor.partItems();
      if (items.length === 1 && items[0].part.text) { await this.edit(items[0].objectId, items[0].id, items[0].copy.id); return; }
      const chosen = editor.chosen();
      if (chosen && !editor.partMode() && chosen.copies.length === 1) {
        await this.add('part', chosen.copies[0], this.middleOf(chosen.copies[0]));
        return;
      }
      if (!chosen) { await this.add('part', null, null); return; }
      editor.setTool(null);
    },

    // The pixel at the middle of a copy's box as the view shows it: what
    // the reference casts its ray through (`find_closest`: the middle of
    // the copy's outline on the screen).
    middleOf(copy) {
      if (!copy.box) return null;
      const frame = this.stage.matrices();
      const at = this.editor.toView(frame, M.centre(copy.box));
      return at;
    },

    // Adds a text of `kind` to `copy`'s object through the pixel `at`, or
    // an object of a text when no copy is given.
    async add(kind, copy, at) {
      try {
        await this.loadFonts();
      } catch (error) {
        this.host.say(error.message, true);
        this.editor.setTool(null);
        return;
      }
      if (this.fonts.default === null || this.fonts.default === undefined) {
        this.host.say(t('No font is installed that the text can be written in.'), true);
        this.editor.setTool(null);
        return;
      }
      const style = { ...this.fonts.style, face: this.fonts.default, angle: 0 };
      const args = { op: 'add', kind, style };
      if (copy) {
        const data = this.data();
        args.object = data.objects[copy.object].id;
        args.copy = copy.id;
        if (at) {
          const ray = this.stage.camera.ray(at[0], at[1]);
          args.origin = ray.origin;
          args.direction = ray.direction;
        }
      }
      this.editor.busy = true;
      let answer;
      try {
        answer = await this.call('text_edit', args);
      } catch (error) {
        this.host.say(error.message, true);
        this.editor.setTool(null);
        return;
      } finally {
        this.editor.busy = false;
      }
      this.editor.take(answer);
      const data = this.data();
      let placing;
      if (copy) {
        placing = data.placings.find((each) => each.id === copy.id);
      } else {
        placing = data.placings.find((each) => each.id === answer.made[0]);
      }
      if (!placing || answer.part === null) return;
      const object = data.objects[placing.object];
      // The part made is selected, the tool on it; an object of one text
      // is selected whole.
      if ((object.part_list || []).length > 1) this.stage.selectParts(placing.id, [answer.part]);
      else this.stage.select([placing.id]);
      if (this.editor.tool !== 'text') this.editor.tool = 'text';
      await this.edit(object.id, answer.part, placing.id, true);
    },

    // Opens the panel on a text part: its description from the session.
    async edit(objectId, part, copy, fresh = false) {
      let info;
      try {
        await this.loadFonts();
        info = await this.call('text_part', { object: objectId, part });
      } catch (error) {
        this.host.say(error.message, true);
        this.editor.setTool(null);
        return;
      }
      const face = info.face ?? this.fonts.default;
      this.open = {
        object: objectId, part, copy,
        style: { ...info.style, face, angle: round(info.angle, 2), distance: info.distance },
        kind: info.kind, onlyPart: info.only_part,
        // A font not found here: only the font can be chosen, then "Apply"
        // (`m_is_unknown_font`).
        unknown: info.face === null ? (info.face_name || '') : null,
        notYet: info.not_yet,
        // The first change made after opening is a step of its own; the
        // rest join it.
        merge: fresh,
        timer: null, error: null, sent: 0, touched: {},
      };
      this.editor.refresh();
      this.panel();
    },

    close() {
      const o = this.open;
      if (!o) return;
      if (o.timer) this.flush();
      this.open = null;
      if (this.listNode) this.listNode.hidden = true;
      if (this.panelNode) this.panelNode.hidden = true;
    },

    // What is selected changed, or the scene: the tool stays on its part
    // while it is there and selected (an undo may take it away).
    follow() {
      const o = this.open;
      if (!o) return;
      const data = this.data();
      const object = data && data.objects.find((each) => each.id === o.object);
      const part = object && (object.part_list || []).find((each) => each.id === o.part);
      if (!part || !part.text) { this.editor.setTool(null); return; }
      const items = this.editor.partItems();
      const whole = !this.editor.partMode() && (object.part_list || []).length === 1 && this.stage.selection().size === 1;
      const selected = whole || (items.length === 1 && items[0].id === o.part);
      if (!selected) {
        // Another text part selected: the tool goes to it.
        if (items.length === 1 && items[0].part.text) { this.edit(items[0].objectId, items[0].id, items[0].copy.id); return; }
        this.editor.setTool(null);
      }
    },

    // After an undo or a redo the part is described again.
    reread() {
      const o = this.open;
      if (o) this.edit(o.object, o.part, o.copy, false);
    },

    // ---- making the part again ----

    changed(now = false) {
      const o = this.open;
      if (!o || o.unknown !== null) { this.panel(); return; }
      if (o.timer) clearTimeout(o.timer);
      o.timer = setTimeout(() => this.flush(), now ? 0 : SETTLE_MS);
      this.panel();
    },

    async flush(extra = {}) {
      const o = this.open;
      if (!o) return;
      if (o.timer) { clearTimeout(o.timer); o.timer = null; }
      if (this.editor.busy) { o.timer = setTimeout(() => this.flush(extra), SETTLE_MS); return; }
      o.sent += 1;
      const sent = o.sent;
      const args = { op: 'set', object: o.object, part: o.part, copy: o.copy, kind: o.kind, style: { ...o.style }, merge: o.merge, ...extra };
      // The turn and the distance are sent once the person set them: they
      // move the part, and the panel shows them rounded.
      if (!o.touched.angle) delete args.style.angle;
      if (!o.touched.distance || o.style.distance === null || o.style.distance === undefined) delete args.style.distance;
      this.editor.busy = true;
      try {
        const answer = await this.call('text_edit', args);
        if (this.open !== o) return;
        o.merge = true;
        o.error = null;
        this.editor.take(answer);
      } catch (error) {
        if (this.open === o && sent === o.sent) o.error = error.message;
      } finally {
        this.editor.busy = false;
        this.panel();
      }
    },

    // ---- the fonts ----

    face() {
      return this.faces ? this.faces.get(this.open.style.face) : null;
    },

    // A face of the family nearest the weight and slant asked for.
    faceOf(family, weight, italic) {
      let best = null;
      for (const face of family.faces) {
        const score = (face.italic === italic ? 0 : 10000) + Math.abs(face.weight - weight);
        if (!best || score < best.score) best = { score, face };
      }
      return best ? best.face : null;
    },

    // The family's sample, drawn in itself: asked for once.
    preview(face, node) {
      const key = face.id;
      const draw = (shape) => {
        if (!shape || !shape.d) return;
        const [x, y, w, h] = shape.box;
        const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        svg.setAttribute('viewBox', `${x} ${y} ${w} ${h}`);
        svg.setAttribute('aria-hidden', 'true');
        svg.setAttribute('class', 'text-sample');
        const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('d', shape.d);
        path.setAttribute('fill-rule', 'nonzero');
        svg.append(path);
        node.replaceChildren(svg);
      };
      if (this.previews.has(key)) { draw(this.previews.get(key)); return; }
      this.previews.set(key, null);
      this.host.call('text_font_preview', { face: key }).then((shape) => {
        this.previews.set(key, shape);
        if (node.isConnected) draw(shape);
      }).catch(() => {});
    },

    // The list of families (`draw_font_list`): each by its name, drawn in
    // itself as it scrolls into view, with a filter.
    toggleList(anchor) {
      const { el } = this.host;
      if (!this.listNode) {
        this.listNode = el('div', { className: 'overlay text-fonts', hidden: true });
        this.listNode.setAttribute('role', 'listbox');
        this.listNode.setAttribute('aria-label', t('Font'));
        this.host.stageNode.append(this.listNode);
      }
      const node = this.listNode;
      if (!node.hidden) { node.hidden = true; return; }
      const filter = el('input', { type: 'search', className: 'text-filter' });
      filter.setAttribute('aria-label', t('Font'));
      const list = el('div', { className: 'text-font-list' });
      const observer = new IntersectionObserver((entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          const row = entry.target;
          observer.unobserve(row);
          this.preview(this.faces.get(Number(row.dataset.face)), row.querySelector('.text-font-sample'));
        }
      }, { root: list });
      const fill = () => {
        const wanted = filter.value.trim().toLowerCase();
        const rows = [];
        const current = this.face();
        for (const family of this.fonts.families) {
          if (wanted && !family.name.toLowerCase().includes(wanted)) continue;
          const face = this.faceOf(family, 400, false);
          const row = el('button', { type: 'button', className: 'text-font', title: family.name });
          row.dataset.face = String(face.id);
          row.setAttribute('role', 'option');
          row.setAttribute('aria-selected', String(Boolean(current && current.family === family)));
          row.append(el('span', { className: 'text-font-name', textContent: family.name }), el('span', { className: 'text-font-sample' }));
          row.addEventListener('click', () => {
            node.hidden = true;
            const was = this.face();
            const chosen = this.faceOf(family, was ? was.weight : 400, was ? was.italic : false);
            this.chooseFace(chosen.id);
          });
          rows.push(row);
        }
        list.replaceChildren(...rows);
        for (const row of rows) observer.observe(row);
      };
      filter.addEventListener('input', fill);
      filter.addEventListener('keydown', (event) => { if (event.key === 'Escape') { node.hidden = true; event.stopPropagation(); } });
      fill();
      const skipped = this.fonts.skipped ? el('p', { className: 'quiet text-skipped', textContent: fmt(t('%d fonts could not be read and are not listed.'), this.fonts.skipped) }) : null;
      node.replaceChildren(filter, list, ...(skipped ? [skipped] : []));
      const box = anchor.getBoundingClientRect();
      const stage = this.host.stageNode.getBoundingClientRect();
      node.style.left = `${box.left - stage.left}px`;
      node.style.top = `${box.bottom - stage.top + 4}px`;
      node.hidden = false;
      filter.focus();
    },

    chooseFace(id) {
      const o = this.open;
      if (!o) return;
      o.style.face = id;
      if (o.unknown !== null) { this.panel(); return; }
      this.changed(true);
    },

    // "Set italic" / "Unset italic" (`draw_italic_button`): the family's
    // italic face, else a skew of 0.2; and back.
    toggleItalic() {
      const o = this.open;
      const face = this.face();
      if (!face) return;
      const on = face.italic || Math.abs(o.style.skew || 0) > 1e-9;
      if (on) {
        o.style.skew = 0;
        if (face.italic) o.style.face = this.faceOf(face.family, face.weight, false).id;
      } else {
        const italic = this.faceOf(face.family, face.weight, true);
        if (italic.italic) o.style.face = italic.id;
        else o.style.skew = 0.2;
      }
      this.changed(true);
    },

    // "Set bold" / "Unset bold" (`draw_bold_button`): the family's bold
    // face, else a boldness of 20; and back.
    toggleBold() {
      const o = this.open;
      const face = this.face();
      if (!face) return;
      const bold = face.weight > 450;
      const on = bold || Math.abs(o.style.boldness || 0) > 1e-9;
      if (on) {
        o.style.boldness = 0;
        if (bold) o.style.face = this.faceOf(face.family, 400, face.italic).id;
      } else {
        const heavy = this.faceOf(face.family, 700, face.italic);
        if (heavy.weight > 450) o.style.face = heavy.id;
        else o.style.boldness = 20;
      }
      this.changed(true);
    },

    // "Set text to face camera" (`face_selected_volume_to_camera`).
    faceCamera() {
      const camera = this.stage.camera;
      const view = camera.perspective ? { eye: camera.position, forward: camera.forward } : { forward: camera.forward };
      this.flush({ face_camera: view, keep_up: this.keepUp !== false });
    },

    // ---- the panel ----

    panel() {
      const { el } = this.host;
      if (!this.panelNode) {
        this.panelNode = el('div', { className: 'edit-panel overlay cut-panel text-panel', id: 'text-panel', hidden: true });
        this.panelNode.setAttribute('role', 'group');
        this.panelNode.setAttribute('aria-label', t('Emboss'));
        this.host.stageNode.append(this.panelNode);
      }
      const node = this.panelNode;
      const o = this.open;
      if (!o || !this.faces) { node.hidden = true; return; }
      // A text box being typed in keeps its place: only the rest is drawn.
      const typing = document.activeElement && document.activeElement.id === 'text-input';
      node.hidden = false;
      const s = o.style;
      const face = this.face();
      const locked = o.unknown !== null;
      const rows = [el('h3', { textContent: t('Emboss') })];
      // The text.
      let input = typing ? document.getElementById('text-input') : null;
      if (!input) {
        input = el('textarea', { id: 'text-input', className: 'text-input', rows: 3, value: s.text, disabled: locked });
        input.setAttribute('aria-label', t('Text'));
        input.addEventListener('input', () => { s.text = input.value; this.changed(); });
      }
      rows.push(input);
      const warnings = [];
      if (!/[^ \n\t\r]/.test(s.text)) warnings.push(t('Embossed text cannot contain only white spaces.'));
      if (o.error) warnings.push(o.error);
      if (o.notYet) warnings.push(o.notYet);
      if (locked) warnings.push(fmt(t("Can't load exactly same font (\"%1%\"). Application selected a similar one (\"%2%\"). You have to specify font for enable edit text."), o.unknown, face ? face.family.name : ''));
      for (const text of warnings) rows.push(el('p', { className: 'cut-note', textContent: text }));
      // "Font", the family list, italic and bold (or "Apply" for a font
      // not found).
      const fontButton = el('button', { type: 'button', className: 'text-font-button', textContent: face ? face.family.name : '' });
      fontButton.addEventListener('click', () => this.toggleList(fontButton));
      const faceSelect = el('select', { className: 'text-face', disabled: !face });
      faceSelect.setAttribute('aria-label', t('Font'));
      if (face) {
        for (const each of face.family.faces) faceSelect.append(new Option(each.style, String(each.id)));
        faceSelect.value = String(face.id);
      }
      faceSelect.addEventListener('change', () => this.chooseFace(Number(faceSelect.value)));
      const fontLine = [fontButton, faceSelect];
      if (locked) {
        const apply = el('button', { type: 'button', textContent: t('Apply') });
        apply.addEventListener('click', () => { o.unknown = null; this.changed(true); });
        fontLine.push(apply);
      } else {
        const italic = el('button', { type: 'button', className: 'icon tool text-style', textContent: 'I', disabled: !face });
        const isItalic = face && (face.italic || Math.abs(s.skew || 0) > 1e-9);
        italic.title = isItalic ? t('Unset italic') : t('Set italic');
        italic.setAttribute('aria-pressed', String(Boolean(isItalic)));
        italic.addEventListener('click', () => this.toggleItalic());
        const bold = el('button', { type: 'button', className: 'icon tool text-style text-bold', textContent: 'B', disabled: !face });
        const isBold = face && (face.weight > 450 || Math.abs(s.boldness || 0) > 1e-9);
        bold.title = isBold ? t('Unset bold') : t('Set bold');
        bold.setAttribute('aria-pressed', String(Boolean(isBold)));
        bold.addEventListener('click', () => this.toggleBold());
        fontLine.push(italic, bold);
      }
      rows.push(this.row(t('Font'), fontLine));
      // "Height", "Depth".
      rows.push(this.row(t('Height'), [this.number(s, 'size', 0.1, LIMITS.size, 'mm', locked)]));
      rows.push(this.row(t('Depth'), [this.number(s, 'depth', 0.1, LIMITS.depth, 'mm', locked)]));
      // "Advanced".
      const advanced = el('details', { className: 'text-advanced', open: Boolean(this.advancedOpen) && !locked });
      advanced.addEventListener('toggle', () => { this.advancedOpen = advanced.open; });
      advanced.append(el('summary', { textContent: t('Advanced') }));
      const notYet = t('Text that follows the surface ("Use surface") cannot be made here yet.');
      const surface = el('input', { type: 'checkbox', checked: Boolean(s.use_surface), disabled: !s.use_surface || locked });
      surface.addEventListener('change', () => { s.use_surface = surface.checked; this.changed(true); });
      const surfaceLabel = el('label', { className: 'edit-check', title: notYet }, surface, ` ${t('Use surface')}`);
      const glyph = el('input', { type: 'checkbox', checked: Boolean(s.per_glyph), disabled: !s.per_glyph || locked });
      glyph.addEventListener('change', () => { s.per_glyph = glyph.checked; this.changed(true); });
      const glyphLabel = el('label', { className: 'edit-check', title: t('Text placed letter by letter ("Per glyph") cannot be made here yet.') }, glyph, ` ${t('Per glyph')}`);
      advanced.append(el('div', { className: 'text-checks' }, surfaceLabel, glyphLabel));
      // "Alignment": three across, three down.
      const align = (key, value, label, path) => {
        const button = el('button', { type: 'button', className: 'icon tool', title: label, disabled: locked });
        button.setAttribute('aria-label', label);
        button.setAttribute('aria-pressed', String(s[key] === value));
        button.append(icon(path));
        button.addEventListener('click', () => { s[key] = value; this.changed(true); });
        return button;
      };
      advanced.append(this.row(t('Alignment'), [
        align('horizontal', 'left', tc('Alignment', 'Left'), 'M2,3h12M2,6.5h7M2,10h12M2,13.5h7'),
        align('horizontal', 'center', tc('Alignment', 'Center'), 'M2,3h12M4.5,6.5h7M2,10h12M4.5,13.5h7'),
        align('horizontal', 'right', tc('Alignment', 'Right'), 'M2,3h12M7,6.5h7M2,10h12M7,13.5h7'),
        align('vertical', 'top', tc('Alignment', 'Top'), 'M2,2h12M8,5v9M5,8l3-3l3,3'),
        align('vertical', 'middle', tc('Alignment', 'Middle'), 'M2,8h12M8,2v4M8,10v4'),
        align('vertical', 'bottom', tc('Alignment', 'Bottom'), 'M2,14h12M8,2v9M5,8l3,3l3-3'),
      ]));
      // The gaps, boldness and skew in the font's units ("points"), within
      // half its ascent each way (`draw_advanced`).
      const half = face ? Math.round(face.ascent / 2) : 500;
      const points = `${t('points')}`;
      const multiline = s.text.includes('\n');
      advanced.append(this.slider(s, 'char_gap', t('Char gap'), [-half, half], 1, points, t('Distance between characters'), locked));
      advanced.append(this.slider(s, 'line_gap', t('Line gap'), [-half, half], 1, points, t('Distance between lines'), locked || !multiline));
      const ascent = face ? face.ascent : 1000;
      advanced.append(this.slider(s, 'boldness', t('Boldness'), [Math.trunc(ascent * LIMITS.boldness[0]), Math.trunc(ascent * LIMITS.boldness[1])], 1, points, t('Tiny / Wide glyphs'), locked));
      advanced.append(this.slider(s, 'skew', t('Skew ratio'), LIMITS.skew, 0.01, '', t('Italic strength ratio'), locked));
      // "From surface": up to twice the depth each way; not for an object
      // of one text, nor with "Use surface".
      const depth = Number(s.depth) || 1;
      const away = s.distance ?? 0;
      advanced.append(this.slider(s, 'distance', t('From surface'), [-2 * depth, 2 * depth], 0.01, 'mm', t('Distance of the center of the text to the model surface.'), locked || o.onlyPart || s.use_surface, away));
      advanced.append(this.slider(s, 'angle', t('Rotation'), LIMITS.angle, 0.01, '°', t('Rotate text Clock-wise.'), locked));
      // "Keep up" (the lock) and "Set text to face camera".
      const keep = el('input', { type: 'checkbox', checked: this.keepUp !== false, disabled: locked || o.onlyPart });
      keep.addEventListener('change', () => { this.keepUp = keep.checked; });
      const keepLabel = el('label', { className: 'edit-check', title: this.keepUp !== false ? t("Unlock the text's rotation when moving text along the object's surface.") : t("Lock the text's rotation when moving text along the object's surface.") }, keep, ` ${t('Keep up')}`);
      const faceCamera = el('button', { type: 'button', textContent: t('Set text to face camera'), title: t('Orient the text towards the camera.'), disabled: locked });
      faceCamera.addEventListener('click', () => this.faceCamera());
      advanced.append(el('div', { className: 'text-checks' }, keepLabel, faceCamera));
      rows.push(advanced);
      // "Operation": not for an object of one text (`is_the_only_one_part`).
      if (!o.onlyPart || o.kind !== 'part') {
        const kinds = el('div', { className: 'joined segmented text-kinds' });
        kinds.setAttribute('role', 'group');
        kinds.setAttribute('aria-label', t('Operation'));
        for (const [value, label, tip] of [['part', t('Join'), t('Click to change text into object part.')], ['negative', tc('EmbossOperation', 'Cut'), t('Click to change part type into negative volume.')], ['modifier', t('Modifier'), t('Click to change part type into modifier.')]]) {
          const button = el('button', { type: 'button', textContent: label, title: tip, disabled: locked });
          button.setAttribute('aria-pressed', String(o.kind === value));
          button.addEventListener('click', () => { if (o.kind !== value) { o.kind = value; this.changed(true); } });
          kinds.append(button);
        }
        rows.push(el('div', { className: 'cut-row' }, el('span', { className: 'cut-part', textContent: t('Operation') }), el('span', { className: 'cut-line' }, kinds)));
      }
      if (typing) {
        // Everything but the box itself, which keeps its caret.
        const keepNode = input;
        node.replaceChildren(...rows.map((row) => (row === input ? keepNode : row)));
        keepNode.focus();
      } else {
        node.replaceChildren(...rows);
      }
    },

    row(label, controls) {
      const { el } = this.host;
      return el('div', { className: 'cut-row' }, el('span', { className: 'cut-part', textContent: label }), el('span', { className: 'cut-line' }, ...controls));
    },

    // A number of the style, in its limits, with its unit.
    number(s, key, step, [lo, hi], unit, disabled) {
      const { el } = this.host;
      const input = el('input', { type: 'number', className: 'text-number', min: lo, max: hi, step, value: round(Number(s[key]), 2), disabled });
      input.addEventListener('change', () => {
        const value = Number(input.value);
        if (!Number.isFinite(value)) { input.value = s[key]; return; }
        s[key] = Math.min(Math.max(value, lo), hi);
        this.changed(true);
      });
      return el('span', { className: 'text-unit' }, input, el('span', { className: 'quiet', textContent: unit }));
    },

    // A slider of the style with its value, made again when let go.
    slider(s, key, label, [lo, hi], step, unit, tip, disabled, shown = s[key]) {
      const { el } = this.host;
      const value = Number(shown) || 0;
      const range = el('input', { type: 'range', min: lo, max: hi, step, value, disabled });
      range.setAttribute('aria-label', label);
      const out = el('span', { className: 'simplify-value', textContent: `${round(value, 2)}${unit ? ` ${unit}` : ''}` });
      range.addEventListener('input', () => { out.textContent = `${round(Number(range.value), 2)}${unit ? ` ${unit}` : ''}`; });
      range.addEventListener('change', () => {
        s[key] = Number(range.value);
        if (this.open) this.open.touched[key] = true;
        this.changed(true);
      });
      const row = this.row(label, [range, out]);
      row.title = tip;
      return row;
    },
  };

  const round = (value, digits) => {
    const power = 10 ** digits;
    const rounded = Math.round((Number(value) || 0) * power) / power;
    return Object.is(rounded, -0) ? 0 : rounded;
  };

  const icon = (path) => {
    const node = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    node.setAttribute('viewBox', '0 0 16 16');
    node.setAttribute('aria-hidden', 'true');
    const line = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    line.setAttribute('d', path);
    node.append(line);
    return node;
  };

  GL.texter = texter;
})(window.EdgeGL);
