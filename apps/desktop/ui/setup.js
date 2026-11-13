'use strict';

// Setting up printers and filaments beyond Printer Selection: the guide's
// "Filament Selection" page (resources/web/guide/22/22.js), reached by
// "Next" from Printer Selection in the setup wizard (Help > "Setup
// Wizard", and the first run), and the Filament manager that
// "Add/Remove filaments" at the foot of each filament list opens
// (PresetManagerDialog.cpp, resources/web/guide/preset_manager). The
// session does the work (`filament_selection`, `filament_selection_save`,
// `filament_manager`, `filament_enable`). Loaded before app.js, whose
// `call`, `$`, `el`, `state` and the rest are used once the page runs.
//
// The guide's own words (its text.js: "Filament Selection", "error",
// "yes", "no") are written as they are: they are not message ids of the
// reference's catalog, and read in English here.

const Setup = (() => {
  // ---- Filament Selection ----

  const fs = {
    // The models ticked on the printer page, not yet kept: [{ vendor, model }].
    models: [],
    rows: [],
    // Ticked by their key, kept while the filters change what is shown.
    ticked: new Set(),
  };

  // A line of boxes to tick by ("Printer", "Filament type", "Vendor"): "All" first, as 22.js has it.
  function filterLine(line, values, labelOf = (value) => value) {
    const box = $(line).querySelector('.fs-values');
    const all = el('input', { type: 'checkbox', checked: true });
    const items = values.map((value) => {
      const input = el('input', { type: 'checkbox', checked: true });
      input.dataset.value = value;
      input.addEventListener('change', () => {
        all.checked = items.every((label) => label.querySelector('input').checked);
        fsSort();
      });
      return el('label', {}, input, labelOf(value));
    });
    all.addEventListener('change', () => {
      for (const label of items) label.querySelector('input').checked = all.checked;
      fsSort();
    });
    box.replaceChildren(el('label', {}, all, 'all'), ...items);
  }

  const tickedIn = (line) => new Set([...$(line).querySelectorAll('.fs-values input[data-value]')]
    .filter((input) => input.checked).map((input) => input.dataset.value));

  // What is shown: a filament of a type and a vendor ticked, cut for a printer ticked (22.js `SortFilament`).
  function fsSort() {
    const printers = new Set();
    for (const printer of fs.printers) {
      if (!tickedIn('fs-printers').has(printer.model)) continue;
      for (const nozzle of printer.nozzles) printers.add(`[${printer.model}++${nozzle}]`);
    }
    // With one printer its line is not shown, and it is always ticked.
    if (fs.printers.length <= 1) for (const printer of fs.printers) for (const nozzle of printer.nozzles) printers.add(`[${printer.model}++${nozzle}]`);
    const types = tickedIn('fs-types');
    const vendors = tickedIn('fs-vendors');
    let shown = 0;
    for (const item of $('fs-items').children) {
      const row = fs.rows[Number(item.dataset.index)];
      const show = types.has(row.type) && vendors.has(row.vendor)
        && (row.models.length === 0 || row.models.some((model) => printers.has(model)));
      item.hidden = !show;
      if (show) shown += 1;
    }
    fsCount(shown);
  }

  function fsCount(shown = null) {
    const visible = shown ?? [...$('fs-items').children].filter((item) => !item.hidden).length;
    $('fs-count').textContent = `${fs.ticked.size} of ${fs.rows.length} filaments ticked · ${visible} shown`;
  }

  function fsDraw() {
    $('fs-items').replaceChildren(...fs.rows.map((row, index) => {
      const input = el('input', { type: 'checkbox', checked: fs.ticked.has(row.key) });
      input.addEventListener('change', () => {
        if (input.checked) fs.ticked.add(row.key); else fs.ticked.delete(row.key);
        fsCount();
      });
      const item = el('label', { className: 'fs-item', title: `${row.vendor} · ${row.type}\n${row.members.join('\n')}` }, input, row.name);
      item.dataset.index = index;
      return item;
    }));
  }

  // The models' usual filaments, and nothing else (22.js `ChooseDefaultFilament`).
  function fsDefaults() {
    fs.ticked = new Set(fs.rows.filter((row) => row.default).map((row) => row.key));
    fsDraw();
    fsSort();
    $('fs-notice').hidden = true;
  }

  async function openFilamentSelection(models) {
    fs.models = models;
    const page = await call('filament_selection', { models });
    fs.rows = page.filaments;
    fs.printers = page.printers;
    fs.ticked = new Set(fs.rows.filter((row) => row.selected).map((row) => row.key));
    filterLine('fs-printers', page.printers.map((printer) => printer.model));
    $('fs-printers').hidden = page.printers.length <= 1;
    filterLine('fs-types', page.types);
    filterLine('fs-vendors', page.vendors);
    $('fs-notice').hidden = true;
    fsDraw();
    // Nothing enabled yet: the printers' usual filaments are ticked.
    if (fs.ticked.size === 0) fsDefaults(); else fsSort();
    $('fs-dialog').showModal();
    $('fs-finish').focus();
  }

  // "All" and "Clear all" tick every filament, or none (22.js `SelectAllFilament`).
  function fsEvery(on) {
    fs.ticked = on ? new Set(fs.rows.map((row) => row.key)) : new Set();
    for (const input of $('fs-items').querySelectorAll('input')) input.checked = on;
    fsCount();
  }

  async function fsFinish() {
    if (fs.ticked.size === 0) {
      $('fs-notice').hidden = false;
      $('fs-yes').focus();
      return;
    }
    // Every preset of each filament ticked, shown or not (22.js `ResponseFilamentResult`).
    const names = fs.rows.filter((row) => fs.ticked.has(row.key)).flatMap((row) => row.members);
    const answer = await call('printers_save', { project: activeId(), models: fs.models });
    await call('filament_selection_save', { names });
    $('fs-dialog').close();
    presetsByMachine.clear();
    await printersChanged(answer);
  }

  // "Back": the printer page again, its ticks as they were.
  function fsBack() {
    $('fs-dialog').close();
    $('printers-dialog').showModal();
  }

  // ---- the Filament manager ----

  const fm = { rows: [], search: '', pending: false };

  function fmMatches(row) {
    const words = fm.search.toLowerCase().match(/\S+/g) || [];
    const hay = `${row.name}\u0000${row.vendor}\u0000${row.type}\u0000${row.printers.join('\u0000')}`.toLowerCase();
    return words.every((word) => hay.includes(word));
  }

  function fmDraw() {
    const shown = fm.rows.filter(fmMatches);
    $('fm-body').replaceChildren(...shown.map((row) => {
      const box = el('input', { type: 'checkbox', checked: row.enabled, disabled: row.lock === 'user' || (row.lock === 'in_use') });
      box.setAttribute('aria-label', row.name);
      // Why it cannot change (preset_manager.js `pm_lock_user`, `pm_lock_in_use`).
      const why = row.lock === 'user' ? 'User presets are always shown' : row.lock === 'in_use' ? 'In use in this project' : '';
      if (why) box.title = why;
      box.addEventListener('change', () => attempt(() => fmEnable(row, box.checked)));
      const printers = row.printers.length > 2 ? `${row.printers[0]} +${row.printers.length - 1}` : row.printers.join(', ');
      return el('tr', { className: row.enabled ? 'on' : '' },
        el('td', { className: 'check' }, box),
        el('td', { textContent: row.name, title: row.members.join('\n') }),
        el('td', { textContent: row.vendor || '—' }),
        el('td', { textContent: row.type || '—' }),
        el('td', { textContent: printers || '—', title: row.printers.join('\n') }),
        el('td', { textContent: row.source === 'User' ? t('User') : 'System' }));
    }));
    const on = fm.rows.filter((row) => row.enabled).length;
    $('fm-count').textContent = `${shown.length} of ${fm.rows.length} shown · ${on} enabled`;
  }

  async function fmEnable(row, on) {
    const answer = await call('filament_enable', { project: activeId(), changes: [{ key: row.key, on }] });
    fm.rows = answer.rows;
    fmDraw();
    $('fm-status').textContent = 'Saved.';
    // The sidebar's lists follow at once, as the reference's do.
    presetsByMachine.clear();
    await showSidebar();
  }

  async function openFilamentManager() {
    const answer = await call('filament_manager', { project: activeId() });
    fm.rows = answer.rows;
    fm.search = '';
    $('fm-search').value = '';
    $('fm-status').textContent = '';
    fmDraw();
    $('fm-dialog').showModal();
    $('fm-search').focus();
  }

  function init() {
    $('fs-all').addEventListener('click', () => fsEvery(true));
    $('fs-clear').addEventListener('click', () => fsEvery(false));
    $('fs-finish').addEventListener('click', () => attempt(fsFinish));
    $('fs-back').addEventListener('click', fsBack);
    $('fs-yes').addEventListener('click', fsDefaults);
    $('fs-no').addEventListener('click', () => { $('fs-notice').hidden = true; });
    // Escape: the notice first, then the whole guide, as its close box.
    $('fs-dialog').addEventListener('cancel', (event) => {
      event.preventDefault();
      if (!$('fs-notice').hidden) { $('fs-notice').hidden = true; return; }
      $('fs-dialog').close();
      attempt(ptCancel);
    });
    $('fm-search').addEventListener('input', () => { fm.search = $('fm-search').value; fmDraw(); });
    $('fm-close').addEventListener('click', () => $('fm-dialog').close());
  }

  return { init, openFilamentSelection, openFilamentManager };
})();
