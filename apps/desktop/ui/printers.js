'use strict';

// The sidebar's printer list and "Select/Remove printers", as the
// reference has them (PresetComboBoxes.cpp `PlaterPresetComboBox::update`,
// Plater.cpp `update_nozzle_settings`, resources/web/guide/24 with
// js/printer_table.js and js/data_table.js). Loaded before app.js, whose
// `call`, `$`, `el`, `state` and the rest are used once the page runs.

// ---- pictures ----

// A printer model's picture, asked for once.
const covers = new Map();
function coverOf(vendor, model) {
  const key = `${vendor}\n${model}`;
  if (!covers.has(key)) {
    covers.set(key, call('printer_cover', { vendor, model }).then((answer) => answer.data, () => null));
  }
  return covers.get(key);
}

// Puts a model's picture in `box`, or leaves the grey stand-in.
async function showCover(box, vendor, model) {
  const key = `${vendor}\n${model}`;
  if (box.dataset.cover === key) return;
  box.dataset.cover = key;
  box.replaceChildren();
  const data = vendor && model ? await coverOf(vendor, model) : null;
  if (box.dataset.cover !== key || !data) return;
  const image = el('img', { alt: '', draggable: false, decoding: 'async' });
  image.addEventListener('error', () => image.remove());
  image.src = data;
  box.replaceChildren(image);
}

// ---- the sidebar's printer list ----

// The list: the project's own printer when it carries one
// ("Project-inside presets"), one row for each printer model added
// ("System presets"), and the entry that opens the printer selection;
// the person's own printer presets between ("User presets"), each by its
// name, "* " before the one in force with something set over it.
function showPrinterBlock() {
  const project = state.project;
  const listing = state.printers;
  const selected = listing.selected;
  const select = $('printer');
  const group = (label, options) => {
    const node = el('optgroup', { label });
    node.append(...options);
    return node;
  };
  const groups = [];
  let value = '';
  // What a row's picture is of, for the list (listbox) to show.
  const pictured = (option, model) => {
    if (model && model.vendor && model.model) Object.assign(option.dataset, { vendor: model.vendor, model: model.model });
    return option;
  };
  if (project.has_settings) {
    const own = selected && selected.source === 'project';
    groups.push(group(t('Project-inside presets'), [pictured(new Option(listing.project_printer || 'The project names no printer', 'project'), own ? selected : null)]));
    if (own) value = 'project';
  }
  const onPreset = selected && selected.source === 'preset';
  const rows = listing.installed.map((model, index) => {
    const option = pictured(new Option(model.model, `m${index}`), model);
    if (model.brand !== model.vendor || !model.model.startsWith(model.brand)) option.title = model.brand;
    if (onPreset && selected.vendor === model.vendor && selected.model === model.model) value = option.value;
    return option;
  });
  // A printer the project is on that is not among those added is still
  // the one in force: it is listed while it is.
  if (onPreset && !selected.added) {
    rows.push(pictured(new Option(selected.model || selected.machine, 'current'), selected));
    value = 'current';
  }
  // A project on its own settings has no printer preset in force.
  const modified = Presets.on('printer')?.modified ?? false;
  const mine = (listing.user || []).map((preset, index) => {
    const option = pictured(new Option(Presets.label(preset.name, onPreset && selected.machine === preset.name && modified), `u${index}`), preset);
    option.title = preset.name;
    if (onPreset && selected.machine === preset.name) value = option.value;
    return option;
  });
  if (mine.length) groups.push(group(t('User presets'), mine));
  // A system printer in force with something set over it has the mark too.
  if (modified && onPreset && !selected.user) {
    const row = rows.find((option) => option.value === value);
    if (row) row.text = Presets.label(row.text, true);
  }
  if (rows.length) groups.push(group(t('System presets'), rows));
  const none = new Option(listing.installed.length ? 'No printer selected' : 'No printer added', '');
  none.disabled = true;
  none.hidden = true;
  select.replaceChildren(none, ...groups, el('hr'), new Option(t('Select/Remove printers (system presets)'), 'wizard'));
  select.value = value;
  select.dataset.shown = value;
  // The reference's tooltip: the preset's own name.
  select.title = (selected && selected.machine) || '';

  // The picture of the model the project is on, where it is known.
  const onModel = selected && selected.vendor && selected.model ? selected : null;
  showCover($('printer-cover'), onModel ? onModel.vendor : '', onModel ? onModel.model : '');

  // Nozzles: the sizes the model has presets for, which pick among them;
  // on a printer with several nozzles, each of them as the preset has it.
  const printer = state.settled.printer;
  const entry = onPreset ? listing.installed.find((model) => model.vendor === selected.vendor && model.model === selected.model) : null;
  let variants = entry ? entry.variants.map((variant) => variant.variant) : [];
  if (selected && selected.variant && !variants.includes(selected.variant)) variants = [selected.variant, ...variants];
  const diameter = $('nozzle-diameter');
  fill(diameter, variants, selected ? selected.variant : null, variants.map((variant) => `${variant}mm`));
  diameter.disabled = !onPreset || variants.length < 2;
  const sizes = printer ? printer.nozzle_diameter || [] : [];
  const several = sizes.length > 1;
  $('nozzle-title').textContent = several ? t('Nozzles') : t('Nozzle');
  const bambu = printer && (printer.vendor === 'BBL' || (printer.model || '').startsWith('Bambu Lab'));
  $('nozzle-tiles').hidden = !several;
  $('nozzle-tiles').replaceChildren(...(several ? sizes : []).map((size, index) => {
    const name = sizes.length === 2 && bambu ? ['L', 'R'][index] : `T${index + 1}`;
    const tile = el('span', { className: 'nozzle-tile', title: `${name}: ${size} mm` },
      el('small', { textContent: name }), el('b', { textContent: String(size) }));
    return tile;
  }));
  $('nozzles').hidden = !(printer && (variants.length || several));
}

// Puts the project shown on a printer and shows what came of it: the
// process, the filaments, the bed type and the bed all follow.
async function selectPrinter(args) {
  const id = activeId();
  let project;
  try {
    project = await call('printer_select', { project: id, ...args });
  } catch (error) {
    // What cannot be selected is said, and the list goes back.
    showPrinterBlock();
    throw error;
  }
  if (project.id !== activeId()) return;
  state.project = project;
  say('');
  await showSidebar();
  await showStage();
  await refreshStrip();
}

async function printerPicked() {
  const select = $('printer');
  const value = select.value;
  if (value === 'wizard') {
    // Not a printer: the list stays on the one it was on.
    select.value = select.dataset.shown;
    await openPrinterSelection();
  } else if (value === 'project') {
    // Its own settings, the bed type among them.
    await setProject({ source: 'project', bed_type: null });
  } else if (value.startsWith('m') || value.startsWith('u')) {
    // What is set over the printer it leaves is asked about first.
    const answer = await Presets.beforeLeaving('printer', 1, select.selectedOptions[0] ? select.selectedOptions[0].label : '');
    if (answer === null) {
      select.value = select.dataset.shown;
      return;
    }
    if (value.startsWith('u')) {
      const preset = state.printers.user[Number(value.slice(1))];
      await selectPrinter({ vendor: preset.vendor, machine: preset.name });
    } else {
      const model = state.printers.installed[Number(value.slice(1))];
      await selectPrinter({ vendor: model.vendor, model: model.model });
    }
  }
}

// ---- "Select/Remove printers" ----

const ptColumns = [
  { key: 'enabled', label: 'Enabled', filter: 'set', cls: 'check',
    text: (row) => (row.enabled ? 'Enabled' : 'Not enabled'), rank: (row) => (row.enabled ? 0 : 1) },
  { key: 'icon', label: '', cls: 'icon' },
  { key: 'brand', label: 'Brand', filter: 'set', find: true, text: (row) => row.brand },
  { key: 'name', label: 'Model', filter: 'text', text: (row) => row.name },
  { key: 'x', label: 'X mm', filter: 'range', cls: 'num', tip: 'Bed width, from the printer\'s default (0.4 mm nozzle) preset' },
  { key: 'y', label: 'Y mm', filter: 'range', cls: 'num', tip: 'Bed depth, from the printer\'s default (0.4 mm nozzle) preset' },
  { key: 'z', label: 'Z mm', filter: 'range', cls: 'num', tip: 'Printable height, from the printer\'s default (0.4 mm nozzle) preset' },
  { key: 'toolheads', label: 'Toolheads', filter: 'set', cls: 'num', tip: 'Number of extruders / toolheads',
    text: (row) => (row.toolheads === null ? '—' : String(row.toolheads)) },
];

const pt = {
  rows: [],
  view: [],
  search: '',
  // The column sorted by and which way: 1 up, -1 down, 0 the order given.
  sort: { key: null, way: 0 },
  // By column: a set of the values shown, a text, or { min, max }.
  filters: new Map(),
  pop: null,
  // The setup wizard (Help > "Setup Wizard", the first run): "Next" goes
  // on to "Filament Selection", which keeps both (setup.js).
  wizard: false,
};

const ptCollator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });
const ptSize = (value) => {
  if (value === null || value === undefined) return '—';
  const whole = Math.round(value);
  return Math.abs(value - whole) < 0.1 ? String(whole) : value.toFixed(1);
};
const ptText = (column, row) => (column.text ? column.text(row) : ptSize(row[column.key]));

function ptMatches(row) {
  const words = pt.search.toLowerCase().match(/\S+/g) || [];
  if (!words.every((word) => row.hay.includes(word))) return false;
  for (const [key, filter] of pt.filters) {
    const column = ptColumns.find((known) => known.key === key);
    if (column.filter === 'set') {
      // Whether a printer is enabled is filtered as it was when the
      // filter was set, so a row does not leave as it is ticked.
      const value = key === 'enabled' ? (row.wasEnabled ? 'Enabled' : 'Not enabled') : column.text(row);
      if (!filter.has(value)) return false;
    } else if (column.filter === 'text') {
      if (!row.name.toLowerCase().includes(filter.toLowerCase())) return false;
    } else {
      const value = row[key];
      if (value === null || value === undefined) return false;
      if (filter.min !== null && value < filter.min) return false;
      if (filter.max !== null && value > filter.max) return false;
    }
  }
  return true;
}

function ptSorted(rows) {
  const { key, way } = pt.sort;
  if (!key || !way) return rows;
  const column = ptColumns.find((known) => known.key === key);
  const sorted = [...rows];
  sorted.sort((a, b) => {
    let order;
    if (column.rank) order = column.rank(a) - column.rank(b);
    else if (column.filter === 'text' || key === 'brand') order = ptCollator.compare(column.text(a), column.text(b));
    else {
      // A printer without the value comes last either way.
      const [x, y] = [a[key], b[key]];
      if (x === null || y === null) return (x === null) - (y === null) || a.order - b.order;
      order = x - y;
    }
    return order * way || a.order - b.order;
  });
  return sorted;
}

// The pictures of the rows in sight, and no others.
let ptSeen = null;

function ptRender() {
  ptClosePop();
  const filtered = pt.search.trim() !== '' || pt.filters.size > 0;
  pt.view = ptSorted(pt.rows.filter(ptMatches));

  $('pt-head').replaceChildren(...ptColumns.map((column) => {
    const cell = el('th', { className: column.cls || '' });
    if (!column.label) return cell;
    const sorting = pt.sort.key === column.key ? pt.sort.way : 0;
    cell.setAttribute('aria-sort', sorting > 0 ? 'ascending' : sorting < 0 ? 'descending' : 'none');
    const title = el('button', { type: 'button', className: 'th-sort', title: column.tip ? `${column.tip}\nClick to sort` : 'Click to sort' },
      column.label, el('span', { className: 'th-way', textContent: sorting > 0 ? ' ▲' : sorting < 0 ? ' ▼' : '' }));
    title.addEventListener('click', () => {
      // Up, down, then back to the order given.
      const next = pt.sort.key !== column.key ? 1 : pt.sort.way === 1 ? -1 : pt.sort.way === -1 ? 0 : 1;
      pt.sort = { key: next ? column.key : null, way: next };
      ptRender();
    });
    const funnel = el('button', { type: 'button', className: `th-filter${pt.filters.has(column.key) ? ' on' : ''}`, title: `Filter ${column.label}` });
    funnel.setAttribute('aria-label', `Filter ${column.label}`);
    funnel.setAttribute('aria-haspopup', 'dialog');
    funnel.innerHTML = '<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M1,1.5h10L7.2,6.2v4.3L4.8,9.3V6.2Z"/></svg>';
    funnel.addEventListener('click', (event) => { event.stopPropagation(); ptOpenPop(column, funnel); });
    cell.append(title, funnel);
    return cell;
  }));

  if (ptSeen) ptSeen.disconnect();
  ptSeen = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      ptSeen.unobserve(entry.target);
      showCover(entry.target, entry.target.dataset.vendor, entry.target.dataset.model);
    }
  }, { root: $('pt-scroll'), rootMargin: '200px' });

  $('pt-body').replaceChildren(...pt.view.map((row) => {
    const box = el('input', { type: 'checkbox', checked: row.enabled });
    box.setAttribute('aria-label', `Enable ${row.name}`);
    const picture = el('span', { className: 'cover' });
    if (row.cover) {
      picture.dataset.vendor = row.vendor;
      picture.dataset.model = row.model;
      ptSeen.observe(picture);
    }
    const line = el('tr', { className: row.enabled ? 'on' : '' },
      el('td', { className: 'check' }, box),
      el('td', { className: 'icon' }, picture),
      ...ptColumns.slice(2).map((column) => {
        const text = ptText(column, row);
        return el('td', { className: `${column.cls || ''}${text === '—' ? ' na' : ''}`, textContent: text, title: column.cls ? '' : text });
      }));
    // A click anywhere on the row ticks or unticks it.
    line.addEventListener('click', (event) => {
      if (event.target !== box) box.checked = !box.checked;
      row.enabled = box.checked;
      line.classList.toggle('on', row.enabled);
      $('pt-error').hidden = true;
      ptCount();
    });
    return line;
  }));
  $('pt-none').hidden = pt.view.length > 0;

  // The search and filters in force, each to take away.
  const chips = [...pt.filters].map(([key, filter]) => {
    const column = ptColumns.find((known) => known.key === key);
    let said;
    if (column.filter === 'set') said = [...filter].join(', ');
    else if (column.filter === 'text') said = `contains "${filter}"`;
    else said = [filter.min !== null ? `≥ ${filter.min}` : '', filter.max !== null ? `≤ ${filter.max}` : ''].filter(Boolean).join(', ');
    const remove = el('button', { type: 'button', className: 'icon', title: 'Remove this filter', textContent: '✕' });
    remove.setAttribute('aria-label', `Remove the filter on ${column.label}`);
    remove.addEventListener('click', () => { pt.filters.delete(key); ptRender(); });
    return el('span', { className: 'chip' }, el('span', { className: 'chip-key', textContent: `${column.label}:` }), ` ${said} `, remove);
  });
  if (filtered) {
    const clear = el('button', { type: 'button', className: 'link', textContent: 'Clear search and filters' });
    clear.addEventListener('click', () => { pt.search = ''; $('pt-search').value = ''; pt.filters.clear(); ptRender(); });
    chips.push(clear);
  }
  $('pt-chips').replaceChildren(...chips);
  $('pt-chips').hidden = chips.length === 0;
  ptCount();
}

function ptCount() {
  const filtered = pt.search.trim() !== '' || pt.filters.size > 0;
  const on = pt.rows.filter((row) => row.enabled).length;
  const [n, shown] = [pt.rows.length, pt.view.length];
  $('pt-count').textContent = filtered ? `${shown} of ${n} shown · ${on} enabled` : `${n} printers · ${on} enabled`;
  $('pt-scope').textContent = filtered ? `All / Clear all: the ${shown} shown` : 'All / Clear all: every printer';
  const shownOn = pt.view.filter((row) => row.enabled).length;
  const tip = `Applies to the ${shown} printers shown by the search and filters (${shownOn} of them enabled)`;
  $('pt-all').title = tip;
  $('pt-clear').title = tip;
}

// "All" and "Clear all" act on the rows shown.
function ptSetShown(on) {
  for (const row of pt.view) row.enabled = on;
  $('pt-error').hidden = true;
  const at = $('pt-scroll').scrollTop;
  ptRender();
  $('pt-scroll').scrollTop = at;
}

function ptClosePop() {
  $('pt-pop').hidden = true;
  $('pt-pop').replaceChildren();
  pt.pop = null;
}

// A column's filter: the values to show, a text the model contains, or
// the least and the most of a size.
function ptOpenPop(column, anchor) {
  if (pt.pop === column.key) { ptClosePop(); return; }
  const pop = $('pt-pop');
  const apply = (filter) => {
    if (filter === null) pt.filters.delete(column.key); else pt.filters.set(column.key, filter);
    if (column.key === 'enabled') for (const row of pt.rows) row.wasEnabled = row.enabled;
    ptRender();
  };
  const parts = [el('h3', { textContent: `Filter ${column.label}` })];
  if (column.filter === 'set') {
    const values = [...new Set(pt.rows.map((row) => column.text(row)))];
    if (column.key === 'enabled') values.sort((a, b) => (a === 'Enabled' ? -1 : 1) - (b === 'Enabled' ? -1 : 1));
    else values.sort(ptCollator.compare);
    const now = pt.filters.get(column.key);
    const boxes = values.map((value) => {
      const box = el('input', { type: 'checkbox', checked: !now || now.has(value) });
      box.dataset.value = value;
      return el('label', {}, box, value);
    });
    const list = el('div', { className: 'check-list' }, ...boxes);
    const take = () => {
      const ticked = [...list.querySelectorAll('input')].filter((box) => box.checked).map((box) => box.dataset.value);
      apply(ticked.length === values.length ? null : new Set(ticked));
    };
    const every = (on) => () => {
      // Those in sight, when the list is searched.
      for (const label of list.children) if (!label.hidden) label.querySelector('input').checked = on;
    };
    if (column.find) {
      const find = el('input', { type: 'search', placeholder: 'Find…' });
      find.setAttribute('aria-label', `Find a ${column.label.toLowerCase()}`);
      find.addEventListener('input', () => {
        const wanted = find.value.trim().toLowerCase();
        for (const label of list.children) label.hidden = !label.textContent.toLowerCase().includes(wanted);
      });
      parts.push(find);
    }
    const all = el('button', { type: 'button', className: 'link', textContent: 'Select all' });
    const no = el('button', { type: 'button', className: 'link', textContent: 'Select none' });
    all.addEventListener('click', every(true));
    no.addEventListener('click', every(false));
    const ok = el('button', { type: 'button', className: 'primary', textContent: t('OK') });
    ok.addEventListener('click', take);
    parts.push(el('div', { className: 'row' }, all, no), list, el('div', { className: 'row end' }, ok));
  } else if (column.filter === 'text') {
    const input = el('input', { type: 'text', placeholder: 'Contains…', value: pt.filters.get(column.key) || '' });
    input.setAttribute('aria-label', `${column.label} contains`);
    const take = () => apply(input.value.trim() === '' ? null : input.value.trim());
    input.addEventListener('keydown', (event) => { if (event.key === 'Enter') { event.preventDefault(); take(); } });
    const ok = el('button', { type: 'button', className: 'primary', textContent: t('OK') });
    ok.addEventListener('click', take);
    parts.push(input, el('div', { className: 'row end' }, ok));
  } else {
    const now = pt.filters.get(column.key) || { min: null, max: null };
    const least = el('input', { type: 'number', min: 0, step: 'any', value: now.min === null ? '' : now.min });
    const most = el('input', { type: 'number', min: 0, step: 'any', value: now.max === null ? '' : now.max });
    const read = (input) => (input.value.trim() === '' || !Number.isFinite(Number(input.value)) ? null : Number(input.value));
    const take = () => {
      const [min, max] = [read(least), read(most)];
      apply(min === null && max === null ? null : { min, max });
    };
    for (const input of [least, most]) {
      input.addEventListener('keydown', (event) => { if (event.key === 'Enter') { event.preventDefault(); take(); } });
    }
    const ok = el('button', { type: 'button', className: 'primary', textContent: t('OK') });
    ok.addEventListener('click', take);
    parts.push(el('label', {}, 'At least', least), el('label', {}, 'At most', most),
      el('p', { className: 'quiet', textContent: 'In mm. Printers without this value are hidden while a limit is set.' }),
      el('div', { className: 'row end' }, ok));
  }
  pop.replaceChildren(...parts);
  pop.hidden = false;
  pt.pop = column.key;
  // Under its funnel, inside the dialog.
  const frame = $('printers-dialog').getBoundingClientRect();
  const from = anchor.getBoundingClientRect();
  const box = pop.getBoundingClientRect();
  pop.style.left = `${Math.max(8, Math.min(from.left - frame.left, frame.width - box.width - 8))}px`;
  pop.style.top = `${from.bottom - frame.top + 4}px`;
  const first = pop.querySelector('input');
  if (first) first.focus();
}

async function openPrinterSelection(options = {}) {
  const dialog = $('printers-dialog');
  if (dialog.open) return;
  pt.wizard = Boolean(options.wizard);
  // The guide's printer page has "Next" (page 21); alone it has "Confirm" (page 24).
  $('pt-confirm').textContent = pt.wizard ? t('Next') : t('Confirm');
  const catalogue = await call('printer_catalogue');
  pt.rows = catalogue.models.map((model, order) => ({
    ...model,
    order,
    wasEnabled: model.enabled,
    hay: `${model.brand}\u0000${model.vendor}\u0000${model.name}`.toLowerCase(),
  }));
  pt.search = '';
  pt.sort = { key: null, way: 0 };
  pt.filters.clear();
  $('pt-search').value = '';
  $('pt-error').hidden = true;
  $('pt-unread').hidden = catalogue.unread.length === 0;
  $('pt-unread').textContent = catalogue.unread.length
    ? `Not listed, because their profiles could not be read: ${catalogue.unread.join('; ')}`
    : '';
  ptRender();
  dialog.showModal();
  $('pt-scroll').scrollTop = 0;
  $('pt-search').focus();
}

// The printers added have changed: every project is shown on the
// printer it is on now, and a project that was moved is named.
async function printersChanged(answer) {
  for (const moved of answer.switched) {
    const mine = state.kept.get(moved.project);
    if (mine) mine.thumbKey = '';
  }
  await showProject();
  await refreshStrip();
  say(answer.switched.map((moved) =>
    `"${moved.name}" is now on ${moved.to}: ${moved.from} is no longer among the printers added.`).join(' '));
}

async function ptConfirm() {
  const models = pt.rows.filter((row) => row.enabled).map((row) => ({ vendor: row.vendor, model: row.model }));
  if (pt.wizard && models.length) {
    // Nothing is kept until "Finish" on the filament page.
    $('printers-dialog').close();
    await Setup.openFilamentSelection(models);
    return;
  }
  let answer;
  try {
    answer = await call('printers_save', { project: activeId(), models });
  } catch (error) {
    // "At least one printer must be selected.", or what went wrong.
    $('pt-error').textContent = error.message;
    $('pt-error').hidden = false;
    return;
  }
  $('printers-dialog').close();
  await printersChanged(answer);
}

// Closed without confirming. On a first run, with no printer added, the
// reference adds its usual one so there is a printer to slice for.
async function ptCancel() {
  $('printers-dialog').close();
  if (state.printers && state.printers.first_run) {
    await printersChanged(await call('printers_default', { project: activeId() }));
  }
}

// With no printer added, the selection opens by itself, as the
// reference's setup does on its first run.
async function firstRun() {
  if (state.printers && state.printers.first_run) await openPrinterSelection({ wizard: true });
}

function initPrinters() {
  // The printer's list: the whole head of its card opens it; each printer has its picture, and the entry
  // that opens the printer selection its gear.
  listbox($('printer'), {
    trigger: $('printer-row'),
    decorate(row, option) {
      if (option.value === 'wizard') {
        row.classList.add('lb-action');
        row.insertAdjacentHTML('afterbegin', '<svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="2.2"/><path d="M8,1.5v2M8,12.5v2M1.5,8h2M12.5,8h2M3.4,3.4l1.4,1.4M11.2,11.2l1.4,1.4M3.4,12.6l1.4-1.4M11.2,4.8l1.4-1.4"/></svg>');
        return;
      }
      const picture = el('span', { className: 'cover' });
      showCover(picture, option.dataset.vendor || '', option.dataset.model || '');
      row.prepend(picture);
    },
  });
  listbox($('nozzle-diameter'), { bare: true });
  $('printer').addEventListener('change', () => attempt(printerPicked));
  $('nozzle-diameter').addEventListener('change', () => attempt(() => selectPrinter({ variant: $('nozzle-diameter').value })));
  $('printer-settings').addEventListener('click', () => attempt(openPrinterSelection));
  $('pt-search').addEventListener('input', () => { pt.search = $('pt-search').value; ptRender(); });
  $('pt-all').addEventListener('click', () => ptSetShown(true));
  $('pt-clear').addEventListener('click', () => ptSetShown(false));
  $('pt-confirm').addEventListener('click', () => attempt(ptConfirm));
  $('pt-cancel').addEventListener('click', () => attempt(ptCancel));
  const dialog = $('printers-dialog');
  // Escape closes what is open, innermost first: a filter, then the dialog.
  dialog.addEventListener('cancel', (event) => {
    event.preventDefault();
    if (pt.pop) ptClosePop(); else attempt(ptCancel);
  });
  dialog.addEventListener('pointerdown', (event) => {
    if (pt.pop && !$('pt-pop').contains(event.target) && !event.target.closest('.th-filter')) ptClosePop();
  });
}
