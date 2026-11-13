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
  const STAR = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 1.4l1.95 4.1 4.45.55-3.27 3.08.84 4.42L8 11.4l-3.97 2.15.84-4.42L1.6 6.05l4.45-.55z"/></svg>';

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
      // The star (preset_manager.js `COL_STAR`): applied at once.
      const star = el('button', { type: 'button', className: `pt-star${row.star ? ' on' : ''}`, innerHTML: STAR });
      star.title = row.star ? 'Remove from favourites' : 'Add to favourites';
      star.setAttribute('aria-label', `${star.title}: ${row.name}`);
      star.setAttribute('aria-pressed', String(row.star));
      star.addEventListener('click', () => attempt(() => fmStar(row)));
      return el('tr', { className: row.enabled ? 'on' : '' },
        el('td', { className: 'check' }, box),
        el('td', { className: 'star' }, star),
        el('td', { textContent: row.name, title: row.members.join('\n') }),
        el('td', { textContent: row.vendor || '—' }),
        el('td', { textContent: row.type || '—' }),
        el('td', { textContent: printers || '—', title: row.printers.join('\n') }),
        el('td', { textContent: row.source === 'User' ? t('User') : 'System' }));
    }));
    const on = fm.rows.filter((row) => row.enabled).length;
    $('fm-count').textContent = `${shown.length} of ${fm.rows.length} shown · ${on} enabled`;
  }

  // Every row of the same favourite key follows (preset_manager.js `star`).
  async function fmStar(row) {
    const answer = await call('favorite_toggle', { key: row.fav });
    const keys = new Set(answer.favs);
    for (const other of fm.rows) other.star = keys.has(other.fav);
    favourites.keys = keys;
    fmDraw();
    presetsByMachine.clear();
    await showSidebar();
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
      fm.rows = (await call('filament_manager', { project: activeId() })).rows;
      fmDraw();
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
    $('fm-search').addEventListener('input', () => { fm.search = $('fm-search').value; fmDraw(); });
    $('fm-close').addEventListener('click', () => $('fm-dialog').close());
  }

  return { init, openFilamentSelection, openFilamentManager, openCreatePrinter, openCreateFilament };
})();
