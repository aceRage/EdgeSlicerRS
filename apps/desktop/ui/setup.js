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

  const fm = { rows: [], view: [], values: {}, search: '', sort: { key: null, dir: 1 }, filters: {}, pop: null, current: null };
  const STAR = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 1.4l1.95 4.1 4.45.55-3.27 3.08.84 4.42L8 11.4l-3.97 2.15.84-4.42L1.6 6.05l4.45-.55z"/></svg>';

  // The values as preset_manager.js shows them: whole degrees, a date of the system's, "—" when unknown.
  const fmTemp = (value) => (value === null || value === undefined ? '—' : String(Math.round(value)));
  function fmDate(seconds) {
    if (seconds === null || seconds === undefined) return '—';
    const date = new Date(seconds * 1000);
    try { return date.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }); } catch { /* below */ }
    return date.toISOString().replace('T', ' ').slice(0, 16);
  }
  function fmPrinters(row) {
    if (row.all) return 'Any printer';
    if (!row.printers.length) return '—';
    if (row.printers.length === 1) return row.printers[0];
    return `${row.printers[0]} +${row.printers.length - 1}`;
  }

  // The columns, in the reference's order (preset_manager.js, `KIND === 'filament'`): each with what its
  // cell shows; `sort` the session's name of the column to sort by, `filter` what its funnel offers.
  const FM_COLUMNS = [
    { key: 'checked', label: () => t('Enabled'), cls: 'check', sort: 'checked', filter: 'set' },
    { key: 'star', label: () => 'Favourite', cls: 'star', sort: 'star', filter: 'set' },
    { key: 'vendor', label: () => t('Vendor'), sort: 'vendor', filter: 'set', cell: (row) => [row.vendor || '—', row.vendor] },
    { key: 'name', label: () => t('Name'), sort: 'name', filter: 'text', cls: 'name' },
    { key: 'type', label: () => t('Type'), sort: 'type', filter: 'set', cell: (row) => [row.type || '—', row.type] },
    { key: 'source', label: () => 'Source', sort: 'source', filter: 'set', cell: (row) => [row.source === 'User' ? t('User') : 'System'] },
    { key: 'printers', label: () => 'Printers', sort: 'printers', filter: 'set', cell: (row) => [fmPrinters(row), row.printers.join('\n')] },
    { key: 'nozzle_t', label: () => 'Nozzle °C', cls: 'num', sort: 'nozzle_t', filter: 'range', tip: t('Nozzle temperature'),
      cell: (row) => [fmTemp(row.nozzle_t), row.nozzle_lo !== null && row.nozzle_hi !== null ? `Range ${fmTemp(row.nozzle_lo)}–${fmTemp(row.nozzle_hi)} °C` : ''] },
    { key: 'bed_t', label: () => 'Bed °C', cls: 'num', sort: 'bed_t', filter: 'range', tip: 'Bed temperature for the current plate type',
      cell: (row) => [fmTemp(row.bed_t)] },
    { key: 'mtime', label: () => 'Modified', sort: 'mtime',
      cell: (row) => [fmDate(row.mtime), row.mtime === null && row.version ? `Vendor bundle ${row.version}` : ''] },
    { key: 'actions', label: () => t('Actions'), cls: 'actions' },
  ];
  const TRASH = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 1h4l.5 1H14v1.5H2V2h3.5zM3 5h10l-.8 9.2a1 1 0 0 1-1 .8H4.8a1 1 0 0 1-1-.8z"/></svg>';

  // ---- rename and delete of one's own (preset_manager.js `startEdit`, `requestDelete`) ----

  // The rename in progress: { key, scope ('family' | 'one'), check (a timer) }.
  let fmEditing = null;

  function fmStatus(text) { $('fm-status').textContent = text || ''; }

  function fmEditor(row) {
    // The part before " @" of a filament variant; the printer part stays, after the field.
    const input = el('input', { type: 'text', className: 'fm-edit-input', value: row.suffix ? row.base : row.name, spellcheck: false });
    input.setAttribute('aria-label', `New name for ${row.name}`);
    const box = el('span', { className: 'fm-edit' }, input);
    if (row.suffix) box.append(el('span', { className: 'fm-edit-suffix', textContent: row.suffix }));
    if (row.suffix && row.family && row.family.length > 1) {
      const one = el('input', { type: 'checkbox', checked: fmEditing.scope === 'one' });
      one.addEventListener('change', () => { fmEditing.scope = one.checked ? 'one' : 'family'; fmCheck(row, input); });
      box.append(el('label', { className: 'fm-edit-scope', title: `Renames all ${row.family.length} printer variants of this filament` }, one, 'Only this printer'));
    }
    input.addEventListener('input', () => {
      clearTimeout(fmEditing.check);
      fmEditing.check = setTimeout(() => attempt(() => fmCheck(row, input)), 150);
    });
    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') { event.preventDefault(); attempt(() => fmCommit(row, input)); }
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); fmEditing = null; fmStatus(''); fmDraw(); }
    });
    setTimeout(() => { input.focus(); input.select(); }, 0);
    return box;
  }

  async function fmCheck(row, input) {
    if (!fmEditing) return;
    const answer = await call('filament_manager_check_name', fmArgs({ key: row.key, name: input.value, scope: fmEditing.scope }));
    input.classList.toggle('invalid', answer.level === 'invalid');
    input.title = answer.message || '';
    fmStatus(answer.message);
  }

  async function fmCommit(row, input) {
    const answer = await call('filament_manager_rename', fmArgs({ key: row.key, name: input.value, scope: fmEditing.scope }));
    if (!answer.ok) {
      input.classList.add('invalid');
      fmStatus(answer.message);
      return;
    }
    fmEditing = null;
    fmTake(answer);
    fmStatus(answer.message || 'Renamed');
    presetsByMachine.clear();
    state.project = await call('project', { project: activeId() });
    await showSidebar();
  }

  function fmRename(row) {
    if (!row.can_rename) { fmStatus('System presets cannot be renamed'); return; }
    fmEditing = { key: row.key, scope: 'family', check: null };
    fmDraw();
  }

  async function fmDelete(row) {
    if (!row.can_delete) { fmStatus('System presets cannot be deleted'); return; }
    const asked = await call('filament_manager_delete', fmArgs({ key: row.key }));
    if (asked.refused) { await inform(asked.title, asked.message); return; }
    if (!(await confirmBox(asked.title, asked.ask, t('Yes'), t('No')))) return;
    fmTake(await call('filament_manager_delete', fmArgs({ key: row.key, confirm: true })));
    fmStatus('Deleted');
    presetsByMachine.clear();
    state.project = await call('project', { project: activeId() });
    await showSidebar();
  }

  // The column sorted by (data_table.js `cycleSort`: up, down, then none; not kept between openings)
  // and the filters of the columns, applied by the session (`filament_manager`).
  function fmArgs(more = {}) {
    return { project: activeId(), search: fm.search, sort: fm.sort, filters: fm.filters, ...more };
  }
  function fmActive(column) {
    const filter = fm.filters[column.key];
    if (!filter) return false;
    if (column.filter === 'set') return (filter.exclude || []).length > 0;
    if (column.filter === 'text') return Boolean(filter);
    return filter.min !== null || filter.max !== null;
  }
  // What a list filter's value is called (preset_manager.js `setLabel`).
  function fmLabel(column, value) {
    if (column.key === 'checked') return value === 'yes' ? t('Enabled') : 'Not enabled';
    if (column.key === 'star') return value === 'yes' ? 'Favourite' : 'Not favourite';
    if (column.key === 'source') return value === 'User' ? t('User') : 'System';
    if (column.key === 'printers' && value && value === fm.current) return `${value} (current)`;
    return value === '' ? '—' : value;
  }

  function fmHead() {
    $('fm-head').replaceChildren(...FM_COLUMNS.map((column) => {
      const cell = el('th', { className: column.cls || '' });
      const sorting = fm.sort.key === column.sort ? fm.sort.dir : 0;
      cell.setAttribute('aria-sort', sorting > 0 ? 'ascending' : sorting < 0 ? 'descending' : 'none');
      const title = el('button', { type: 'button', className: 'th-sort', title: column.tip ? `${column.tip}. Click to sort` : 'Click to sort' },
        column.label(), el('span', { className: 'th-way', textContent: sorting > 0 ? ' ▲' : sorting < 0 ? ' ▼' : '' }));
      title.addEventListener('click', () => attempt(() => {
        const next = fm.sort.key !== column.sort ? 1 : fm.sort.dir === 1 ? -1 : 0;
        fm.sort = next ? { key: column.sort, dir: next } : { key: null, dir: 1 };
        return fmLoad();
      }));
      cell.append(title);
      if (column.filter) {
        const funnel = el('button', { type: 'button', className: `th-filter${fmActive(column) ? ' on' : ''}`, title: `Filter ${column.label()}` });
        funnel.setAttribute('aria-label', `Filter ${column.label()}`);
        funnel.innerHTML = '<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M1,1.5h10L7.2,6.2v4.3L4.8,9.3V6.2Z"/></svg>';
        funnel.addEventListener('click', (event) => { event.stopPropagation(); fmFilter(column, funnel); });
        cell.append(funnel);
      }
      return cell;
    }));
  }

  function fmClosePop() {
    $('fm-pop').hidden = true;
    $('fm-pop').replaceChildren();
    fm.pop = null;
  }

  // A column's funnel (data_table.js): the values to show, a text the name contains, or the least and most.
  function fmFilter(column, anchor) {
    if (fm.pop === column.key) { fmClosePop(); return; }
    const pop = $('fm-pop');
    const apply = (filter) => attempt(() => {
      if (filter === null) delete fm.filters[column.key]; else fm.filters[column.key] = filter;
      fmClosePop();
      return fmLoad();
    });
    const ok = el('button', { type: 'button', className: 'primary', textContent: t('OK') });
    const parts = [el('h3', { textContent: `Filter ${column.label()}` })];
    if (column.filter === 'set') {
      const values = fm.values[column.key] || [];
      const now = new Set((fm.filters[column.key] || {}).exclude || []);
      const list = el('div', { className: 'check-list' }, ...values.map((value) => {
        const box = el('input', { type: 'checkbox', checked: !now.has(value) });
        box.dataset.value = value;
        return el('label', {}, box, fmLabel(column, value));
      }));
      if (['vendor', 'type', 'printers'].includes(column.key)) {
        const find = el('input', { type: 'search', placeholder: 'Find…' });
        find.addEventListener('input', () => {
          const wanted = find.value.trim().toLowerCase();
          for (const label of list.children) label.hidden = !label.textContent.toLowerCase().includes(wanted);
        });
        parts.push(find);
      }
      const every = (on) => () => { for (const label of list.children) if (!label.hidden) label.querySelector('input').checked = on; };
      const all = el('button', { type: 'button', className: 'link', textContent: 'Select all' });
      const none = el('button', { type: 'button', className: 'link', textContent: 'Select none' });
      all.addEventListener('click', every(true));
      none.addEventListener('click', every(false));
      ok.addEventListener('click', () => {
        const exclude = [...list.querySelectorAll('input')].filter((box) => !box.checked).map((box) => box.dataset.value);
        apply(exclude.length ? { exclude } : null);
      });
      parts.push(el('div', { className: 'row' }, all, none), list);
    } else if (column.filter === 'text') {
      const input = el('input', { type: 'text', placeholder: 'Contains…', value: fm.filters[column.key] || '' });
      const take = () => apply(input.value.trim() === '' ? null : input.value.trim());
      input.addEventListener('keydown', (event) => { if (event.key === 'Enter') { event.preventDefault(); take(); } });
      ok.addEventListener('click', take);
      parts.push(input);
    } else {
      const now = fm.filters[column.key] || { min: null, max: null };
      const least = el('input', { type: 'number', step: '5', value: now.min === null ? '' : now.min });
      const most = el('input', { type: 'number', step: '5', value: now.max === null ? '' : now.max });
      const read = (input) => (input.value.trim() === '' || !Number.isFinite(Number(input.value)) ? null : Number(input.value));
      const take = () => {
        const [min, max] = [read(least), read(most)];
        apply(min === null && max === null ? null : { min, max });
      };
      for (const input of [least, most]) input.addEventListener('keydown', (event) => { if (event.key === 'Enter') { event.preventDefault(); take(); } });
      ok.addEventListener('click', take);
      parts.push(el('label', {}, 'At least', least), el('label', {}, 'At most', most),
        el('p', { className: 'quiet', textContent: 'Rows without this value are hidden while a limit is set.' }));
    }
    parts.push(el('div', { className: 'row end' }, ok));
    pop.replaceChildren(...parts);
    pop.hidden = false;
    fm.pop = column.key;
    const frame = $('fm-dialog').getBoundingClientRect();
    const from = anchor.getBoundingClientRect();
    pop.style.left = `${Math.max(8, Math.min(from.left - frame.left, frame.width - pop.offsetWidth - 8))}px`;
    pop.style.top = `${from.bottom - frame.top + 4}px`;
    const first = pop.querySelector('input');
    if (first) first.focus();
  }

  // The filters in force, each to take away, and "Clear search and filters" (data_table.js `renderChips`).
  function fmChips() {
    const chips = FM_COLUMNS.filter(fmActive).map((column) => {
      const filter = fm.filters[column.key];
      let said;
      if (column.filter === 'set') {
        const values = fm.values[column.key] || [];
        const shown = values.filter((value) => !filter.exclude.includes(value));
        said = shown.length <= 3 ? shown.map((value) => fmLabel(column, value)).join(', ') || 'none' : `${shown.length} of ${values.length}`;
        said = `${column.label()}: ${said}`;
      } else if (column.filter === 'text') said = `${column.label()}: “${filter}”`;
      else if (filter.min !== null && filter.max !== null) said = `${column.label()}: ${filter.min}–${filter.max}`;
      else if (filter.min !== null) said = `${column.label()} ≥ ${filter.min}`;
      else said = `${column.label()} ≤ ${filter.max}`;
      const remove = el('button', { type: 'button', className: 'icon', title: 'Remove this filter', textContent: '✕' });
      remove.addEventListener('click', () => attempt(() => { delete fm.filters[column.key]; return fmLoad(); }));
      return el('span', { className: 'chip' }, `${said} `, remove);
    });
    if (chips.length || fm.search.trim()) {
      const clear = el('button', { type: 'button', className: 'link', textContent: 'Clear search and filters' });
      clear.addEventListener('click', () => attempt(() => {
        fm.filters = {};
        fm.search = '';
        $('fm-search').value = '';
        return fmLoad();
      }));
      chips.push(clear);
    }
    $('fm-chips').replaceChildren(...chips);
    $('fm-chips').hidden = chips.length === 0;
  }

  // The toolbar's "Printer" list: one printer, or "All printers" ("{n} printers" when the funnel picked several).
  function fmPrinterList() {
    const select = $('fm-printer');
    const values = fm.values.printers || [];
    const excluded = new Set((fm.filters.printers || {}).exclude || []);
    const shown = values.filter((value) => !excluded.has(value));
    const options = [new Option('All printers', ''), ...values.map((value) => new Option(fmLabel(FM_COLUMNS[6], value), value))];
    if (shown.length > 1 && shown.length < values.length) options.push(new Option(`${shown.length} printers`, '\u0000several'));
    select.replaceChildren(...options);
    select.value = shown.length === values.length ? '' : shown.length === 1 ? shown[0] : '\u0000several';
  }

  async function fmLoad() {
    const answer = await call('filament_manager', fmArgs());
    fmTake(answer);
  }
  function fmTake(answer) {
    fm.rows = answer.rows;
    fm.view = answer.view;
    fm.values = answer.values;
    fm.current = answer.current_printer;
    fmHead();
    fmChips();
    fmPrinterList();
    fmDraw();
  }

  function fmRow(row) {
    const box = el('input', { type: 'checkbox', checked: row.enabled, disabled: row.lock === 'user' || (row.lock === 'in_use') });
    box.setAttribute('aria-label', `Enable ${row.name}`);
    // Why it cannot change (preset_manager.js `lockText`).
    const why = row.lock === 'user' ? 'User presets are always shown' : row.lock === 'in_use' ? 'In use in this project' : '';
    if (why) box.title = why;
    box.addEventListener('change', () => attempt(() => fmEnable(row, box.checked)));
    // The star (preset_manager.js `COL_STAR`): applied at once.
    const star = el('button', { type: 'button', className: `pt-star${row.star ? ' on' : ''}`, innerHTML: STAR });
    star.title = row.star ? 'Remove from favourites' : 'Add to favourites';
    star.setAttribute('aria-label', `${star.title}: ${row.name}`);
    star.setAttribute('aria-pressed', String(row.star));
    star.addEventListener('click', () => attempt(() => fmStar(row)));
    const cells = FM_COLUMNS.map((column) => {
      if (column.key === 'checked') return el('td', { className: 'check' }, box);
      if (column.key === 'star') return el('td', { className: 'star' }, star);
      if (column.key === 'actions') {
        const cell = el('td', { className: 'actions' });
        if (row.can_delete) {
          const remove = el('button', { type: 'button', className: 'icon fm-act', innerHTML: TRASH, title: `Delete ${row.name}` });
          remove.setAttribute('aria-label', remove.title);
          remove.addEventListener('click', () => attempt(() => fmDelete(row)));
          cell.append(remove);
        }
        return cell;
      }
      if (column.key === 'name' && fmEditing && fmEditing.key === row.key) return el('td', { className: 'name editing' }, fmEditor(row));
      if (column.key === 'name') {
        let title = row.name;
        if (row.members.length > 1 || row.members[0] !== row.name) title += `\n${row.members.join('\n')}`;
        if (row.in_use) title += '\nIn use';
        const cell = el('td', { className: `name${row.in_use ? ' inuse' : ''}`, textContent: row.name, title });
        // A double-click or F2 renames one's own (preset_manager.js `onDblClick`, `onRowKey`).
        cell.addEventListener('dblclick', () => fmRename(row));
        return cell;
      }
      const [text, title] = column.cell(row);
      return el('td', { className: `${column.cls || ''}${text === '—' ? ' na' : ''}`, textContent: text, title: title || '' });
    });
    const line = el('tr', { className: row.enabled ? 'on' : '', tabIndex: -1 }, ...cells);
    line.addEventListener('keydown', (event) => {
      if (event.target !== line) return;
      if (event.key === 'F2') { event.preventDefault(); fmRename(row); }
      if (event.key === 'Delete') { event.preventDefault(); attempt(() => fmDelete(row)); }
    });
    return line;
  }

  function fmDraw() {
    const byKey = new Map(fm.rows.map((row) => [row.key, row]));
    const shown = fm.view.map((key) => byKey.get(key)).filter(Boolean);
    $('fm-body').replaceChildren(...shown.map(fmRow));
    $('fm-none').hidden = shown.length > 0;
    const on = fm.rows.filter((row) => row.enabled).length;
    const filtered = fm.search.trim() !== '' || FM_COLUMNS.some(fmActive);
    // preset_manager.js `tx.count_all`, `tx.count`.
    $('fm-count').textContent = filtered ? `${shown.length} of ${fm.rows.length} shown · ${on} enabled` : `${fm.rows.length} filaments · ${on} enabled`;
  }

  // Every row of the same favourite key follows (preset_manager.js `star`).
  async function fmStar(row) {
    const answer = await call('favorite_toggle', { key: row.fav });
    const keys = new Set(answer.favs);
    favourites.keys = keys;
    await fmLoad();
    presetsByMachine.clear();
    await showSidebar();
  }

  async function fmEnable(row, on) {
    fmTake(await call('filament_enable', fmArgs({ changes: [{ key: row.key, on }] })));
    $('fm-status').textContent = 'Saved.';
    // The sidebar's lists follow at once, as the reference's do.
    presetsByMachine.clear();
    await showSidebar();
  }

  async function openFilamentManager() {
    fm.search = '';
    fm.sort = { key: null, dir: 1 };
    fm.filters = {};
    $('fm-search').value = '';
    $('fm-status').textContent = '';
    fmClosePop();
    await fmLoad();
    $('fm-dialog').showModal();
    $('fm-search').focus();
  }

  // ---- shared by the two "Create" dialogs ----

  // A row of a form: its name on the left, its controls on the right (the reference's OPTION_SIZE column).
  const formRow = (label, ...controls) => el('div', { className: 'cr-row' },
    el('span', { className: 'cr-name', textContent: label }), el('div', { className: 'cr-controls' }, ...controls));

  // A list to pick from with a prompt standing in until something is picked ("Select Vendor").
  function promptList(prompt, values, labels = values) {
    const select = el('select');
    const none = new Option(prompt, '');
    none.disabled = true;
    none.hidden = true;
    select.append(none, ...values.map((value, at) => new Option(labels[at], value)));
    select.value = '';
    return select;
  }

  function radios(name, choices, onChange) {
    const box = el('div', { className: 'cr-radios' });
    choices.forEach(([value, label], at) => {
      const input = el('input', { type: 'radio', name, value, checked: at === 0 });
      input.addEventListener('change', () => onChange(value));
      box.append(el('label', {}, input, label));
    });
    return box;
  }

  // The reference's "<app> - Info" message boxes.
  const info = (text) => inform(t('Info'), text);

  // ---- "Create Printer/Nozzle" ----

  const cp = { choices: null, page: 1, nozzleOnly: false, custom: false, bedModel: '', bedTexture: '', presets: null, template: true };

  function cpField(value, unit = 'mm') {
    const input = el('input', { type: 'text', inputMode: 'numeric', value, className: 'cr-number' });
    // As the reference's fields: digits only (wxFILTER_DIGITS).
    input.addEventListener('input', () => { input.value = input.value.replace(/[^0-9]/g, ''); });
    return el('span', { className: 'cr-unit' }, input, el('small', { textContent: unit }));
  }

  function cpForm() {
    const page = cp.page1;
    return {
      type: cp.nozzleOnly ? 'nozzle' : 'printer',
      custom: cp.custom,
      vendor: cp.custom ? page.customVendor.value : page.vendor.value,
      model: cp.custom ? page.customModel.value : page.model.value,
      printer: page.existing.value,
      nozzle: page.nozzle.value,
      bed: [page.x.querySelector('input').value, page.y.querySelector('input').value],
      origin: [page.ox.querySelector('input').value, page.oy.querySelector('input').value],
      height: page.height.querySelector('input').value,
      bed_model: cp.bedModel,
      bed_texture: cp.bedTexture,
    };
  }

  // What the first page shows for the type picked (`select_curr_radiobox`).
  function cpShowType() {
    const page = cp.page1;
    page.printerLine.hidden = cp.nozzleOnly;
    page.existingLine.hidden = !cp.nozzleOnly;
    page.info.hidden = cp.nozzleOnly;
    page.vendor.closest('.lb').hidden = cp.custom;
    page.model.closest('.lb').hidden = cp.custom;
    page.customVendor.hidden = !cp.custom;
    page.customModel.hidden = !cp.custom;
  }

  async function cpBedFile(kind, label) {
    const path = await pick(kind === 'model' ? 'bed_model' : 'bed_texture');
    if (!path) return;
    if (kind === 'model') cp.bedModel = ''; else cp.bedTexture = '';
    label.textContent = t('Empty');
    try {
      const answer = await call('create_printer_bed_file', { kind, path });
      if (kind === 'model') cp.bedModel = answer.path; else cp.bedTexture = answer.path;
      label.textContent = answer.name;
      label.title = answer.path;
    } catch (error) {
      // "Invalid file format." is said in a box; the size in the line, as the reference has them.
      if (error.message === t('Invalid file format.')) await inform(t('Error'), error.message);
      else label.textContent = error.message;
    }
  }

  function cpPage1() {
    const choices = cp.choices;
    const vendor = promptList(t('Select Vendor'), choices.vendors.map((entry) => entry.vendor));
    const model = promptList(t('Select Model'), []);
    vendor.addEventListener('change', () => {
      const entry = choices.vendors.find((known) => known.vendor === vendor.value);
      model.replaceChildren(...entry.models.map((name) => new Option(name, name)));
      model.value = entry.models[0] || '';
    });
    const customVendor = el('input', { type: 'text', placeholder: t('Input Custom Vendor'), hidden: true });
    const customModel = el('input', { type: 'text', placeholder: t('Input Custom Model'), hidden: true });
    const cant = el('input', { type: 'checkbox' });
    cant.addEventListener('change', () => { cp.custom = cant.checked; cpShowType(); });
    const existing = promptList(t('Select Printer'), choices.existing);
    const nozzle = el('select');
    nozzle.append(...choices.nozzles.map((value) => new Option(value, value)));
    const loadModel = el('button', { type: 'button', textContent: t('Load...') });
    const modelName = el('span', { className: 'quiet', textContent: t('Empty') });
    loadModel.addEventListener('click', () => attempt(() => cpBedFile('model', modelName)));
    const loadTexture = el('button', { type: 'button', textContent: t('Load...') });
    const textureName = el('span', { className: 'quiet', textContent: t('Empty') });
    loadTexture.addEventListener('click', () => attempt(() => cpBedFile('texture', textureName)));
    const page = {
      vendor, model, customVendor, customModel, existing, nozzle,
      x: cpField('200'), y: cpField('200'), ox: cpField('0'), oy: cpField('0'), height: cpField('200'),
    };
    const lists = el('div', { className: 'cr-pair' }, listbox(vendor), listbox(model), customVendor, customModel);
    page.printerLine = formRow(t('Printer'), lists, el('label', { className: 'cr-check' }, cant, t("Can't find my printer model")));
    page.existingLine = formRow(t('Printer'), listbox(existing));
    page.info = el('div', {},
      formRow(t('Bed Shape'), el('span', { textContent: t('Rectangle') })),
      formRow(t('Printable Space'), el('span', { className: 'cr-axis', textContent: 'X' }), page.x, el('span', { className: 'cr-axis', textContent: 'Y' }), page.y),
      formRow(t('Origin'), el('span', { className: 'cr-axis', textContent: 'X' }), page.ox, el('span', { className: 'cr-axis', textContent: 'Y' }), page.oy),
      formRow(t('Hot Bed STL'), loadModel, modelName),
      formRow(t('Hot Bed SVG'), loadTexture, textureName),
      formRow(t('Max Print Height'), page.height));
    cp.page1 = page;
    const ok = el('button', { type: 'button', className: 'primary', textContent: t('OK') });
    ok.addEventListener('click', () => attempt(cpNext));
    const cancel = el('button', { type: 'button', textContent: t('Cancel') });
    cancel.addEventListener('click', () => $('cp-dialog').close());
    return el('div', { className: 'cr-page' },
      formRow(t('Create Type'), radios('cp-type', [['printer', t('Create Printer')], ['nozzle', t('Create Nozzle for Existing Printer')]],
        (value) => { cp.nozzleOnly = value === 'nozzle'; cpShowType(); })),
      page.printerLine, page.existingLine,
      formRow(t('Nozzle Diameter'), listbox(nozzle)),
      page.info,
      el('div', { className: 'dialog-foot' }, el('div', { className: 'grow' }), ok, cancel));
  }

  // The second page's lists of presets to make, with "Select All" / "Deselect All".
  function cpTicks(title, names) {
    const boxes = names.map((name) => el('label', { className: 'fs-item', title: name }, el('input', { type: 'checkbox', checked: false, value: name }), name));
    const grid = el('div', { className: 'cr-grid' }, ...boxes);
    const every = (on) => () => { for (const label of boxes) label.querySelector('input').checked = on; };
    const all = el('button', { type: 'button', className: 'link', textContent: t('Select All') });
    all.addEventListener('click', every(true));
    const none = el('button', { type: 'button', className: 'link', textContent: t('Deselect All') });
    none.addEventListener('click', every(false));
    return el('div', { className: 'cr-ticks' }, el('h3', { textContent: title }), grid, el('div', { className: 'row' }, all, none));
  }

  async function cpPresets() {
    const page = cp.page2;
    page.lists.replaceChildren();
    cp.presets = null;
    if (!page.vendor.value || !page.model.value) return;
    const offered = await call('create_printer_presets', { vendor: page.vendor.value, model: page.model.value, template: cp.template });
    cp.presets = offered;
    page.lists.append(cpTicks(t('Filament Preset Template'), offered.filaments), cpTicks(t('Process Preset Template'), offered.processes));
  }

  function cpPage2() {
    const vendor = promptList(t('Select Vendor'), cp.choices.template_vendors);
    const model = promptList(t('Select Model'), []);
    const lists = el('div', { className: 'cr-lists' });
    cp.page2 = { vendor, model, lists };
    vendor.addEventListener('change', () => attempt(async () => {
      const answer = await call('create_printer_models', { vendor: vendor.value, nozzle: cp.page1.nozzle.value });
      model.replaceChildren(...answer.models.map((name) => new Option(name, name)));
      model.value = answer.models[0];
      await cpPresets();
    }));
    model.addEventListener('change', () => attempt(cpPresets));
    const back = el('button', { type: 'button', textContent: t('Return') });
    back.addEventListener('click', () => cpShow(1));
    const create = el('button', { type: 'button', className: 'primary', textContent: t('Create') });
    create.addEventListener('click', () => attempt(() => cpCreate({})));
    const cancel = el('button', { type: 'button', textContent: t('Cancel') });
    cancel.addEventListener('click', () => $('cp-dialog').close());
    return el('div', { className: 'cr-page' },
      formRow(t('Printer Preset'), el('div', { className: 'cr-stack' },
        el('span', { className: 'quiet', textContent: t('Create Based on Current Printer') }),
        el('div', { className: 'cr-pair' }, listbox(vendor), listbox(model)))),
      formRow(t('Presets'), radios('cp-presets', [['template', t('Create from Template')], ['current', t('Create Based on Current Printer')]],
        (value) => { cp.template = value === 'template'; attempt(cpPresets); })),
      el('div', { className: 'setup-scroll cr-scroll' }, lists),
      el('div', { className: 'dialog-foot' }, el('div', { className: 'grow' }), back, create, cancel));
  }

  function cpShow(page) {
    cp.page = page;
    $('cp-dialog').querySelector('.cr-p1').hidden = page !== 1;
    $('cp-dialog').querySelector('.cr-p2').hidden = page !== 2;
    for (const [at, step] of $('cp-dialog').querySelectorAll('.cr-step').entries()) step.classList.toggle('on', at + 1 === page);
  }

  async function cpNext() {
    try {
      await call('create_printer_check', cpForm());
    } catch (error) {
      await info(error.message);
      return;
    }
    cpShow(2);
  }

  async function cpCreate(more) {
    const page = cp.page2;
    const ticked = (title) => [...page.lists.querySelectorAll('.cr-ticks')].filter((box) => box.querySelector('h3').textContent === title)
      .flatMap((box) => [...box.querySelectorAll('input:checked')].map((input) => input.value));
    const args = {
      ...cpForm(),
      project: activeId(),
      preset_vendor: page.vendor.value,
      preset_model: page.model.value,
      template: cp.template,
      filaments: ticked(t('Filament Preset Template')),
      processes: ticked(t('Process Preset Template')),
      ...more,
    };
    let answer;
    try {
      answer = await call('create_printer', args);
    } catch (error) {
      await info(error.message);
      // An illegal printable area goes back to the first page.
      if (error.message === t('You have entered an illegal input in the printable area section on the first page. Please check before creating it.')
        || error.message === t('The custom printer or model is not entered, please enter it.')) cpShow(1);
      return;
    }
    if (!answer.created) {
      // Written over only when the person says so.
      if (!(await confirmBox(answer.title, answer.ask, t('Yes'), t('Cancel')))) return;
      await cpCreate({ ...more, ...(answer.step === 'printer' ? { rewrite: true } : { rewrite_presets: true }) });
      return;
    }
    $('cp-dialog').close();
    state.project = answer;
    presetsByMachine.clear();
    await showSidebar();
    await showStage();
    await refreshStrip();
    // "Printer Created Successfully": "Printer Setting" opens the printer's settings.
    if (await confirmBox(t('Printer Created Successfully'), `${t('Printer Created')}\n${t('Please go to printer settings to edit your presets')}`, t('Printer Setting'), t('Cancel'))) {
      $('printer-edit').click();
    }
  }

  async function openCreatePrinter() {
    cp.choices = await call('create_printer_choices', {});
    Object.assign(cp, { nozzleOnly: false, custom: false, bedModel: '', bedTexture: '', template: true });
    const dialog = $('cp-dialog');
    const steps = el('div', { className: 'cr-steps' },
      el('span', { className: 'cr-step on' }, el('b', { textContent: '1' }), t('Create Printer')),
      el('span', { className: 'cr-step' }, el('b', { textContent: '2' }), t('Import Preset')));
    dialog.replaceChildren(el('h2', { id: 'cp-title', textContent: t('Create Printer/Nozzle') }), steps,
      el('div', { className: 'cr-p1' }, cpPage1()), el('div', { className: 'cr-p2', hidden: true }, cpPage2()));
    cpShowType();
    cpShow(1);
    dialog.showModal();
  }

  // ---- "Create Filament" ----

  const cf = { choices: null, offered: null, base: true };

  // A row of the table: printer, nozzle and preset ("Copy Current Filament Preset ").
  function cfRow(machine = null, filament = null) {
    const table = cf.offered.table;
    const printer = el('select');
    printer.append(...table.map((row, at) => new Option(row.printer, String(at))));
    const nozzle = el('select');
    const preset = el('select');
    const fillPresets = (want) => {
      const entry = table[Number(printer.value)];
      const at = entry ? entry.nozzles[Number(nozzle.value)] : null;
      const names = at ? at.presets : [];
      preset.replaceChildren(...names.map((name) => new Option(name, name)));
      // "No compatible preset" stands where none is.
      if (!names.length) preset.append(Object.assign(new Option(t('No compatible preset'), ''), { disabled: true, selected: true }));
      else preset.value = names.includes(want) ? want : names[0];
    };
    const fillNozzles = (want, wantPreset) => {
      const entry = table[Number(printer.value)];
      const nozzles = entry ? entry.nozzles : [];
      nozzle.replaceChildren(...nozzles.map((at, index) => new Option(at.nozzle, String(index))));
      const found = nozzles.findIndex((at) => at.machine === want);
      nozzle.value = String(Math.max(0, found));
      fillPresets(wantPreset);
    };
    const start = table.findIndex((row) => row.nozzles.some((at) => at.machine === machine));
    printer.value = String(Math.max(0, start));
    fillNozzles(machine, filament);
    printer.addEventListener('change', () => fillNozzles(null, null));
    nozzle.addEventListener('change', () => fillPresets(null));
    const remove = el('button', { type: 'button', className: 'danger', textContent: t('Remove') });
    const row = el('div', { className: 'cr-table-row' }, listbox(printer), listbox(nozzle), listbox(preset), remove);
    row.pick = () => {
      const entry = table[Number(printer.value)];
      const at = entry ? entry.nozzles[Number(nozzle.value)] : null;
      return at && preset.value ? { printer: at.machine, preset: preset.value } : null;
    };
    remove.addEventListener('click', () => row.remove());
    return row;
  }

  async function cfOffered() {
    const page = cf.page;
    const type = page.type.value;
    page.base.replaceChildren(new Option(t('Select Filament Preset'), ''));
    page.base.options[0].disabled = true;
    page.base.value = '';
    page.printers.replaceChildren();
    page.rows.replaceChildren();
    cf.offered = null;
    if (!type) return;
    cf.offered = await call('create_filament_presets', { project: activeId(), type });
    page.base.append(...cf.offered.base.map((choice, at) => new Option(choice.name, String(at))));
    // The first row starts on what the project prints with.
    page.rows.append(cfRow(cf.offered.current.machine, cf.offered.current.filament));
  }

  function cfShowMode() {
    const page = cf.page;
    page.base.closest('.lb').hidden = !cf.base;
    page.printers.hidden = !cf.base;
    page.tableBox.hidden = cf.base;
    page.note.textContent = cf.base
      ? t('We could create the filament presets for your following printer:')
      : t('We would rename the presets as "Vendor Type Serial @printer you selected".\nTo add preset for more printers, please go to printer selection');
  }

  async function cfCreate(more) {
    const page = cf.page;
    const presets = cf.base
      ? [...page.printers.querySelectorAll('input:checked')].map((input) => JSON.parse(input.value))
      : [...page.rows.children].map((row) => row.pick()).filter(Boolean);
    const args = {
      custom: page.cant.checked,
      vendor: page.vendor.value,
      custom_vendor: page.customVendor.value,
      type: page.type.value,
      serial: page.serial.value,
      presets,
      ...more,
    };
    let answer;
    try {
      answer = await call('create_filament', args);
    } catch (error) {
      await info(error.message);
      return;
    }
    if (!answer.created) {
      if (!(await confirmBox(answer.title, answer.ask, t('Yes'), t('No')))) return;
      await cfCreate({ ...more, ...(answer.step === 'name' ? { continue_name: true } : { rewrite: true }) });
      return;
    }
    $('cf-dialog').close();
    presetsByMachine.clear();
    await showSidebar();
    if ($('fm-dialog').open) {
      await fmLoad();
    }
    await inform(t('Filament Created Successfully'), `${t('Filament Created')}\n${t('Please go to filament setting to edit your presets if you need.\nPlease note that nozzle temperature, hot bed temperature, and maximum volumetric speed has a significant impact on printing quality. Please set them carefully.')}`);
  }

  async function openCreateFilament() {
    cf.choices = await call('create_filament_choices', {});
    cf.base = true;
    const vendor = promptList(t('Select Vendor'), cf.choices.vendors);
    const customVendor = el('input', { type: 'text', placeholder: t('Input Custom Vendor'), maxLength: 50, hidden: true });
    const cant = el('input', { type: 'checkbox' });
    const type = promptList(t('Select Type'), cf.choices.types);
    const serial = el('input', { type: 'text', maxLength: 50 });
    const base = el('select');
    const printers = el('div', { className: 'cr-grid' });
    const rows = el('div', { className: 'cr-table' });
    const addRow = el('button', { type: 'button', textContent: t('+ Add row') });
    addRow.addEventListener('click', () => { if (cf.offered) rows.append(cfRow()); });
    const tableBox = el('div', { hidden: true },
      el('div', { className: 'cr-table-row cr-table-head' }, el('span', { textContent: t('Printer') }), el('span', { textContent: t('Nozzle') }),
        el('span', { textContent: t('Filament Preset') }), el('span')),
      rows, el('div', { className: 'row' }, addRow));
    const note = el('p', { className: 'quiet cr-note' });
    cf.page = { vendor, customVendor, cant, type, serial, base, printers, rows, tableBox, note };
    cant.addEventListener('change', () => {
      vendor.closest('.lb').hidden = cant.checked;
      customVendor.hidden = !cant.checked;
    });
    type.addEventListener('change', () => attempt(cfOffered));
    base.addEventListener('change', () => {
      const choice = cf.offered && cf.offered.base[Number(base.value)];
      printers.replaceChildren(...(choice ? choice.printers : []).map((pair) => el('label', { className: 'fs-item', title: pair.preset },
        el('input', { type: 'checkbox', value: JSON.stringify(pair) }), pair.printer)));
    });
    const create = el('button', { type: 'button', className: 'primary', textContent: t('Create') });
    create.addEventListener('click', () => attempt(() => cfCreate({})));
    const cancel = el('button', { type: 'button', textContent: t('Cancel') });
    cancel.addEventListener('click', () => $('cf-dialog').close());
    const dialog = $('cf-dialog');
    dialog.replaceChildren(
      el('h2', { id: 'cf-title', textContent: t('Create Filament') }),
      el('div', { className: 'cr-page' },
        el('h3', { className: 'cr-section', textContent: t('Basic Information') }),
        formRow(t('Vendor'), el('div', { className: 'cr-stack' }, el('div', { className: 'cr-pair' }, listbox(vendor), customVendor),
          el('label', { className: 'cr-check' }, cant, t("Can't find vendor I want")))),
        formRow(t('Type'), el('div', { className: 'cr-pair' }, listbox(type))),
        formRow(t('Serial'), el('div', { className: 'cr-stack' }, serial, el('small', { className: 'quiet', textContent: t('e.g. Basic, Matte, Silk, Marble') }))),
        el('h3', { className: 'cr-section', textContent: t('Add Filament Preset under this filament') }),
        formRow(t('Filament Preset'), el('div', { className: 'cr-stack' },
          radios('cf-mode', [['base', t('Create Based on Current Filament')], ['copy', t('Copy Current Filament Preset ')]],
            (value) => { cf.base = value === 'base'; cfShowMode(); }),
          el('div', { className: 'cr-pair' }, listbox(base)))),
        note,
        el('div', { className: 'setup-scroll cr-scroll' }, el('div', { className: 'cr-lists' }, printers, tableBox))),
      el('div', { className: 'dialog-foot' }, el('div', { className: 'grow' }), create, cancel));
    base.append(Object.assign(new Option(t('Select Filament Preset'), ''), { disabled: true, selected: true }));
    cfShowMode();
    dialog.showModal();
  }

  function init() {
    // The guide's "Back" (its text.js, t8; "Zurück" in German) is not the
    // catalogs' "Back", which is a model's back side ("Rückseite"):
    // outside English the catalogs' "Return" ("Zurück") says it.
    $('fs-back').textContent = I18n.language.startsWith('en') ? 'Back' : t('Return');
    $('fm-create').addEventListener('click', () => attempt(openCreateFilament));
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
    let searching = null;
    $('fm-search').addEventListener('input', () => {
      fm.search = $('fm-search').value;
      clearTimeout(searching);
      searching = setTimeout(() => attempt(fmLoad), 150);
    });
    $('fm-printer').addEventListener('change', () => attempt(() => {
      const value = $('fm-printer').value;
      if (value === '\u0000several') return undefined;
      if (value === '') delete fm.filters.printers;
      else fm.filters.printers = { exclude: (fm.values.printers || []).filter((other) => other !== value) };
      return fmLoad();
    }));
    // Escape: a filter first, then the dialog (preset_manager.js `escape`).
    $('fm-dialog').addEventListener('cancel', (event) => { if (fm.pop) { event.preventDefault(); fmClosePop(); } });
    $('fm-dialog').addEventListener('pointerdown', (event) => {
      if (fm.pop && !$('fm-pop').contains(event.target) && !event.target.closest('.th-filter')) fmClosePop();
    });
    $('fm-close').addEventListener('click', () => $('fm-dialog').close());
  }

  return { init, openFilamentSelection, openFilamentManager, openCreatePrinter, openCreateFilament };
})();
