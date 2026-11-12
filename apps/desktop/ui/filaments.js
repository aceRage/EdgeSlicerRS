'use strict';

// The sidebar's list of filaments as an editor (Plater.cpp `Sidebar`): a
// row for each filament with its colour ("Click to select filament
// color"), its preset and its pages; "Add one filament" and "Remove last
// filament" in the head; a row's menu ("Edit", "Delete", "Merge with");
// "Apply All"; "Flushing volumes" and its dialog. Every change is one
// `filament_edit`, which the session answers as it answers an edit of the
// scene, so it is taken as one.
//
// Loaded before app.js, whose helpers (`$`, `el`, `call`, `attempt`,
// `showMenu`, `listbox`, `fill`, `svg`) it uses when it runs.
const Filaments = (() => {
  // The reference's most (libslic3r.h `MAXIMUM_FILAMENT_NUMBER`).
  const MAX = 64;
  // The colours the picker offers to begin with: the reference's palette
  // for a filament added (Plater.cpp `get_next_color_for_filament`), with
  // white, greys and black.
  const PALETTE = [
    '#00C1AE', '#F4E2C1', '#ED1C24', '#00FF7F', '#F26722', '#FFEB31', '#7841CE', '#115877',
    '#ED1E79', '#2EBDEF', '#345B2F', '#800080', '#FA8173', '#800000', '#F7B763', '#A4C41E',
    '#FFFFFF', '#C0C0C0', '#808080', '#000000',
  ];
  // The most a box of the flushing volumes takes (WipeTowerDialog.cpp
  // `MAX_FLUSH_VALUE`).
  const FLUSH_MOST = 2147483520;
  const PLUS = 'M8,3.5v9M3.5,8h9';
  const MINUS = 'M3.5,8h9';

  let shown = null;          // the project the rows were made for

  // A colour the page can show: `#RRGGBB` of what the project holds.
  const plain = (colour) => (/^#[0-9a-f]{6}/i.test(colour || '') ? colour.slice(0, 7).toUpperCase() : null);

  // What a filament is called in a menu: its preset's name, else its number
  // (GUI_Factories.cpp `create_filament_action_menu`).
  function presetName(project, index) {
    const choices = project.choices;
    if (choices.source === 'presets') {
      const named = choices.filaments;
      return named.length ? named[Math.min(index, named.length - 1)] : null;
    }
    return (project.saved.filaments || [])[index] || null;
  }

  // Makes an edit and shows what came of it, as an edit of the scene is.
  async function edit(args) {
    const project = state.project;
    if (!project) return;
    const answer = await call('filament_edit', { project: project.id, ...args });
    if (answer.done === null) return;
    // Taken as an edit of the scene is: the sidebar and the object list
    // are drawn again with the filaments as they now are (`editApplied`).
    window.EdgeGL.editor.take(answer);
  }

  // ---- the head ----

  function head(project) {
    const count = project.filament_count;
    const add = $('filament-add');
    const remove = $('filament-remove');
    add.disabled = count >= MAX;
    // There while there is more than one to take away, as in the reference.
    remove.hidden = count <= 1;
    // "Flushing volumes" too (Plater.cpp `on_filaments_change`).
    $('flush-open').hidden = count <= 1;
  }

  // ---- "Flushing volumes" (WipeTowerDialog.cpp `WipingDialog`) ----
  //
  // A box for each pair, from a filament (a row) to another (a column),
  // shows its volume times the multiplier, cut to a whole number, as the
  // reference shows it. The volumes are kept as they are before the
  // multiplier: one typed in is that number over the multiplier, and one
  // not touched is kept exactly. (The reference reads every box back over
  // the multiplier on OK, so a dialog opened and closed with OK rounds
  // every volume; and with the multiplier at 0 it keeps nothing. Neither
  // is done here: a box not touched keeps its volume, and with the
  // multiplier at 0 the boxes cannot be typed in.)

  let flush = null;          // { project, count, colours, matrix, multiplier, minimums, ... }

  const shownVolume = (volume, multiplier) => Math.trunc(volume * multiplier + 1e-9);

  function flushBoxes() {
    return [...$('flush-table').querySelectorAll('input')];
  }

  // The boxes from the volumes and the multiplier.
  function flushFill() {
    const { count, matrix, multiplier } = flush;
    for (const box of flushBoxes()) {
      const from = Number(box.dataset.from);
      const to = Number(box.dataset.to);
      if (from === to) continue;
      box.value = String(shownVolume(matrix[from * count + to], multiplier));
      box.readOnly = multiplier === 0;
    }
    flushWarn();
  }

  // A volume under what its filament flushes at least, or over the most,
  // in --danger, and the suggestion with it (`update_warning_texts`; the
  // least is the filament flushed out's, as "Re-calculate" takes it,
  // where the reference measures against the one flushed in).
  function flushWarn() {
    let any = false;
    for (const box of flushBoxes()) {
      const from = Number(box.dataset.from);
      if (from === Number(box.dataset.to)) continue;
      const value = Number(box.value);
      const out = value < (flush.minimums[from] || 0) || value > flush.max_volume;
      box.classList.toggle('out', out);
      box.setAttribute('aria-invalid', String(out));
      any ||= out;
    }
    $('flush-suggest').classList.toggle('out', any);
  }

  // A box typed in: a whole number, at least 0 (`EVT_TEXT`), kept over
  // the multiplier (`on_apply_text_modify`).
  function flushTyped(box) {
    const { count, multiplier } = flush;
    const from = Number(box.dataset.from);
    const to = Number(box.dataset.to);
    let value = parseInt(box.value, 10);
    if (!Number.isFinite(value) || value < 0) value = 0;
    value = Math.min(value, FLUSH_MOST);
    box.value = String(value);
    if (multiplier > 0 && value !== shownVolume(flush.matrix[from * count + to], multiplier)) {
      flush.matrix[from * count + to] = value / multiplier;
    }
    flushWarn();
  }

  // The multiplier typed in: from 0 to 3, or put there and said.
  async function flushMultiplier() {
    const field = $('flush-multiplier');
    let value = parseFloat(field.value.replace(',', '.'));
    if (!Number.isFinite(value)) value = flush.multiplier;
    const most = flush.max_multiplier;
    if (value < 0 || value > most) {
      value = value < 0 ? 0 : most;
      field.value = value.toFixed(2);
      await inform(t('Warning'), fmt(t('The multiplier should be in range [%.2f, %.2f].'), 0, most));
    }
    flush.multiplier = value;
    field.value = value.toFixed(2);
    flushFill();
  }

  function flushTable() {
    const { count, colours } = flush;
    const swatchOf = (index) => {
      const colour = plain(colours[index]);
      const swatch = el('span', { className: 'swatch', textContent: String(index + 1) });
      if (colour) {
        swatch.style.background = colour;
        swatch.style.color = brightness(colour) > 0.55 ? '#1c2026' : '#ffffff';
      }
      return swatch;
    };
    const corner = el('th', { className: 'flush-corner', scope: 'col' },
      el('span', { className: 'to', textContent: t('To') }),
      el('span', { className: 'from', textContent: t('From') }));
    const head = el('tr', {}, corner);
    for (let to = 0; to < count; to += 1) {
      const cell = el('th', { scope: 'col' }, swatchOf(to));
      cell.setAttribute('aria-label', `${t('To')} ${fmt(t('Filament %d'), to + 1)}`);
      head.append(cell);
    }
    const body = el('tbody');
    for (let from = 0; from < count; from += 1) {
      const row = el('tr', {}, el('th', { scope: 'row', ariaLabel: `${t('From')} ${fmt(t('Filament %d'), from + 1)}` }, swatchOf(from)));
      for (let to = 0; to < count; to += 1) {
        const box = el('input', { type: 'text', inputMode: 'numeric', autocomplete: 'off', spellcheck: false });
        box.dataset.from = String(from);
        box.dataset.to = String(to);
        box.setAttribute('aria-label', `${t('From')} ${fmt(t('Filament %d'), from + 1)} ${t('To')} ${fmt(t('Filament %d'), to + 1)}`);
        if (from === to) {
          box.value = '0';
          box.readOnly = true;
          box.tabIndex = -1;
        } else {
          box.addEventListener('change', () => flushTyped(box));
          box.addEventListener('keydown', (event) => {
            if (event.key === 'Enter') { event.preventDefault(); flushTyped(box); }
          });
        }
        row.append(el('td', {}, box));
      }
      body.append(row);
    }
    $('flush-table').replaceChildren(el('thead', {}, head), body);
  }

  async function openFlush() {
    const project = state.project;
    if (!project) return;
    const shown = await call('flush_volumes', { project: project.id });
    flush = { project: project.id, ...shown, matrix: shown.matrix.slice() };
    $('flush-multiplier').value = flush.multiplier.toFixed(2);
    // The least any filament flushes, and the most a flush typically is
    // (Orca #11778: not the most a box takes).
    const least = Math.min(...flush.minimums);
    $('flush-suggest').textContent = fmt(t('Suggestion: Flushing Volume in range [%d, %d]'), Number.isFinite(least) ? least : 0, flush.typical_max);
    $('flush-error').hidden = true;
    flushTable();
    flushFill();
    $('flush-dialog').showModal();
  }

  async function flushCalc() {
    const answer = await call('flush_calc', { project: flush.project, colours: flush.colours });
    flush.matrix = answer.matrix.slice();
    flushFill();
  }

  async function flushOk() {
    // What was typed and not yet left.
    const focused = document.activeElement;
    if (focused && focused.closest && focused.closest('#flush-table')) flushTyped(focused);
    if (focused === $('flush-multiplier') && focused.value !== flush.multiplier.toFixed(2)) await flushMultiplier();
    const { project, matrix, multiplier } = flush;
    try {
      const answer = await call('filament_edit', { project, op: 'flush', matrix, multiplier });
      $('flush-dialog').close();
      if (answer.done !== null) window.EdgeGL.editor.take(answer);
    } catch (error) {
      const shown = $('flush-error');
      shown.textContent = String(error.message || error);
      shown.hidden = false;
    }
  }

  // ---- a row ----

  function row(project, index, name) {
    const colour = plain((project.colours || [])[index]);
    const swatch = el('button', {
      type: 'button',
      className: 'swatch swatch-pick',
      textContent: String(index + 1),
      title: t('Click to select filament color'),
      ariaLabel: `${fmt(t('Filament %d'), index + 1)}: ${t('Color')}${colour ? ` ${colour}` : ''}`,
    });
    if (colour) {
      swatch.style.background = colour;
      swatch.style.color = brightness(colour) > 0.55 ? '#1c2026' : '#ffffff';
    }
    swatch.addEventListener('click', () => openPicker(swatch, index, colour || '#F2754E'));
    // Its own pages of settings ("Edit" in the reference's row menu).
    const editButton = el('button', { type: 'button', className: 'icon tool', title: t('Click to edit preset'), ariaLabel: `${fmt(t('Filament %d'), index + 1)}: ${t('Filament settings')}` }, svg(EDIT_ICON));
    editButton.addEventListener('click', () => attempt(() => openPresetPanel('filament', index + 1)));
    const node = el('div', { className: 'filament' }, swatch, name, editButton);
    node.addEventListener('contextmenu', (event) => {
      event.preventDefault();
      let { clientX: x, clientY: y } = event;
      // From the keys (the menu key, Shift+F10): beside the row.
      if (!x && !y) {
        const box = node.getBoundingClientRect();
        x = box.left + 24;
        y = box.bottom;
      }
      rowMenu(project, index, x, y);
    });
    return node;
  }

  // The row's menu (GUI_Factories.cpp `create_filament_action_menu`).
  function rowMenu(project, index, x, y) {
    const count = project.filament_count;
    const colours = project.colours || [];
    const targets = [];
    for (let other = 0; other < count; other += 1) {
      if (other === index) continue;
      const label = presetName(project, other) || fmt(t('Filament %d'), other + 1);
      targets.push([`${other + 1}  ${label}`, () => attempt(() => edit({ op: 'merge', filament: index + 1, into: other + 1 })),
        { swatch: plain(colours[other]) || 'transparent' }]);
    }
    showMenu(x, y, [
      [t('Edit'), () => attempt(() => openPresetPanel('filament', index + 1))],
      [t('Delete'), () => attempt(() => edit({ op: 'delete', filament: index + 1 })), { title: t('Delete this filament'), disabled: count <= 1 }],
      [t('Merge with'), null, { items: targets, disabled: targets.length === 0 }],
    ]);
  }

  // ---- "Apply All" ----

  function applyAll(project, offered) {
    const box = $('apply-all');
    const fromPresets = project.choices.source === 'presets';
    // Only where the filaments' presets are chosen here: a project on its
    // own settings has none to choose.
    box.hidden = !fromPresets || offered.length === 0;
    if (box.hidden) return;
    const list = $('apply-all-preset');
    const first = presetName(project, 0);
    // What was picked stays picked while it is still offered.
    const kept = list.value && offered.includes(list.value) && shown === project.id ? list.value : first;
    fill(list, offered, kept);
    shown = project.id;
  }

  // ---- the colour picker ----

  let picking = null;        // { index, from, hue, saturation, value }

  const hex = (n) => Math.round(n).toString(16).padStart(2, '0').toUpperCase();
  function fromHsv(h, s, v) {
    const f = (n) => {
      const k = (n + h / 60) % 6;
      return v - v * s * Math.max(0, Math.min(k, 4 - k, 1));
    };
    return `#${hex(f(5) * 255)}${hex(f(3) * 255)}${hex(f(1) * 255)}`;
  }
  function toHsv(colour) {
    const [r, g, b] = [1, 3, 5].map((at) => parseInt(colour.slice(at, at + 2), 16) / 255);
    const max = Math.max(r, g, b);
    const delta = max - Math.min(r, g, b);
    let h = 0;
    if (delta) {
      if (max === r) h = 60 * (((g - b) / delta) % 6);
      else if (max === g) h = 60 * ((b - r) / delta + 2);
      else h = 60 * ((r - g) / delta + 4);
    }
    return { hue: (h + 360) % 360, saturation: max ? delta / max : 0, value: max };
  }

  function picker() {
    let node = $('colour-picker');
    if (node) return node;
    node = el('div', { id: 'colour-picker', className: 'colour-picker', hidden: true });
    node.setAttribute('role', 'dialog');
    node.setAttribute('aria-label', t('Click to select filament color'));
    const title = el('h3', { id: 'cp-title' });
    const chips = el('div', { className: 'cp-chips', role: 'group', ariaLabel: t('Colours') },
      ...PALETTE.map((colour) => {
        const chip = el('button', { type: 'button', className: 'swatch cp-chip', title: colour, ariaLabel: colour });
        chip.style.background = colour;
        chip.addEventListener('click', () => setPicked(colour));
        chip.addEventListener('dblclick', () => setPicked(colour, true));
        return chip;
      }));
    const area = el('div', { className: 'cp-area', tabIndex: 0, ariaLabel: 'Saturation and brightness' });
    area.setAttribute('role', 'slider');
    const knob = el('span', { className: 'cp-knob' });
    area.append(knob);
    const hue = el('input', { type: 'range', min: 0, max: 359, step: 1, className: 'cp-hue', ariaLabel: t('Hue') });
    hue.style.background = `linear-gradient(to right, ${[0, 60, 120, 180, 240, 300, 360].map((at) => fromHsv(at % 360, 1, 1)).join(', ')})`;
    const field = el('input', { type: 'text', className: 'cp-hex', maxLength: 7, spellcheck: false, ariaLabel: t('Colour'), autocomplete: 'off' });
    const sample = el('span', { className: 'swatch cp-sample' });
    const ok = el('button', { type: 'button', className: 'primary', textContent: t('OK') });
    const cancel = el('button', { type: 'button', textContent: t('Cancel') });
    node.append(title, chips, area, hue, el('div', { className: 'cp-line' }, sample, field), el('div', { className: 'row end' }, ok, cancel));

    const drag = (event) => {
      const box = area.getBoundingClientRect();
      picking.saturation = Math.min(1, Math.max(0, (event.clientX - box.left) / box.width));
      picking.value = 1 - Math.min(1, Math.max(0, (event.clientY - box.top) / box.height));
      draw();
    };
    area.addEventListener('pointerdown', (event) => {
      area.setPointerCapture(event.pointerId);
      drag(event);
      area.focus();
    });
    area.addEventListener('pointermove', (event) => { if (area.hasPointerCapture(event.pointerId)) drag(event); });
    area.addEventListener('keydown', (event) => {
      const step = event.shiftKey ? 0.1 : 0.02;
      const moves = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] };
      if (!moves[event.key]) return;
      event.preventDefault();
      picking.saturation = Math.min(1, Math.max(0, picking.saturation + moves[event.key][0]));
      picking.value = Math.min(1, Math.max(0, picking.value + moves[event.key][1]));
      draw();
    });
    hue.addEventListener('input', () => { picking.hue = Number(hue.value); draw(); });
    field.addEventListener('input', () => {
      const typed = field.value.trim().startsWith('#') ? field.value.trim() : `#${field.value.trim()}`;
      if (/^#[0-9a-f]{6}$/i.test(typed)) setPicked(typed.toUpperCase(), false, true);
    });
    field.addEventListener('keydown', (event) => { if (event.key === 'Enter') { event.preventDefault(); ok.click(); } });
    ok.addEventListener('click', () => {
      const { index, from } = picking;
      const colour = picked();
      closePicker();
      if (colour !== from) attempt(() => edit({ op: 'colour', filament: index + 1, colour }));
    });
    cancel.addEventListener('click', closePicker);
    node.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closePicker(); }
    });
    document.addEventListener('pointerdown', (event) => {
      if (!node.hidden && !node.contains(event.target) && !(picking && picking.button.contains(event.target))) closePicker();
    });
    document.body.append(node);
    return node;
  }

  const picked = () => fromHsv(picking.hue, picking.saturation, picking.value);

  function setPicked(colour, accept = false, typed = false) {
    Object.assign(picking, toHsv(colour));
    draw(typed);
    if (accept) $('colour-picker').querySelector('.primary').click();
  }

  function draw(typed = false) {
    const node = $('colour-picker');
    const colour = picked();
    const area = node.querySelector('.cp-area');
    // The square's colours are of the hue picked: data, not the page's.
    area.style.background = `linear-gradient(to top, #000, transparent), linear-gradient(to right, #fff, ${fromHsv(picking.hue, 1, 1)})`;
    const knob = node.querySelector('.cp-knob');
    knob.style.left = `${picking.saturation * 100}%`;
    knob.style.top = `${(1 - picking.value) * 100}%`;
    area.setAttribute('aria-valuetext', colour);
    node.querySelector('.cp-hue').value = String(Math.round(picking.hue));
    node.querySelector('.cp-sample').style.background = colour;
    if (!typed) node.querySelector('.cp-hex').value = colour;
    for (const chip of node.querySelectorAll('.cp-chip')) chip.setAttribute('aria-pressed', String(chip.title === colour));
  }

  function openPicker(button, index, colour) {
    const node = picker();
    picking = { index, from: colour, button, ...toHsv(colour) };
    node.querySelector('#cp-title').textContent = fmt(t('Filament %d'), index + 1);
    node.hidden = false;
    draw();
    const at = button.getBoundingClientRect();
    const box = node.getBoundingClientRect();
    node.style.left = `${Math.max(4, Math.min(at.left, window.innerWidth - box.width - 4))}px`;
    const below = at.bottom + 4 + box.height <= window.innerHeight;
    node.style.top = `${below ? at.bottom + 4 : Math.max(4, at.top - box.height - 4)}px`;
    node.querySelector('.cp-hex').focus();
  }

  function closePicker() {
    const node = $('colour-picker');
    if (!node || node.hidden) return;
    node.hidden = true;
    if (picking && picking.button.isConnected) picking.button.focus();
    picking = null;
  }

  // ---- set up once ----

  function init() {
    const add = $('filament-add');
    const remove = $('filament-remove');
    add.append(svg(PLUS));
    remove.append(svg(MINUS));
    add.addEventListener('click', () => attempt(() => edit({ op: 'add' })));
    remove.addEventListener('click', () => attempt(() => edit({ op: 'remove_last' })));
    listbox($('apply-all-preset'), { bare: true });
    $('apply-all-button').addEventListener('click', () => attempt(() => edit({ op: 'apply_all', preset: $('apply-all-preset').value })));
    $('flush-open').addEventListener('click', () => attempt(openFlush));
    $('flush-calc').addEventListener('click', () => attempt(flushCalc));
    $('flush-ok').addEventListener('click', () => attempt(flushOk));
    $('flush-cancel').addEventListener('click', () => $('flush-dialog').close());
    const multiplier = $('flush-multiplier');
    multiplier.addEventListener('change', () => attempt(flushMultiplier));
    multiplier.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') { event.preventDefault(); attempt(flushMultiplier); }
    });
  }

  return { init, head, row, applyAll, closePicker };
})();
