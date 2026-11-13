'use strict';

// Preferences (Preferences.cpp `PreferencesDialog`): the dialog, drawn
// from the session's description of it (`preferences`, src/api/prefs.rs),
// and what the page does for each setting: the theme, the camera's
// buttons and wheel (gl/stage.js reads `Prefs.get`), the units, the view
// type Preview opens with, the print button's last action, and the
// timers of "Auto-Backup" and "Auto-Save project". Every value is kept
// by the session in the app's own settings.
//
// Loaded before app.js: `call`, `t`, `el`, `$`, `confirmBox` and the like
// are app.js's, and are only used once the page runs.

/* global call, t, el, $, attempt, say, confirmBox, listbox, I18n, applyListing, refreshStrip, showProject,
   showButtons, saveProject, EdgeTheme */

const Prefs = (() => {
  let values = {};
  let described = null;
  const listeners = [];
  const timers = { backup: 0, autosave: 0 };

  const flag = (key) => values[key] === 'true' || values[key] === '1';

  // What the session keeps, put to work.
  function apply() {
    // "Enable Dark mode": none kept follows the system.
    const dark = values.dark_color_mode;
    EdgeTheme.set(dark === 'true' ? 'dark' : dark === 'false' ? 'light' : null);
    schedule();
    for (const listener of listeners) listener(values);
  }

  async function load() {
    const answer = await call('preferences');
    described = answer;
    values = answer.values;
    apply();
    return values;
  }

  async function set(key, value) {
    const answer = await call('preference_set', { key, value: String(value) });
    values = answer.values;
    apply();
    return values[key];
  }

  // "Auto-Backup" every `backup_interval` seconds, "Auto-Save project"
  // every `autosave_interval` (MainFrame.cpp `update_autosave_timer`).
  function schedule() {
    clearInterval(timers.backup);
    clearInterval(timers.autosave);
    timers.backup = 0;
    timers.autosave = 0;
    const seconds = (key) => Math.max(1, Number(values[key]) || 0) * 1000;
    if (flag('backup_switch')) {
      timers.backup = setInterval(() => call('backup_write').catch(() => {}), seconds('backup_interval'));
    }
    if (flag('autosave_switch')) {
      timers.autosave = setInterval(() => attempt(autosave), seconds('autosave_interval'));
    }
  }

  // Each project already saved once to a project file, with unsaved
  // changes: saved to it as "Save Project" saves it, quietly. One that
  // would ask (something its file held would not be kept) is left.
  async function autosave() {
    const { due } = await call('autosave_due');
    for (const id of due) {
      const answer = await call('project_save', { project: id });
      if (answer.saved) say(`Auto-saved ${answer.path}.`);
    }
    if (due.length) await refreshStrip();
  }

  // At the start: a backup left by a run that did not end as it should
  // is offered back (Plater.cpp: "Previous unsaved project detected, do
  // you want to restore it?").
  async function offerBackups() {
    const { backups } = await call('backup_list');
    if (!backups.length) return;
    const names = backups.map((backup) => backup.name).join('\n');
    const yes = await confirmBox(`EdgeSlicerRS - ${t('Restore')}`, `${t('Previous unsaved project detected, do you want to restore it?')}\n\n${names}`, t('Yes'), t('No'));
    if (yes) {
      const listing = await call('backup_restore');
      await applyListing(listing);
      await showProject();
    } else {
      await call('backup_clear');
    }
  }

  // ---- units (Preferences > Units, `use_inches`) ----

  const IN = 25.4;
  const OZ = 28.34952;
  const imperial = () => values.use_inches === '1';
  // GizmoObjectManipulation.cpp: mm or in, a field's number and its unit.
  const lengthUnit = () => (imperial() ? t('in') : t('mm'));
  const fromMm = (mm) => (imperial() ? mm / IN : mm);
  const toMm = (shown) => (imperial() ? shown * IN : shown);
  // GCodeViewer.cpp: "%.2f in" and "%.2f oz", or "%.2f m" and "%.2f g".
  const filamentText = (metres, grams, number) => {
    const length = imperial() ? `${number((metres * 1000) / IN, 2)} in` : `${number(metres, 2)} m`;
    if (grams === null || grams === undefined) return length;
    return `${length}  ${imperial() ? `${number(grams / OZ, 2)} oz` : `${number(grams, 2)} g`}`;
  };

  // ---- the view type Preview opens with (`preview_default_view_type`) ----

  function defaultViewType(filaments) {
    const types = (described && described.view_types) || [];
    const named = (key) => (types.find((type) => type.key === key) || {}).name;
    const chosen = values.preview_default_view_type || 'auto';
    if (chosen === 'last') return named(values.preview_last_view_type) || 'Line Type';
    return named(chosen) || (filaments > 1 ? 'Filament' : 'Line Type');
  }

  function viewTypePicked(name) {
    const types = (described && described.view_types) || [];
    const type = types.find((one) => one.name === name);
    if (type && values.preview_last_view_type !== type.key) set('preview_last_view_type', type.key).catch(() => {});
  }

  // ---- the print button's last action (`remember_print_action`) ----

  let printChoice = null;
  function printAction() {
    if (printChoice) return printChoice;
    return flag('remember_print_action') ? values.print_action_last || null : null;
  }
  function choosePrintAction(what) {
    printChoice = what;
    if (flag('remember_print_action')) set('print_action_last', what).catch(() => {});
  }

  // ---- the dialog ----

  let page = 0;

  function control(item, shown) {
    const value = shown[item.key] ?? '';
    if (item.kind === 'flag') {
      const box = el('input', { type: 'checkbox', checked: value === 'true' || value === '1' });
      // The theme's box shows the theme in force while none is kept.
      if (item.key === 'dark_color_mode' && value === '') box.checked = !EdgeTheme.light();
      box.addEventListener('change', () => { shown[item.key] = box.checked ? 'true' : 'false'; draw(shown); });
      return box;
    }
    if (item.kind === 'choice') {
      const select = el('select');
      for (const option of item.options) select.append(new Option(t(option.label) + option.after, option.value));
      select.value = value;
      select.dataset.value = value;
      select.addEventListener('change', () => { shown[item.key] = select.value; });
      return select;
    }
    const field = el('input', { type: 'number', value, step: item.whole ? '1' : 'any', min: String(item.least), max: String(item.most) });
    field.addEventListener('change', () => { shown[item.key] = field.value.trim(); });
    return field;
  }

  // The settings shown, as they stand in the dialog (not yet kept).
  function draw(shown) {
    const pages = described.pages;
    const tabs = $('prefs-tabs');
    tabs.replaceChildren(...pages.map((one, at) => {
      const tab = el('button', { type: 'button', textContent: t(one.title) });
      tab.setAttribute('role', 'tab');
      tab.setAttribute('aria-selected', String(at === page));
      tab.addEventListener('click', () => { page = at; draw(shown); });
      return tab;
    }));
    const body = $('prefs-body');
    body.replaceChildren();
    for (const group of pages[page].groups) {
      body.append(el('h3', { className: 'caps', textContent: t(group.title) }));
      const card = el('div', { className: 'card' });
      if (page === 0 && group === pages[0].groups[0]) {
        // "Language", first of the reference's General Settings.
        const select = el('select', { id: 'prefs-language' });
        select.append(...I18n.languages.map((language) => new Option(language.name, language.code)));
        select.value = shown.language;
        select.dataset.value = shown.language;
        select.addEventListener('change', () => { shown.language = select.value; });
        card.append(el('div', { className: 'card-row' }, el('label', { htmlFor: 'prefs-language', textContent: t('Language') }), select));
        listbox(select, { bare: true });
      }
      for (const item of group.items) {
        const row = el('div', { className: `card-row${item.under ? ' prefs-under' : ''}` });
        const id = `pref-${item.key}`;
        const input = control(item, shown);
        input.id = id;
        const label = el('label', { htmlFor: id, textContent: t(item.label), title: t(item.tip) });
        input.title = t(item.tip);
        // A setting under another ("every") is off while that one is.
        if (item.under) input.disabled = !(shown[item.under] === 'true' || shown[item.under] === '1');
        row.append(label, item.kind === 'number' && item.unit ? el('span', { className: 'unit' }, input, ` ${t(item.unit)}`) : input);
        card.append(row);
        // The page's own list, not the system's (STYLE.md).
        if (item.kind === 'choice') listbox(input, { bare: true });
      }
      body.append(card);
    }
  }

  async function show() {
    if (!described) await load();
    const shown = { ...values, language: I18n.language };
    page = 0;
    draw(shown);
    const dialog = $('prefs');
    dialog.returnValue = 'cancel';
    dialog.addEventListener('close', () => {
      if (dialog.returnValue !== 'ok') return;
      attempt(async () => {
        for (const [key, value] of Object.entries(shown)) {
          if (key === 'language' || value === values[key] || (key === 'dark_color_mode' && value === '')) continue;
          await set(key, value);
        }
        if (shown.language !== I18n.language) await I18n.choose(shown.language);
        // What the page shows that the settings change.
        if (typeof showProject === 'function') await showProject();
      });
    }, { once: true });
    dialog.showModal();
  }

  return {
    load, set, show, offerBackups,
    get: (key) => values[key],
    flag,
    onChange: (listener) => listeners.push(listener),
    lengthUnit, fromMm, toMm, filamentText,
    defaultViewType, viewTypePicked, printAction, choosePrintAction,
  };
})();
