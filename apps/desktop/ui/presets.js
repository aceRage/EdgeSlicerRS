'use strict';

// A person's own presets in the page: the lists in the reference's groups
// with the modified mark (PresetComboBoxes.cpp `PlaterPresetComboBox::update`,
// `Preset::label`), each list's cog menu ("Save current %s", "Rename
// preset", "Delete this preset", "Compare presets"), the dialogs of saving
// and renaming (SavePresetDialog.cpp), "Transfer or discard changes"
// (UnsavedChangesDialog.cpp) and "Compare presets" (DiffPresetDialog). The
// session does the work (`preset_check`, `preset_save`, `preset_rename`,
// `preset_delete`, `preset_changes`, `preset_compare`); the page asks.
// Loaded before app.js, whose `call`, `$`, `el`, `state` and the rest are
// used once the page runs.

const Presets = (() => {
  // The title of each kind's settings tab, as "Save current %s" names it
  // (Tab.hpp: TabPrint "Process", TabFilament "Filament", TabPrinter "Machine").
  const TITLES = { process: 'Process', filament: 'Filament', printer: 'Machine' };
  const COG = 'M8,5.5a2.5,2.5 0 1 0 0,5a2.5,2.5 0 1 0 0,-5ZM6.9,1.5h2.2l.4,1.9l1.3.7l1.8-.7l1.1,1.9l-1.4,1.3v1.5l1.4,1.3l-1.1,1.9l-1.8-.7l-1.3.7l-.4,1.9H6.9l-.4-1.9l-1.3-.7l-1.8.7l-1.1-1.9l1.4-1.3V6.6L2.3,5.3l1.1-1.9l1.8.7l1.3-.7Z';

  // The name a list shows for a preset: "* " before it while something is
  // set over it (GUI_App.cpp `update_suffix_modified`, `Preset::label`).
  const label = (name, modified) => (modified ? `${t('*')} ${name}` : name);

  // Fills a preset list in the reference's groups and order: the
  // person's ("User presets"), then the vendor's ("System presets"). A
  // group with nothing in it is left out, as the reference leaves it out.
  function fillList(select, { user = [], system = [], value = null, modified = false }) {
    const group = (title, names) => {
      const node = el('optgroup', { label: title });
      node.append(...names.map((name) => new Option(label(name, modified && name === value), name)));
      return node;
    };
    const groups = [];
    if (user.length) groups.push(group(t('User presets'), user));
    if (system.length) groups.push(group(t('System presets'), system));
    // What the project is on that neither list holds is still listed.
    if (value && !user.includes(value) && !system.includes(value)) groups.unshift(new Option(label(value, modified), value));
    select.replaceChildren(...groups);
    select.disabled = user.length + system.length === 0;
    if (value !== null && value !== undefined) select.value = value;
    select.title = value || '';
  }

  // The "* " of each list, as what is set over its preset now stands.
  function refreshMarks() {
    const mark = (select, current) => {
      if (!select || !current) return;
      for (const option of select.options) {
        if (option.value === current.name) option.text = label(current.name, current.modified);
      }
    };
    mark($('process'), on('process'));
    for (const select of $('filaments').querySelectorAll('select')) mark(select, on('filament', Number(select.dataset.filament)));
    if (typeof showPrinterBlock === 'function' && state.printers) showPrinterBlock();
  }

  // ---- what each kind is on ----

  // The preset a kind (and filament, from 1) is on, whether it is one of
  // the person's, and whether anything is set over it.
  function on(kind, filament = 1) {
    const project = state.project;
    if (!project || project.choices.source !== 'presets') return null;
    const choices = project.choices;
    if (kind === 'process') return { name: choices.process, modified: Object.keys(choices.over || {}).length > 0 };
    if (kind === 'printer') return { name: choices.machine, modified: Object.keys(choices.printer_over || {}).length > 0 };
    const names = choices.filaments || [];
    const own = (choices.filament_over || [])[filament - 1] || {};
    return { name: names[Math.min(filament - 1, names.length - 1)], modified: Object.keys(own).length > 0 };
  }

  // Whether a preset of a kind is one of the person's, by the lists the
  // session gave.
  async function isUsers(kind, name) {
    const mine = await call('user_presets');
    const list = mine[kind] || [];
    return list.some((preset) => preset.name === name);
  }

  // ---- the cog menu ----

  // `which` is the kind, or a function giving { kind, filament } when it
  // is asked (the panel's, which shows a filament's or the printer's).
  function cogButton(which) {
    const button = el('button', { type: 'button', className: 'icon tool preset-cog', title: t('Preset'), ariaLabel: t('Preset') });
    button.append(svg(COG));
    button.setAttribute('aria-haspopup', 'menu');
    button.addEventListener('click', () => {
      const { kind, filament = 1 } = typeof which === 'function' ? which() : { kind: which };
      attempt(() => openMenu(button, kind, filament));
    });
    return button;
  }

  async function openMenu(button, kind, filament) {
    const current = on(kind, filament);
    const box = button.getBoundingClientRect();
    if (!current || !current.name) {
      showMenu(box.left, box.bottom + 4, [[t('Compare presets'), () => attempt(() => compare(kind))]]);
      return;
    }
    const mine = await isUsers(kind, current.name);
    showMenu(box.left, box.bottom + 4, [
      [fmt(t('Save current %s'), t(TITLES[kind])), () => attempt(() => save(kind, filament))],
      // Only the person's own can be renamed or deleted; for a system
      // preset the entries are not there, as the reference hides its
      // delete button.
      ...(mine ? [
        [t('Rename preset'), () => attempt(() => rename(kind, current.name))],
        [t('Delete this preset'), () => attempt(() => remove(kind, current.name))],
      ] : []),
      null,
      [t('Compare presets'), () => attempt(() => compare(kind, current.name))],
    ]);
  }

  // ---- after a change: the project and its lists anew ----

  async function shown(project) {
    if (project && project.id === activeId()) state.project = project;
    presetsByMachine.clear();
    await showSidebar();
    await refreshStrip();
  }

  // ---- "Save preset" and "Rename preset" ----

  let nameDialog = null;
  function nameDialogNodes() {
    if (nameDialog) return nameDialog;
    const title = el('h2', { id: 'pn-title' });
    const caption = el('label', { id: 'pn-label', htmlFor: 'pn-name' });
    const input = el('input', { type: 'text', id: 'pn-name', spellcheck: false, autocomplete: 'off' });
    const message = el('p', { id: 'pn-message', className: 'pn-message', role: 'status' });
    const ok = el('button', { type: 'submit', className: 'primary', value: 'ok', textContent: t('OK') });
    const cancel = el('button', { type: 'submit', value: 'cancel', textContent: t('Cancel'), formNoValidate: true });
    const form = el('form', { method: 'dialog' }, title, caption, input, message, el('div', { className: 'row end' }, ok, cancel));
    const dialog = el('dialog', { id: 'preset-name-dialog', className: 'preset-dialog' }, form);
    dialog.setAttribute('aria-labelledby', 'pn-title');
    form.addEventListener('submit', (event) => {
      dialog.returnValue = event.submitter ? event.submitter.value : 'ok';
    });
    document.body.append(dialog);
    nameDialog = { dialog, title, caption, input, message, ok };
    return nameDialog;
  }

  // Asks for a name as the reference's dialog does: checked as it is
  // typed, OK off while it cannot be had, a warning when it saves over a
  // preset of the person's. Resolves to { name, level } or null.
  async function askName({ heading, caption, kind, filament, preset, rename }) {
    const nodes = nameDialogNodes();
    nodes.title.textContent = heading;
    nodes.caption.textContent = caption;
    const base = { project: activeId(), kind, ...(kind === 'filament' ? { filament } : {}), ...(preset ? { preset } : {}), rename: Boolean(rename) };
    const first = await call('preset_check', base);
    nodes.input.value = rename ? preset : first.suggested || '';
    let level = 'ok';
    let turn = 0;
    const check = async () => {
      const mine = ++turn;
      const answer = await call('preset_check', { ...base, name: nodes.input.value });
      if (mine !== turn) return;
      level = answer.level;
      nodes.message.textContent = answer.message || '';
      nodes.message.className = `pn-message ${answer.level}`;
      nodes.ok.disabled = answer.level === 'invalid' || (rename && nodes.input.value === preset);
    };
    nodes.input.oninput = () => { check().catch(() => {}); };
    await check();
    const promise = modal(nodes.dialog);
    nodes.input.select();
    if (!(await promise)) return null;
    await check();
    if (level === 'invalid') return null;
    return { name: nodes.input.value, level };
  }

  async function save(kind, filament = 1) {
    const title = t(TITLES[kind]);
    const answer = await askName({
      heading: t('Save preset'),
      caption: fmt(t('Save %s as'), title),
      kind,
      filament,
    });
    if (!answer) return false;
    const project = await call('preset_save', {
      project: activeId(),
      kind,
      ...(kind === 'filament' ? { filament } : {}),
      name: answer.name,
      // The dialog said it saves over the preset of that name.
      accept: answer.level === 'warn',
    });
    if (!project.saved) return false;
    await shown(project);
    return true;
  }

  async function rename(kind, preset) {
    const answer = await askName({
      heading: t('Rename preset'),
      caption: t('Name'),
      kind,
      preset,
      rename: true,
    });
    if (!answer) return;
    const project = await call('preset_rename', { project: activeId(), kind, preset, name: answer.name });
    // The pages knew it by its old name: not another preset under what
    // is set over it, so nothing is said of carrying settings over.
    for (const book of [processPages, filamentPages, printerPages]) {
      for (const [whose, name] of book.seen) if (name === preset) book.seen.set(whose, answer.name);
    }
    await shown(project);
  }

  // "Delete this preset": refused with the reference's reason, else its
  // question, then gone.
  async function remove(kind, preset) {
    let answer;
    try {
      answer = await call('preset_delete', { project: activeId(), kind, preset });
    } catch (error) {
      await inform(fmt(t('%1% Preset'), t('Delete')), String(error.message || error));
      return;
    }
    if (!(await confirmBox(answer.title, answer.ask, t('Yes'), t('No')))) return;
    await shown(await call('preset_delete', { project: activeId(), kind, preset, confirm: true }));
  }

  // ---- a table of options by page and group ----

  // The pages of a kind, to show an option's value as its page does (a
  // list's value by its label).
  const bookOf = (kind) => ({ process: processPages, filament: filamentPages, printer: printerPages }[kind]);

  async function valueShown(kind, key, value) {
    const data = await loadPages(bookOf(kind));
    const option = data.options[key.split('#')[0]];
    if (!option) return value;
    if (option.type === 'bool') return value === '1' ? '✓' : value === '0' ? '—' : value;
    if (option.values && option.labels) {
      const at = option.values.indexOf(value);
      if (at >= 0) return option.labels[at];
    }
    return value;
  }

  async function optionTable(kind, pages, heads) {
    const book = bookOf(kind);
    const data = await loadPages(book);
    const body = el('tbody');
    for (const page of pages) {
      const title = page.extruder ? fmt(t(page.title), page.extruder) : t(page.title);
      body.append(el('tr', { className: 'pc-page' }, el('th', { colSpan: 3, scope: 'rowgroup', textContent: title })));
      for (const group of page.groups) {
        if (group.title) body.append(el('tr', { className: 'pc-group' }, el('th', { colSpan: 3, scope: 'rowgroup', textContent: t(group.title) })));
        for (const row of group.rows) {
          const option = data.options[row.key.split('#')[0]];
          const name = option ? option.label : t(row.label);
          const [old, now] = await Promise.all([valueShown(kind, row.key, row.old), valueShown(kind, row.key, row.new)]);
          body.append(el('tr', {},
            el('td', { textContent: name, title: row.key }),
            el('td', { className: 'pc-value', textContent: old, title: old }),
            el('td', { className: 'pc-value', textContent: now, title: now })));
        }
      }
    }
    const head = el('thead', {}, el('tr', {}, ...heads.map((text) => el('th', { scope: 'col', textContent: text, title: text }))));
    return el('table', { className: 'data-table pc-table' }, head, body);
  }

  // ---- "Transfer or discard changes" ----

  let changesDialog = null;
  function changesNodes() {
    if (changesDialog) return changesDialog;
    const text = el('p', { id: 'pc-text', className: 'pc-text' });
    const table = el('div', { className: 'pc-scroll' });
    const buttons = {};
    const foot = el('div', { className: 'dialog-foot' }, el('div', { className: 'grow' }));
    // Transfer is never asked here: where it can be had it is done without
    // asking (beforeLeaving). Cancel stands for the reference's close box.
    for (const [value, words, primary] of [['discard', 'Discard'], ['save', 'Save', true], ['cancel', 'Cancel']]) {
      const button = el('button', { type: 'submit', value, textContent: t(words), className: primary ? 'primary' : '' });
      buttons[value] = button;
      foot.append(button);
    }
    const title = el('h2', { id: 'pc-title', textContent: t('Transfer or discard changes') });
    const form = el('form', { method: 'dialog' }, el('div', { className: 'pc-head' }, title, text), table, foot);
    const dialog = el('dialog', { id: 'preset-changes', className: 'preset-dialog wide' }, form);
    dialog.setAttribute('aria-labelledby', 'pc-title');
    form.addEventListener('submit', (event) => { dialog.returnValue = event.submitter ? event.submitter.value : 'cancel'; });
    document.body.append(dialog);
    changesDialog = { dialog, text, table, buttons };
    return changesDialog;
  }

  // Before a kind's preset is left with something set over it: what the
  // reference asks (Tab.cpp `may_discard_current_dirty_preset`). Resolves
  // to what to do: 'transfer' (what is set goes over the new preset),
  // 'discard' (let go of), 'saved' (saved as a preset first) or null
  // (stay). Nothing set: 'none'.
  async function beforeLeaving(kind, filament, to) {
    const current = on(kind, filament);
    if (!current || !current.modified || current.name === to) return 'none';
    const changes = await call('preset_changes', { project: activeId(), kind, ...(kind === 'filament' ? { filament } : {}), to });
    // Nothing the pages show is changed: let go of without asking
    // (Tab.cpp `may_discard_current_dirty_preset`, `getUpdateItemCount`).
    if (!changes.count) return 'none';
    if (changes.transfer) {
      // Where the reference can transfer, it does so without asking
      // (UnsavedChangesDialog.cpp `ShowModal`) and says what was kept.
      say(changes.note);
      // The pages are told the preset changed under what is set, so that
      // they do not say it a second time (settings.js `showSettings`).
      const book = bookOf(kind);
      for (const [whose, name] of book.seen) {
        const [project, slot] = whose.split(':');
        if (Number(project) === activeId() && name === current.name && (kind !== 'filament' || Number(slot) === filament)) book.seen.set(whose, to);
      }
      return 'transfer';
    }
    // A printer, or a filament of another type: save or discard, or stay.
    const nodes = changesNodes();
    nodes.text.textContent = fmt(t('You have changed some settings of preset "%1%".'), changes.name)
      + t('\nYou can save or discard the preset values you have modified.');
    nodes.table.replaceChildren(await optionTable(kind, changes.pages, [t('Settings'), t('Old Value'), t('New Value')]));
    nodes.buttons.discard.title = t('All changes will be discarded.');
    nodes.buttons.save.title = t('Save the selected options.');
    nodes.dialog.returnValue = 'cancel';
    const answer = await new Promise((resolve) => {
      nodes.dialog.addEventListener('close', () => resolve(nodes.dialog.returnValue), { once: true });
      nodes.dialog.showModal();
    });
    if (answer === 'discard') {
      const args = { project: activeId(), reset: true };
      if (kind === 'process') await call('process_set', args);
      else await call('settings_set', { ...args, kind, ...(kind === 'filament' ? { filament } : {}) });
      return 'discard';
    }
    if (answer === 'save') return (await save(kind, filament)) ? 'saved' : null;
    return null;
  }

  // ---- "Compare presets" ----

  let compareDialog = null;
  function compareNodes() {
    if (compareDialog) return compareDialog;
    const kind = el('select', { id: 'cmp-kind', ariaLabel: t('Preset') });
    for (const value of ['printer', 'process', 'filament']) kind.append(new Option(t({ printer: 'Printer', process: 'Process', filament: 'Filament' }[value]), value));
    const left = el('select', { id: 'cmp-left', ariaLabel: t('Preset') });
    const right = el('select', { id: 'cmp-right', ariaLabel: t('Preset') });
    const all = el('input', { type: 'checkbox', id: 'cmp-all' });
    const result = el('div', { className: 'pc-scroll', id: 'cmp-result' });
    const info = el('p', { className: 'quiet', id: 'cmp-info', textContent: t('Select presets to compare') });
    const title = el('h2', { id: 'cmp-title', textContent: t('Compare presets') });
    const close = el('button', { type: 'submit', value: 'cancel', className: 'primary', textContent: t('Close') });
    const form = el('form', { method: 'dialog' },
      el('div', { className: 'pc-head' }, title,
        el('div', { className: 'cmp-pick' }, listbox(kind, { bare: false }), listbox(left), listbox(right)),
        el('label', { className: 'tick' }, all, ` ${t('Show all presets (including incompatible)')}`), info),
      result,
      el('div', { className: 'dialog-foot' }, el('div', { className: 'grow' }), close));
    const dialog = el('dialog', { id: 'preset-compare', className: 'preset-dialog wide' }, form);
    dialog.setAttribute('aria-labelledby', 'cmp-title');
    document.body.append(dialog);
    compareDialog = { dialog, kind, left, right, all, result, info };
    const refresh = () => attempt(() => showComparison());
    kind.addEventListener('change', () => attempt(() => fillCompare(null)));
    all.addEventListener('change', () => attempt(() => fillCompare(null)));
    left.addEventListener('change', refresh);
    right.addEventListener('change', refresh);
    return compareDialog;
  }

  async function fillCompare(first) {
    const nodes = compareNodes();
    const kind = nodes.kind.value;
    const listed = await call('preset_compare', { project: activeId(), kind, all: nodes.all.checked });
    const current = on(kind) ? on(kind).name : null;
    const leftValue = first || nodes.left.value || current;
    const names = [...listed.user, ...listed.system];
    fillList(nodes.left, { user: listed.user, system: listed.system, value: names.includes(leftValue) ? leftValue : names[0] });
    const rightValue = names.includes(nodes.right.value) && nodes.right.value !== nodes.left.value
      ? nodes.right.value : names.find((name) => name !== nodes.left.value) || names[0];
    fillList(nodes.right, { user: listed.user, system: listed.system, value: rightValue });
    await showComparison();
  }

  async function showComparison() {
    const nodes = compareNodes();
    const kind = nodes.kind.value;
    if (!nodes.left.value || !nodes.right.value) {
      nodes.result.replaceChildren();
      return;
    }
    const answer = await call('preset_compare', { project: activeId(), kind, left: nodes.left.value, right: nodes.right.value });
    if (nodes.left.value !== answer.left || nodes.right.value !== answer.right) return;
    if (!answer.count) {
      nodes.info.textContent = 'These two presets are the same.';
      nodes.result.replaceChildren();
      return;
    }
    nodes.info.textContent = t('Select presets to compare');
    nodes.result.replaceChildren(await optionTable(kind, answer.pages, [t('Settings'), answer.left, answer.right]));
  }

  async function compare(kind, name = null) {
    const nodes = compareNodes();
    nodes.kind.value = kind;
    // The closed face follows the attribute (listbox).
    nodes.kind.dataset.value = kind;
    nodes.left.value = '';
    nodes.right.value = '';
    await fillCompare(name);
    nodes.dialog.showModal();
  }


  // ---- "Import Configs..." and "Export Preset Bundle..." (the menu bar) ----

  // After presets came or went: the lists and what they show, anew.
  async function reread() {
    presetsByMachine.clear();
    if (state.project) {
      state.project = await call('project', { project: activeId() });
      await showSidebar();
    }
  }

  // MainFrame.cpp `load_config_file`: a file picked, then the reference's
  // questions ("Untrusted settings in file", "Overwrite config") as the
  // session asks them, then what came of it.
  async function importConfigs() {
    const path = await pick('presets');
    if (!path) return;
    const args = { path };
    for (;;) {
      const answer = await call('preset_import', args);
      if (answer.imported) {
        await reread();
        await inform(answer.title, answer.message);
        return;
      }
      if (answer.untrusted) {
        args.keep_untrusted = !(await confirmBox(answer.title, answer.ask, answer.remove, answer.keep));
      } else if (answer.conflicts) {
        args.overwrite = await confirmBox(answer.title, answer.ask, t('Yes to All'), t('No to All'));
      } else {
        return;
      }
    }
  }

  let exportDialog = null;
  function exportNodes() {
    if (exportDialog) return exportDialog;
    const types = el('div', { className: 'ex-types', role: 'radiogroup' });
    const names = el('div', { className: 'ex-names pc-scroll' });
    const message = el('p', { className: 'pn-message invalid', role: 'status' });
    const ok = el('button', { type: 'submit', className: 'primary', value: 'ok', textContent: t('Export') });
    const cancel = el('button', { type: 'submit', value: 'cancel', textContent: t('Cancel'), formNoValidate: true });
    const title = el('h2', { id: 'ex-title', textContent: t('Export Preset Bundle') });
    const form = el('form', { method: 'dialog' }, title, types, names, message, el('div', { className: 'dialog-foot' }, el('div', { className: 'grow' }), ok, cancel));
    const dialog = el('dialog', { id: 'preset-export', className: 'preset-dialog wide' }, form);
    dialog.setAttribute('aria-labelledby', 'ex-title');
    form.addEventListener('submit', (event) => { dialog.returnValue = event.submitter ? event.submitter.value : 'cancel'; });
    document.body.append(dialog);
    exportDialog = { dialog, types, names, message, ok };
    return exportDialog;
  }

  // ExportConfigsDialog: the kind of bundle, the presets of that kind to
  // put in it (all ticked), a folder, then the archive(s) written.
  async function exportBundle() {
    const choices = await call('preset_export', {});
    const nodes = exportNodes();
    let type = choices.types[0].type;
    const showNames = () => {
      const listed = choices[type] || [];
      nodes.names.replaceChildren(...listed.map((name) => el('label', { className: 'tick', title: name },
        el('input', { type: 'checkbox', value: name, checked: true }), ` ${name}`)));
      nodes.ok.disabled = listed.length === 0;
      nodes.message.textContent = '';
    };
    nodes.types.replaceChildren(...choices.types.map((one) => {
      const input = el('input', { type: 'radio', name: 'ex-type', value: one.type, checked: one.type === type });
      input.addEventListener('change', () => { type = one.type; showNames(); });
      const label = el('label', { className: 'tick' }, input, ` ${one.title}`);
      return one.note ? el('div', {}, label, el('p', { className: 'quiet ex-note', textContent: one.note })) : label;
    }));
    showNames();
    if (!(await modal(nodes.dialog))) return;
    const ticked = [...nodes.names.querySelectorAll('input:checked')].map((input) => input.value);
    const dir = await pick('preset_folder');
    if (!dir) return;
    const args = { path: dir, type, names: ticked };
    for (;;) {
      const answer = await call('preset_export', args);
      if (answer.exported) {
        await inform(t('Export Preset Bundle'), `${answer.message}\n\n${answer.files.join('\n')}`);
        return;
      }
      if (!answer.ask) return;
      args.replace = await confirmBox(t('Export Preset Bundle'), answer.ask, t('Yes'), t('No'));
    }
  }

  // The menu bar's entries call these when they are there (menu.js).
  window.presetImport = () => attempt(importConfigs);
  window.presetExport = () => attempt(exportBundle);

  return { fillList, label, on, refreshMarks, cogButton, beforeLeaving, save, rename, remove, compare };
})();
