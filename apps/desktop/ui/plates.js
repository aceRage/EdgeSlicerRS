'use strict';

// A plate as a plate: its menu, its name, its lock, its own settings, and
// its tools beside it in the 3D scene. What the reference offers: the
// icons it draws beside each plate (PartPlate.cpp `render_icons`, acted on
// by Plater.cpp `select_plate_by_hover_id`), the plate's menu
// (GUI_Factories.cpp `create_plate_menu`, `append_menu_item_locked`,
// `append_menu_item_plate_name`), "Duplicate Current Plate" (the Edit
// menu, MainFrame.cpp), "Edit Plate Name" and "Plate Settings"
// (PlateSettingsDialog.cpp, `Plater::open_platesettings_dialog`).
//
// Every change is one `scene_edit` (scene-edit.js `commit`): the session
// answers with the project and its scene as they then are.
(function (GL) {
  const ICONS = {
    remove: 'M2.5,4.5h11M6,4.5v-2h4v2M4,4.5l1,9.5h6l1-9.5M6.8,7v4.5M9.2,7v4.5',
    arrange: 'M1.5,1.5h5v5h-5ZM9.5,1.5h5v5h-5ZM1.5,9.5h5v5h-5ZM9.5,9.5h5v5h-5Z',
    locked: 'M3.5,7.5h9v7h-9ZM5.5,7.5v-2.5a2.5,2.5,0,0,1,5,0v2.5',
    unlocked: 'M3.5,7.5h9v7h-9ZM5.5,7.5v-2.5a2.5,2.5,0,0,1,5,0',
    settings: 'M2,4.5h7M12,4.5h2M2,11.5h2M7,11.5h7M9,2.5v4h3v-4ZM4,9.5v4h3v-4Z',
    name: 'M2.5,13.5l1-3.5l7.5-7.5l2.5,2.5l-7.5,7.5ZM9.5,4l2.5,2.5',
    front: 'M8,13.5v-9M4.5,8l3.5-3.5l3.5,3.5M2.5,2h11',
    add: 'M8,3v10M3,8h10',
    remove_row: 'M3,8h10',
  };
  // The layer that stands for "End" in a range (PlateSettingsDialog.cpp
  // `MAX_LAYER_VALUE`): sent as no number.
  const LONGEST_NAME = 250;

  // Sets a list's value where its closed face (app.js `listbox`) sees it:
  // the face follows what is done to the <select>'s attributes.
  const setValue = (select, value) => {
    select.value = value;
    select.dataset.value = value;
  };

  const svg = (path) => {
    const node = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    node.setAttribute('viewBox', '0 0 16 16');
    node.setAttribute('aria-hidden', 'true');
    const line = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    line.setAttribute('d', path);
    node.append(line);
    return node;
  };

  const plates = {
    host: null,

    init(host) {
      this.host = host;
      this.buildTools();
      this.buildName();
      this.buildSettings();
      GL.plates = this;
    },

    // ---- what there is ----

    project() { return this.host.project(); },
    edit() { return GL.editor; },
    listed(id) {
      const project = this.project();
      return project && (project.plates || []).find((plate) => plate.id === id);
    },
    // The copies on a plate, by id, as the scene places them.
    copiesOn(id) {
      const data = this.edit() && this.edit().data();
      return data ? data.placings.filter((placing) => placing.plate === id).map((placing) => placing.id) : [];
    },
    // What the plate is called where it is named: "Plate 2", its name after it.
    label(plate) {
      return plate.name ? `${t('Plate')} ${plate.id} (${plate.name})` : `${t('Plate')} ${plate.id}`;
    },
    commit(args, more) { return this.edit().commit(args, more); },
    usable() {
      const edit = this.edit();
      return Boolean(edit && edit.editing() && !edit.busy && !this.project().empty);
    },

    // ---- the plate's menu ----

    // The plate's menu, at a point: the reference's plate menu with the
    // plate's lock and name (`plate_menu`), then what its icons beside
    // the plate do and "Duplicate Current Plate", there being no Edit
    // menu here to hold it.
    menu(x, y, id) {
      const plate = this.listed(id);
      if (!plate || !this.edit()) return;
      const project = this.project();
      const on = this.copiesOn(id);
      const count = project.plates.length;
      this.host.menu(x, y, [
        [t('Select All'), () => this.host.stage.select(on), { disabled: !on.length, title: t('select all objects on current plate') }],
        [t('Delete All'), () => this.commit({ op: 'delete', copies: on }), { disabled: !on.length, title: t('delete all objects on current plate') }],
        [t('Arrange'), () => this.commit({ op: 'arrange', plate: id }), { disabled: !on.length, title: t('arrange current plate') }],
        [t('Delete Plate'), () => this.commit({ op: 'delete_plate', plate: id }), { disabled: count < 2, title: t('Remove the selected plate') }],
        null,
        [plate.locked ? t('Unlock') : t('Lock'), () => this.commit({ op: 'plate_lock', plate: id, value: !plate.locked })],
        [t('Edit Plate Name'), () => this.rename(id)],
        [t('Customize current plate'), () => this.customize(id)],
        [t('Move plate to the front'), () => this.commit({ op: 'plate_front', plate: id }), { disabled: id === 1 }],
        [t('Duplicate Current Plate'), () => this.commit({ op: 'plate_duplicate', plate: id }, { select: true }), { title: t('Duplicate the current plate') }],
      ]);
    },

    // The menu from the keys, under what has them (Shift+F10, the menu key).
    keyMenu(event, node, id) {
      if (event.key !== 'ContextMenu' && !(event.key === 'F10' && event.shiftKey)) return;
      event.preventDefault();
      const box = node.getBoundingClientRect();
      this.menu(box.left, box.bottom, id);
    },

    // ---- its tools beside it in the scene ----

    buildTools() {
      const { el } = this.host;
      const tool = (id, label, path, act) => {
        const node = el('button', { type: 'button', className: 'icon tool', id: `plate-tool-${id}`, title: label });
        node.setAttribute('aria-label', label);
        node.append(svg(path));
        node.addEventListener('click', () => { if (this.shown) act(this.shown); });
        node.addEventListener('contextmenu', (event) => { event.preventDefault(); if (this.shown) this.menu(event.clientX, event.clientY, this.shown); });
        return node;
      };
      this.tools = {
        remove: tool('remove', t('Remove current plate (if not last one)'), ICONS.remove, (id) => this.commit({ op: 'delete_plate', plate: id })),
        arrange: tool('arrange', t('Arrange objects on current plate'), ICONS.arrange, (id) => this.commit({ op: 'arrange', plate: id })),
        lock: tool('lock', t('Lock current plate'), ICONS.unlocked, (id) => {
          const plate = this.listed(id);
          if (plate) this.commit({ op: 'plate_lock', plate: id, value: !plate.locked });
        }),
        settings: tool('settings', t('Customize current plate'), ICONS.settings, (id) => this.customize(id)),
        name: tool('name', t('Edit current plate name'), ICONS.name, (id) => this.rename(id)),
        front: tool('front', t('Move plate to the front'), ICONS.front, (id) => this.commit({ op: 'plate_front', plate: id })),
      };
      const tools = this.tools;
      this.bar = el('div', { className: 'toolbar vertical plate-tools', id: 'plate-tools', hidden: true },
        tools.remove, tools.arrange, tools.lock, tools.settings, tools.name, tools.front);
      this.bar.setAttribute('role', 'toolbar');
      this.bar.setAttribute('aria-label', t('Plate'));
      this.host.stageNode.append(this.bar);
    },

    // The tools as things are now: for the plate shown, while Prepare
    // edits a project with plates.
    refresh() {
      if (!this.bar) return;
      const project = this.project();
      const plate = project && this.edit() && this.edit().editing() && project.plates.length ? this.listed(project.plate) : null;
      this.shown = plate ? plate.id : null;
      if (!plate) { this.bar.hidden = true; return; }
      const tools = this.tools;
      const busy = !this.usable();
      tools.remove.disabled = busy || project.plates.length < 2;
      tools.arrange.disabled = busy || !this.copiesOn(plate.id).length;
      tools.lock.disabled = busy;
      tools.settings.disabled = busy;
      tools.name.disabled = busy;
      tools.front.disabled = busy || plate.id === 1;
      const lockLabel = plate.locked ? t('Unlock current plate') : t('Lock current plate');
      tools.lock.title = lockLabel;
      tools.lock.setAttribute('aria-label', lockLabel);
      tools.lock.setAttribute('aria-pressed', String(Boolean(plate.locked)));
      tools.lock.replaceChildren(svg(plate.locked ? ICONS.locked : ICONS.unlocked));
      // A plate with settings of its own shows it on its tool, as the
      // reference draws its "changed" icon.
      tools.settings.classList.toggle('set', Boolean(plate.customized));
      this.bar.dataset.plate = String(plate.id);
    },

    // Stands the tools beside the plate shown, at its far right corner,
    // as the reference stands its icons down the plate's right side.
    // Called each time the scene is drawn.
    place(frame) {
      if (!this.bar) return;
      this.refresh();
      if (!this.shown) return;
      const scene = this.host.stage.scene;
      const parts = scene.plate(this.shown);
      if (!parts || !parts.box) { this.bar.hidden = true; return; }
      const box = parts.box;
      const at = this.edit().toView(frame, [box[2], box[3], 0]);
      const near = this.edit().toView(frame, [box[2], box[1], 0]);
      const [width, height] = this.host.stage.renderer.css;
      const room = this.bar.offsetHeight || 200;
      // At the corner seen highest of the plate's right side.
      const top = Math.min(at[1], near[1]);
      const left = Math.max(at[0], near[0]) + 8;
      const inside = left > 0 && left < width - 40 && top < height - 40 && top + room > 0;
      this.bar.hidden = !inside;
      if (!inside) return;
      this.bar.style.left = `${Math.round(Math.min(left, width - 44))}px`;
      this.bar.style.top = `${Math.round(Math.max(8, Math.min(top, height - room - 8)))}px`;
    },

    // ---- "Edit Plate Name" ----

    buildName() {
      const { el } = this.host;
      this.nameField = el('input', { type: 'text', id: 'plate-name-text', maxLength: LONGEST_NAME, autocomplete: 'off', spellcheck: false });
      this.nameDialog = el('dialog', { id: 'plate-name', className: 'plate-name' },
        el('form', { method: 'dialog' },
          el('h2', { textContent: t('Edit Plate Name') }),
          el('label', { htmlFor: 'plate-name-text', textContent: t('Plate name') }),
          this.nameField,
          el('div', { className: 'row end' },
            el('button', { type: 'submit', value: 'ok', className: 'primary', textContent: t('OK') }),
            el('button', { type: 'submit', value: 'cancel', textContent: t('Cancel') }))));
      document.body.append(this.nameDialog);
    },

    ask(dialog) {
      return new Promise((resolve) => {
        dialog.returnValue = 'cancel';
        dialog.addEventListener('close', () => resolve(dialog.returnValue === 'ok'), { once: true });
        dialog.showModal();
      });
    },

    // Names a plate (action 6 of `select_plate_by_hover_id`: the plate is
    // shown first). An empty name is no name.
    async rename(id) {
      const plate = this.listed(id);
      if (!plate || !this.usable()) return;
      if (id !== this.project().plate) await this.host.plate(id);
      this.nameField.value = plate.name || '';
      const answered = this.ask(this.nameDialog);
      this.nameField.focus();
      this.nameField.setSelectionRange(this.nameField.value.length, this.nameField.value.length);
      if (!(await answered)) return;
      await this.commit({ op: 'plate_rename', plate: id, name: this.nameField.value });
    },

    // ---- "Plate Settings" ("Customize current plate") ----

    buildSettings() {
      const { el, listbox } = this.host;
      const choice = (id, values) => {
        const select = el('select', { id });
        for (const [value, label] of values) select.append(el('option', { value, textContent: label }));
        return select;
      };
      const s = this.settingsFields = {
        name: el('input', { type: 'text', id: 'ps-name', maxLength: LONGEST_NAME, autocomplete: 'off', spellcheck: false }),
        bed: choice('ps-bed', [['', t('Same as Global Plate Type')]]),
        sequence: choice('ps-sequence', [['', t('Same as Global Print Sequence')], ['by layer', t('By Layer')], ['by object', t('By Object')]]),
        spiral: choice('ps-spiral', [['', t('Same as Global')], ['on', t('Enable')], ['off', t('Disable')]]),
        first: choice('ps-first', [['auto', t('Auto')], ['custom', t('Customize')]]),
        other: choice('ps-other', [['auto', t('Auto')], ['custom', t('Customize')]]),
      };
      const row = (label, id, field) => el('div', { className: 'card-row' }, el('label', { htmlFor: id, textContent: label }), field);
      this.firstOrder = el('div', { className: 'ps-order-row' });
      this.ranges = el('div', { className: 'ps-ranges' });
      const add = el('button', { type: 'button', className: 'icon tool', title: t('Add') });
      add.setAttribute('aria-label', 'Add a range of layers');
      add.append(svg(ICONS.add));
      add.addEventListener('click', () => this.addRange());
      const drop = el('button', { type: 'button', className: 'icon tool', title: t('Delete') });
      drop.setAttribute('aria-label', 'Delete the last range of layers');
      drop.append(svg(ICONS.remove_row));
      drop.addEventListener('click', () => { if (this.ranges.children.length > 1) this.ranges.lastChild.remove(); });
      this.rangeTools = el('div', { className: 'ps-range-tools' }, add, drop);
      this.settingsProblem = el('p', { className: 'error grow', id: 'ps-problem' });
      this.settingsDialog = el('dialog', { id: 'plate-settings', className: 'plate-settings' },
        el('form', { method: 'dialog' },
          el('h2', { id: 'ps-title', textContent: t('Plate Settings') }),
          el('div', { className: 'card' },
            row(t('Plate name'), 'ps-name', s.name),
            row(t('Bed type'), 'ps-bed', listbox(s.bed, { bare: true })),
            row(t('Print sequence'), 'ps-sequence', listbox(s.sequence, { bare: true })),
            row(t('Spiral vase'), 'ps-spiral', listbox(s.spiral, { bare: true })),
            row(t('First layer filament sequence'), 'ps-first', listbox(s.first, { bare: true })),
            this.firstOrder,
            row(t('Other layer filament sequence'), 'ps-other', listbox(s.other, { bare: true })),
            this.rangeTools,
            this.ranges),
          el('div', { className: 'row end' },
            this.settingsProblem,
            el('button', { type: 'submit', value: 'ok', className: 'primary', textContent: t('OK') }),
            el('button', { type: 'submit', value: 'cancel', textContent: t('Cancel') }))));
      this.settingsDialog.setAttribute('aria-labelledby', 'ps-title');
      s.first.addEventListener('change', () => this.showOrders());
      s.other.addEventListener('change', () => {
        if (s.other.value === 'custom' && !this.ranges.children.length) this.addRange();
        this.showOrders();
      });
      // OK checks the ranges first, as the reference's does, and stays
      // open with what is wrong.
      this.settingsDialog.querySelector('button[value="ok"]').addEventListener('click', (event) => {
        const problem = this.rangeProblem();
        if (problem) {
          event.preventDefault();
          this.settingsProblem.textContent = problem;
        }
      });
      document.body.append(this.settingsDialog);
    },

    // The filaments of the project, each with its colour, for an order.
    filaments() {
      const count = this.edit() ? this.edit().filamentCount() : 1;
      const colours = this.project().colours || [];
      return Array.from({ length: count }, (_, at) => ({ number: at + 1, colour: colours[at] || null }));
    },

    // An order of filaments to rearrange (the reference's `DragCanvas`): a
    // swatch for each, dragged into place, or moved with Left and Right.
    orderStrip(order) {
      const { el } = this.host;
      const filaments = this.filaments();
      const known = new Map(filaments.map((filament) => [filament.number, filament]));
      // Each filament once, in the order given, the rest after it.
      const numbers = order.filter((number) => known.has(number));
      for (const filament of filaments) if (!numbers.includes(filament.number)) numbers.push(filament.number);
      const strip = el('div', { className: 'ps-order', role: 'list' });
      strip.setAttribute('aria-label', 'Filament order');
      const name = (chip) => {
        const at = [...strip.children].indexOf(chip) + 1;
        chip.setAttribute('aria-label', `Filament ${chip.dataset.filament}, ${at} of ${strip.children.length}`);
      };
      const rename = () => [...strip.children].forEach(name);
      for (const number of numbers) {
        const chip = el('button', { type: 'button', className: 'ps-chip', textContent: String(number) });
        chip.dataset.filament = String(number);
        chip.setAttribute('role', 'listitem');
        chip.title = 'Drag, or Left and Right, to put it in its place';
        const colour = known.get(number).colour;
        const swatch = el('span', { className: 'swatch' });
        if (colour) swatch.style.background = colour;
        chip.prepend(swatch);
        chip.addEventListener('keydown', (event) => {
          if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
          event.preventDefault();
          if (event.key === 'ArrowLeft' && chip.previousElementSibling) strip.insertBefore(chip, chip.previousElementSibling);
          if (event.key === 'ArrowRight' && chip.nextElementSibling) strip.insertBefore(chip.nextElementSibling, chip);
          chip.focus();
          rename();
        });
        chip.addEventListener('pointerdown', (event) => {
          if (event.button !== 0) return;
          chip.setPointerCapture(event.pointerId);
          chip.classList.add('held');
          const move = (moved) => {
            const over = document.elementFromPoint(moved.clientX, moved.clientY);
            const target = over && over.closest('.ps-chip');
            if (!target || target === chip || target.parentNode !== strip) return;
            const box = target.getBoundingClientRect();
            strip.insertBefore(chip, moved.clientX < box.left + box.width / 2 ? target : target.nextElementSibling);
          };
          const up = () => {
            chip.classList.remove('held');
            chip.removeEventListener('pointermove', move);
            chip.removeEventListener('pointerup', up);
            chip.removeEventListener('pointercancel', up);
            rename();
          };
          chip.addEventListener('pointermove', move);
          chip.addEventListener('pointerup', up);
          chip.addEventListener('pointercancel', up);
          chip.addEventListener('lostpointercapture', up, { once: true });
        });
        strip.append(chip);
      }
      rename();
      return strip;
    },

    orderOf(strip) {
      return [...strip.querySelectorAll('.ps-chip')].map((chip) => Number(chip.dataset.filament));
    },

    // One range of later layers: "Layer" [from] "to" [to, or "End"] and
    // its order (`OtherLayersSeqPanel::append_layer`).
    addRange(range = null) {
      const { el, listbox } = this.host;
      const at = this.ranges.children.length;
      const begin = el('input', { type: 'number', min: '2', step: '1', className: 'ps-layer', id: `ps-begin-${at}` });
      begin.value = range ? String(range.begin) : at ? '' : '2';
      const endKind = el('select', { id: `ps-end-kind-${at}` }, el('option', { value: 'end', textContent: t('End') }), el('option', { value: 'custom', textContent: t('Customize') }));
      const end = el('input', { type: 'number', min: '2', step: '1', className: 'ps-layer', id: `ps-end-${at}` });
      const custom = range && range.end !== null && range.end !== undefined;
      setValue(endKind, custom ? 'custom' : 'end');
      end.value = custom ? String(range.end) : '';
      end.hidden = !custom;
      endKind.addEventListener('change', () => { end.hidden = endKind.value !== 'custom'; if (!end.hidden) end.focus(); });
      begin.setAttribute('aria-label', 'From layer');
      end.setAttribute('aria-label', 'To layer');
      const order = this.orderStrip(range ? range.order : this.filaments().map((filament) => filament.number));
      const node = el('div', { className: 'ps-range' },
        el('span', { className: 'ps-range-head' },
          el('label', { htmlFor: begin.id, textContent: t('Layer') }), begin,
          el('span', { textContent: t('to') }), listbox(endKind, { bare: true }), end),
        order);
      node.fields = { begin, endKind, end, order };
      this.ranges.append(node);
    },

    // What is wrong with the ranges as typed, in the reference's words.
    rangeProblem() {
      if (this.settingsFields.other.value !== 'custom') return '';
      for (const range of this.ranges.children) {
        const { begin, endKind, end } = range.fields;
        const low = Number(begin.value);
        if (!begin.value || !Number.isInteger(low) || low < 2) return t('Please input layer value (>= 2).');
        if (endKind.value === 'custom') {
          const high = Number(end.value);
          if (!end.value || !Number.isInteger(high) || high < 2) return t('Please input layer value (>= 2).');
        }
      }
      return '';
    },

    showOrders() {
      const s = this.settingsFields;
      this.firstOrder.hidden = s.first.value !== 'custom';
      const custom = s.other.value === 'custom';
      this.ranges.hidden = !custom;
      this.rangeTools.hidden = !custom;
    },

    // Opens "Plate Settings" for a plate (action 5 of
    // `select_plate_by_hover_id`: the plate is shown first) and makes what
    // was changed: the settings as one step, the name as another, as the
    // reference renames the plate after its dialog closes.
    async customize(id) {
      if (!this.listed(id) || !this.usable()) return;
      if (id !== this.project().plate) await this.host.plate(id);
      const plate = this.listed(id);
      const own = plate.own || {};
      const s = this.settingsFields;
      const { el } = this.host;
      s.name.value = plate.name || '';
      // The printer's plates; a plate's own that is not among them is
      // shown as it is named.
      const settled = this.host.settled();
      const types = settled && settled.printer && settled.printer.bed_types;
      const choices = types ? types.choices.map((choice) => [choice.value, t(choice.label)]) : [];
      if (own.bed_type && !choices.some(([value]) => value === own.bed_type)) choices.push([own.bed_type, own.bed_type]);
      s.bed.replaceChildren(el('option', { value: '', textContent: t('Same as Global Plate Type') }),
        ...choices.map(([value, label]) => el('option', { value, textContent: label })));
      setValue(s.bed, own.bed_type || '');
      s.bed.disabled = !(types && types.enabled);
      setValue(s.sequence, own.print_sequence || '');
      setValue(s.spiral, own.spiral_mode === true ? 'on' : own.spiral_mode === false ? 'off' : '');
      const first = own.first_layer_sequence || [];
      setValue(s.first, first.length ? 'custom' : 'auto');
      this.firstOrder.replaceChildren(this.orderStrip(first));
      const ranges = own.other_layers_sequence || [];
      setValue(s.other, ranges.length ? 'custom' : 'auto');
      this.ranges.replaceChildren();
      for (const range of ranges) this.addRange(range);
      this.settingsProblem.textContent = '';
      for (const select of Object.values(s)) select.dispatchEvent(new Event('change'));
      this.showOrders();
      if (!(await this.ask(this.settingsDialog))) return;

      const asked = {
        op: 'plate_settings',
        plate: id,
        bed_type: s.bed.value || null,
        print_sequence: s.sequence.value || null,
        spiral_mode: s.spiral.value === 'on' ? true : s.spiral.value === 'off' ? false : null,
        first_layer_sequence: s.first.value === 'custom' ? this.orderOf(this.firstOrder) : [],
        other_layers_sequence: s.other.value === 'custom'
          ? [...this.ranges.children].map((range) => ({
            begin: Number(range.fields.begin.value),
            end: range.fields.endKind.value === 'custom' ? Number(range.fields.end.value) : null,
            order: this.orderOf(range.fields.order),
          }))
          : [],
      };
      const name = s.name.value;
      if (!(await this.commit(asked))) return;
      if (name !== (plate.name || '')) await this.commit({ op: 'plate_rename', plate: id, name });
    },
  };

  GL.plateTools = plates;
})(window.EdgeGL);
