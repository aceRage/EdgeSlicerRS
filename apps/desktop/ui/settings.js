'use strict';

// The pages of settings of the three kinds of preset, drawn from data by
// one renderer: the process's in the sidebar ("Quality", "Strength",
// "Speed", "Support", "Multimaterial", "Others"; Tab.cpp `TabPrint::build`),
// and a filament's or the printer's in the panel that takes the
// sidebar's place when one of them is opened ("Filament", "Cooling",
// "Setting Overrides", …; "Basic information", "Machine G-code",
// "Extruder 1", …; `TabFilament::build`, `TabPrinter::build_fff`).
//
// Nothing here names an option: the pages, the groups, every option's
// label, tooltip, kind, unit, limits and list come from `settings_pages`
// (made from the reference's source by tools/make_settings_pages.py), and
// what holds for the project shown comes from `settings` (`process` for
// the process): each option's value by the preset, what is set over it,
// which options the reference's rules hide or grey out, and what the
// engine does with each. An extruder's own option is named `key#n` (n
// from 0), on a page of its own for each extruder.
//
// Loaded before app.js, whose `call`, `$`, `el`, `state`, `listbox`,
// `say` and the rest are used once the page runs.

// The "Advance" switch, one for every page as the reference's is.
const settingsMode = { advanced: false };

// One kind of preset's pages, drawn into the elements named `${prefix}-…`.
function settingsBook(kind, prefix) {
  return {
    kind,
    prefix,
    data: null,          // { pages, options }, asked for once
    built: new Map(),    // page title -> { node, groups: [{ head, body, lines: [{ node, rows }] }] }
    rows: new Map(),     // name (key, or key#n) -> a row of a built page
    answer: null,        // what was last said of the project shown
    page: null,          // the page open
    filament: 1,         // of a filament's pages, which filament
    seen: new Map(),     // "project:filament" -> the preset its settings were last shown over
    asked: 0,            // answers asked for, counted
    taken: 0,            // the number of the one shown
    found: [],           // what the search lists
    at: 0,               // the row of that list the keys are on
  };
}

const processPages = settingsBook('process', 'process');
const filamentPages = settingsBook('filament', 'preset');
const printerPages = settingsBook('printer', 'preset');
// The panel and what it shows: { book } while it is open.
const presetPanel = { book: null };

// What the marks beside an option say.
const NOT_FOLLOWED = 'Not followed: with the settings as they are the engine does not read this setting, so changing it changes nothing in the print. It is not followed yet, or is read only when another setting turns it on.';
const WITH_A_NOTE = 'Followed with a note';
// The reference's own words (Tab.cpp `m_tt_value_revert`, Field.cpp `get_formatted_tooltip_text`).
const REVERT = t('Click to drop current modify and reset to saved value.');
const PARAMETER = t('parameter name');
// The overrides page: an option's tick box turns the filament's own value on.
const OVERRIDE_OFF = { printer: 'Off: the printer\'s value for this filament\'s extruder is used.', process: 'Off: the process\'s value is used.' };

const svg = (path, box = '0 0 16 16') => {
  const node = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  node.setAttribute('viewBox', box);
  node.setAttribute('aria-hidden', 'true');
  const line = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  line.setAttribute('d', path);
  node.append(line);
  return node;
};
const REVERT_ICON = 'M3.5,8a4.5,4.5 0 1 0 1.4,-3.3M3.2,2.6v2.5h2.5';
const EDIT_ICON = 'M10.6,2.4l3,3l-8.1,8.1H2.5v-3ZM9,4l3,3';

// The process's fields keep the names they had (po-…); a filament's are
// fo-…, the printer's mo-… (a machine's).
const bookId = (book, name) => `${{ process: 'po', filament: 'fo', printer: 'mo' }[book.kind]}-${name.replace('#', '-')}`;
const baseKey = (name) => name.split('#')[0];
const extruderOf = (name) => (name.includes('#') ? Number(name.split('#')[1]) : null);

async function loadPages(book) {
  if (book.data) return book.data;
  const data = book.kind === 'process' ? await call('process_pages') : await call('settings_pages', { kind: book.kind });
  if (!settingsMode.loaded) {
    settingsMode.loaded = true;
    settingsMode.advanced = remembered.get('process.advanced') === '1';
  }
  for (const id of ['process-advanced', 'preset-advanced']) $(id).checked = settingsMode.advanced;
  book.data = translatedBook(data);
  return book.data;
}

// The pages' words in the language: the file holds the reference's
// message ids (one file for every language), put in the language here,
// once, as the reference shows each through `_()` (an option's label,
// full label, tooltip and list labels; a group's title; a line's label
// and tooltip). What the code looks things up by stays as it is: the
// keys, the values of a list, the pages' titles (shown with `pageTitle`)
// and the units (shown with `unitFor`).
function translatedBook(data) {
  for (const option of Object.values(data.options)) {
    for (const field of ['label', 'full_label', 'tooltip']) if (option[field]) option[field] = t(option[field]);
    if (option.labels) option.labels = option.labels.map((label) => t(label));
  }
  for (const page of data.pages) {
    for (const group of page.groups) {
      if (group.title) group.title = t(group.title);
      for (const line of group.lines) {
        if (line.label) line.label = t(line.label);
        if (line.tooltip) line.tooltip = t(line.tooltip);
        if (line.labels) for (const key of Object.keys(line.labels)) line.labels[key] = t(line.labels[key]);
      }
    }
  }
  return data;
}

// A page's title in the language; an extruder's ("Extruder 2") is the
// reference's "Extruder %d" with its number.
function pageTitle(book, title) {
  const pages = (book.data && book.data.pages) || [];
  if (pages.some((page) => page.title === title)) return t(title);
  const template = pages.find((page) => page.per === 'extruder' && new RegExp(`^${page.title.replace('%d', '(\\d+)')}$`).test(title));
  if (!template) return t(title);
  return fmt(t(template.title), Number(new RegExp(template.title.replace('%d', '(\\d+)')).exec(title)[1]));
}

// The pages as they stand: of the printer's, an extruder's page for each
// extruder (its options named for it), and not "Motion ability" for a
// flavor it is not shown for. Each is { title, groups, extruder }.
function bookPages(book) {
  const data = book.data;
  if (!data) return [];
  const answer = book.answer;
  if (!answer || !answer.pages) return data.pages.map((page) => ({ title: page.title, groups: page.groups, extruder: null }));
  const away = new Set(answer.away || []);
  return answer.pages.filter((title) => !away.has(title)).map((title) => {
    const own = data.pages.find((page) => page.title === title);
    if (own) return { title, groups: own.groups, extruder: null };
    const template = data.pages.find((page) => page.per === 'extruder' && new RegExp(`^${page.title.replace('%d', '(\\d+)')}$`).test(title));
    const number = Number(new RegExp(template.title.replace('%d', '(\\d+)')).exec(title)[1]);
    return { title, groups: template.groups, extruder: number - 1 };
  });
}

const nameIn = (page, key) => (page.extruder === null ? key : `${key}#${page.extruder}`);

// ---- a value as it is typed and as it is kept ----

// What is typed for an option, as the session takes it, or the reason
// it cannot be taken: the same limits the session holds to, said before
// anything is sent. Nothing typed gives the option back to the preset.
function typedValue(option, typed) {
  const label = option.label;
  const text = typed.trim();
  if (text === '') return { value: null };
  const range = (number, shown, most) => {
    const under = option.min !== undefined && number < option.min;
    const over = most !== undefined && number > most;
    if (!under && !over) return null;
    // As the session says it (settings.rs `out_of_range`): Field.cpp's
    // words, with both limits or with the one there is.
    if (option.min !== undefined && most !== undefined) {
      return `${label}: ${fmt(t('Value %s is out of range. The valid range is from %d to %d.'), shown, option.min, most)}`;
    }
    return option.min !== undefined
      ? `${label}: ${t('Value is out of range.')} (≥ ${option.min})`
      : `${label}: ${t('Value is out of range.')} (≤ ${most})`;
  };
  const number = (written) => (/^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i.test(written.trim()) ? Number(written) : NaN);
  switch (option.type) {
    case 'int': {
      const value = number(text);
      if (!Number.isFinite(value) || !Number.isInteger(value)) return { error: `${label}: ${t('Invalid numeric.')}` };
      return { value: String(value), error: range(value, text, option.max) };
    }
    case 'float': {
      if (text.endsWith('%')) return { error: fmt(t("%s can't be a percentage"), label) };
      const value = number(text);
      if (!Number.isFinite(value)) return { error: `${label}: ${t('Invalid numeric.')}` };
      return { value: String(value), error: range(value, text, option.max) };
    }
    case 'percent': {
      const value = number(text.replace(/%$/, ''));
      if (!Number.isFinite(value)) return { error: `${label}: ${t('Invalid numeric.')}` };
      return { value: `${value}%`, error: range(value, `${value}%`, option.max) };
    }
    case 'float_or_percent': {
      const percent = text.endsWith('%');
      const value = number(text.replace(/%$/, ''));
      if (!Number.isFinite(value)) return { error: `${label}: ${t('Invalid numeric.')}` };
      const most = percent ? option.max : (option.max_literal !== undefined ? option.max_literal : option.max);
      return { value: percent ? `${value}%` : String(value), error: range(value, percent ? `${value}%` : text, most) };
    }
    default:
      return { value: typed };
  }
}

// The unit after a field: the reference's own, and of a length that may
// be a percentage the one that fits what is typed ("mm or %": "mm", and
// nothing after a value that carries its own %).
function unitFor(option, value) {
  const unit = option.unit || '';
  if (option.type !== 'float_or_percent') return unit === '%' && option.type === 'percent' ? '%' : t(unit);
  return String(value).trim().endsWith('%') ? '' : t(unit.replace(/ or %$/, ''));
}

// ---- one option's row ----

// The control for an option, by its kind: a tick box, a number with its
// unit, a list (the page's own), a line of text, or a box of text.
// `commit(value)` sets the option (null gives it back to the preset). A
// process's "strings" are lines, a filament's or an extruder's one text.
function settingsControl(book, option, id, commit) {
  const field = el('span', { className: 'opt-field' });
  let read;
  let show;
  let input;
  const lines = option.type === 'strings' && book.kind === 'process';
  if (option.type === 'bool') {
    input = el('input', { type: 'checkbox', id });
    input.addEventListener('change', () => commit(input.checked ? '1' : '0'));
    field.append(input);
    show = (value) => { input.checked = value === '1'; };
  } else if (option.type === 'enum' && option.values && option.values.length) {
    input = el('select', { id });
    // Named outright: the row is not in the page yet when its list is made.
    input.setAttribute('aria-label', option.label);
    input.addEventListener('change', () => commit(input.value));
    field.append(input);
    show = (value) => {
      const values = [...option.values];
      const labels = [...option.labels];
      // A value the list does not have is shown as it is named.
      if (!values.includes(value)) { values.push(value); labels.push(value); }
      if (input.options.length !== values.length || !values.every((each, index) => input.options[index].value === each)) fill(input, values, value, labels);
      input.value = value;
      // The list's closed face follows what is done to the <select>'s
      // children and attributes; a value alone it would not see.
      input.dataset.shown = value;
    };
  } else if (option.multiline || option.full_width || option.code || lines) {
    input = el('textarea', { id, spellcheck: false, rows: Math.max(2, Math.min(12, Math.round((option.height || 5) / 2))) });
    if (option.code || lines) input.classList.add('code');
    input.addEventListener('change', () => commit(input.value === '' ? null : input.value));
    field.append(input);
    field.classList.add('wide');
    show = (value, preset) => { input.value = value; input.placeholder = preset; };
  } else {
    const numeric = option.type === 'int' || option.type === 'float';
    const texty = option.type === 'string' || option.type === 'strings' || option.type === 'points' || option.type === 'point';
    input = el('input', { id, type: numeric ? 'number' : 'text', autocomplete: 'off', spellcheck: false });
    if (numeric) input.step = 'any';
    if (!numeric && !texty) input.inputMode = 'decimal';
    const unit = el('span', { className: 'opt-unit' });
    const framed = !texty || Boolean(option.unit);
    const frame = framed ? el('span', { className: 'unit' }, input, unit) : input;
    if (!numeric && (!texty || option.type.startsWith('point'))) input.classList.add('number');
    // The percent sign of a percentage is the field's unit, not typed.
    const bare = (value) => (option.type === 'percent' ? String(value).replace(/%$/, '') : value);
    read = () => typedValue(option, input.value);
    input.addEventListener('change', () => {
      const typed = read();
      if (typed.error) { commit(undefined, typed.error); return; }
      commit(typed.value);
    });
    input.addEventListener('input', () => { unit.textContent = unitFor(option, input.value); });
    field.append(frame);
    // A list of values to pick from beside what can be typed.
    if (option.values && option.values.length) {
      const more = el('button', { type: 'button', className: 'icon opt-more', title: option.label, ariaLabel: `${option.label}: values to pick from` });
      more.addEventListener('click', () => {
        const box = more.getBoundingClientRect();
        showMenu(box.left, box.bottom + 2, option.values.map((value, index) => [option.labels[index], () => commit(typedValue(option, value).value), { checked: bare(input.value) === bare(value) }]));
      });
      if (frame === input) {
        const wrap = el('span', { className: 'unit' }, input, more);
        field.replaceChildren(wrap);
      } else {
        frame.append(more);
      }
    }
    show = (value, preset) => {
      input.value = bare(value);
      input.placeholder = bare(preset);
      unit.textContent = unitFor(option, value);
    };
  }
  return { field, input, show };
}

// A row for an option, named `name` (its key, or key#n of extruder n).
// `label` is the line's own name for it, where it has one.
function settingsRow(book, name, option, label = null) {
  const id = bookId(book, name);
  const row = el('div', { className: 'opt' });
  row.dataset.key = name;
  const labelNode = el('label', { className: 'opt-label', htmlFor: id, textContent: label || option.label });
  const follow = el('span', { className: 'opt-follow', hidden: true });
  follow.setAttribute('role', 'img');
  const reset = el('button', { type: 'button', className: 'icon opt-reset', title: REVERT, ariaLabel: `${option.label}: reset to the preset's value` }, svg(REVERT_ICON));
  const commit = (value, error) => attempt(async () => {
    if (error) {
      // Refused here, with the reason: nothing is sent, and the field
      // goes back to what holds.
      showSettingsRow(book, name, { force: true });
      throw new Error(error);
    }
    try {
      await setSettings(book, { key: name, value });
      say('');
    } catch (refused) {
      showSettingsRow(book, name, { force: true });
      throw refused;
    }
  });
  reset.addEventListener('click', () => commit(null));
  const control = settingsControl(book, option, id, commit);
  const head = el('span', { className: 'opt-head' });
  // A filament's override: its tick box turns the filament's own value
  // on (starting from the value in force) and off ("nil").
  let tick = null;
  if (option.override) {
    tick = el('input', { type: 'checkbox', className: 'opt-override' });
    tick.setAttribute('aria-label', `${option.label}: the filament's own value`);
    tick.addEventListener('change', () => {
      const answer = book.answer;
      const from = (answer && answer.fallback && answer.fallback[name]) || '';
      commit(tick.checked ? (from === 'nil' ? '0' : from) : 'nil');
    });
    head.append(tick);
  }
  head.append(labelNode, follow);
  row.append(head, control.field, reset);
  if (control.field.classList.contains('wide')) row.classList.add('wide');
  // The list is the page's own; made once the <select> stands in its row with its label.
  if (control.input.tagName === 'SELECT') {
    const face = listbox(control.input).querySelector('.lb-trigger');
    labelNode.addEventListener('click', (event) => { event.preventDefault(); face.focus(); });
  }
  return { key: name, option, node: row, label: labelNode, follow, reset, control, tick };
}

// Puts what holds for one option on its row: its value, whether it is
// set by hand, hidden or greyed out, and what the engine does with it.
// A field that is being typed in is left as it is, unless `force`d back
// to what holds (a value that was refused).
function showSettingsRow(book, name, { force = false } = {}) {
  const row = book.rows.get(name);
  const answer = book.answer;
  if (!row || !answer) return;
  const option = row.option;
  const key = baseKey(name);
  const preset = answer.values[name] === undefined ? (option.default || '') : answer.values[name];
  const own = Object.prototype.hasOwnProperty.call(answer.set, name);
  let value = own ? answer.set[name] : preset;
  // An override that is off shows what stands in its place.
  const nil = Boolean(row.tick) && (value === 'nil' || value === '');
  if (nil) value = (answer.fallback && answer.fallback[name]) || '';
  const input = row.control.input;
  const typing = document.activeElement === input && input.type !== 'checkbox' && input.value !== row.shown;
  if (force || !typing) {
    row.control.show(value, preset === 'nil' ? '' : preset);
    row.shown = input.value;
  }
  row.node.classList.toggle('set', own);
  row.reset.hidden = !own;
  const fixed = answer.fixed && answer.fixed[key];
  const off = answer.off.has(name) || Boolean(option.readonly) || Boolean(fixed);
  row.node.classList.toggle('off', off);
  row.node.classList.toggle('fixed', Boolean(fixed));
  input.disabled = off || nil;
  if (row.tick) {
    row.tick.checked = !nil;
    row.tick.disabled = off;
    row.tick.title = nil ? OVERRIDE_OFF[option.override] : '';
  }
  const more = row.node.querySelector('.opt-more');
  if (more) more.disabled = off || nil;
  // What the engine does with it. Nothing is said of one the rules have
  // greyed out, or of one not set here: it is off, whoever reads it.
  const notes = answer.notes[name];
  const unread = !off && answer.unread.has(name);
  row.node.classList.toggle('unread', unread);
  row.node.classList.toggle('noted', Boolean(notes));
  const said = notes ? `${WITH_A_NOTE}: ${notes.join('\n')}` : (unread ? NOT_FOLLOWED : '');
  row.follow.hidden = !said;
  row.follow.title = said;
  row.follow.setAttribute('aria-label', said);
  const tip = [said, fixed || '', nil ? OVERRIDE_OFF[option.override] : '', option.tooltip, `${PARAMETER}: ${key}`].filter(Boolean).join('\n\n');
  row.label.title = tip;
}

// ---- a page ----

// A page's groups and rows, built when the page is first opened.
function buildSettingsPage(book, page) {
  const options = book.data.options;
  const node = el('div', { className: 'process-page' });
  const groups = page.groups.map((group) => {
    const head = el('h3', { textContent: group.title });
    const body = el('div', { className: 'opt-group' });
    const lines = group.lines.map((line) => {
      const rows = line.keys.map((key) => {
        const option = options[key];
        // A line's own name for an option ("Target"), the option's full
        // name where it stands alone in the reference ("Maximum speed X").
        const label = (line.labels && line.labels[key]) || (line.full && option.full_label) || null;
        const row = settingsRow(book, nameIn(page, key), option, label);
        book.rows.set(row.key, row);
        return row;
      });
      // Several options under one label: the label, then a row each.
      let box = rows[0].node;
      let title = null;
      if (line.label) {
        title = el('div', { className: 'opt-line-label', textContent: line.label, title: line.tooltip || '' });
        box = el('div', { className: 'opt-line' }, title, ...rows.map((row) => row.node));
      }
      body.append(box);
      return { node: box, rows, title, label: line.label, first: line.keys[0] };
    });
    node.append(head, body);
    return { head, body, lines };
  });
  return { node, groups };
}

// Whether an option is shown in the mode the "Advance" switch is in.
// Developer options are the reference's developer mode's, which is a
// preference this app has not got.
const inMode = (option) => option.mode === 'simple' || (settingsMode.advanced && option.mode === 'advanced');

function showSettingsPage(book) {
  const data = book.data;
  const answer = book.answer;
  if (!data || !answer) return;
  const prefix = book.prefix;
  // A page with nothing to show in this mode is not among the tabs, as
  // the reference leaves it out.
  const all = bookPages(book);
  const offered = all.filter((each) => each.groups.some((group) => group.lines.some((line) => line.keys.some((key) => inMode(data.options[key])))));
  const page = offered.find((each) => each.title === book.page) || offered[0] || all[0];
  book.page = page.title;
  if (!book.built.has(page.title)) book.built.set(page.title, buildSettingsPage(book, page));
  const built = book.built.get(page.title);
  let shown = 0;
  for (const group of built.groups) {
    let any = false;
    for (const line of group.lines) {
      let lineShown = false;
      for (const row of line.rows) {
        const visible = inMode(row.option) && !answer.hidden.has(row.key);
        row.node.hidden = !visible;
        if (visible) { showSettingsRow(book, row.key); lineShown = true; }
      }
      // The rules may name a line their own way ("Cool Steel Plate").
      if (line.title) line.title.textContent = (answer.labels && answer.labels[line.first] && t(answer.labels[line.first])) || line.label;
      line.node.hidden = !lineShown;
      any = any || lineShown;
      if (lineShown) shown += 1;
    }
    group.head.hidden = !any;
    group.body.hidden = !any;
  }
  $(`${prefix}-none`).hidden = shown !== 0;
  // Put in only when it is another page's turn: taking a page out and in
  // again would take the keys from the field they are in.
  if (built.node.parentNode !== $(`${prefix}-page`)) $(`${prefix}-page`).replaceChildren(built.node);
  const tabId = (title) => `${prefix}-tab-${title.replace(/\s+/g, '-')}`;
  $(`${prefix}-page`).setAttribute('aria-labelledby', tabId(page.title));
  // The tabs, each with how many of its options are set by hand.
  $(`${prefix}-tabs`).replaceChildren(...offered.map((each) => {
    const count = answer.counts[each.title] || 0;
    const tab = el('button', { type: 'button', id: tabId(each.title), className: 'process-tab', textContent: pageTitle(book, each.title) });
    tab.dataset.page = each.title;
    tab.setAttribute('role', 'tab');
    tab.setAttribute('aria-selected', String(each.title === page.title));
    tab.tabIndex = each.title === page.title ? 0 : -1;
    if (count) {
      tab.append(el('span', { className: 'process-count', textContent: String(count) }));
      tab.title = count === 1 ? '1 setting changed' : `${count} settings changed`;
    }
    tab.addEventListener('click', () => openSettingsPage(book, each.title));
    return tab;
  }));
  $(`${prefix}-reset`).hidden = answer.changed === 0;
  const refused = $(`${prefix}-refused`);
  refused.hidden = !answer.refused;
  refused.textContent = answer.refused ? `The engine will not slice with these settings: ${answer.refused}` : '';
}

function openSettingsPage(book, title, { focus = false } = {}) {
  book.page = title;
  if (state.project) kept(state.project.id)[`${book.kind}Page`] = title;
  showSettingsPage(book);
  if (focus) {
    const tab = [...$(`${book.prefix}-tabs`).children].find((each) => each.dataset.page === title);
    if (tab) tab.focus();
  }
}

// ---- what holds for the project shown ----

// Answers are numbered as they are asked for; one that comes back after
// a later one has been taken is out of date and is let go.
function takeAnswer(book, answer, asked) {
  if (asked < book.taken) return false;
  book.taken = asked;
  for (const list of ['hidden', 'off', 'unread']) answer[list] = new Set(answer[list]);
  book.answer = answer;
  return true;
}

const askFor = (book) => (book.kind === 'process' ? {} : { kind: book.kind, ...(book.kind === 'filament' ? { filament: book.filament } : {}) });

// Asks what holds for the project shown and shows the page it was on.
async function showSettings(book) {
  const project = state.project;
  const asked = ++book.asked;
  const [, answer] = await Promise.all([loadPages(book), call(book.kind === 'process' ? 'process' : 'settings', { project: project.id, ...askFor(book) })]);
  if (project.id !== activeId() || !takeAnswer(book, answer, asked)) return;
  book.page = kept(project.id)[`${book.kind}Page`] || book.page;
  // Another preset under what was set by hand: said as the reference says
  // it when a change of printer carries the changed settings over
  // (Tab.cpp `apply_print_settings_carry`).
  const whose = `${project.id}:${book.filament}`;
  const before = book.seen.get(whose);
  if (before !== undefined && before !== answer.name && before && answer.changed > 0) {
    const what = book.kind === 'filament' ? `filament ${book.filament}` : book.kind;
    say(answer.changed === 1
      ? `Kept 1 ${what} setting from "${before}" (revertable).`
      : `Kept ${answer.changed} ${what} settings from "${before}" (revertable).`);
  }
  book.seen.set(whose, answer.name);
  showSettingsPage(book);
  showSettingsSearch(book);
}

// The sidebar's process pages, with the rest of the sidebar; and the
// panel, when one is open, with them.
async function showProcess() {
  await showSettings(processPages);
  if (presetPanel.book) await showPresetPanel();
}

// Sets an option over the preset, gives it back (value null), or gives
// all of them back (`reset`); then shows what came of it. The slices made
// before go, as with anything else the project is sliced with.
async function setSettings(book, args) {
  const project = state.project;
  const asked = ++book.asked;
  const answer = book.kind === 'process'
    ? await call('process_set', { project: project.id, ...args })
    : await call('settings_set', { project: project.id, ...askFor(book), ...args });
  if (project.id !== activeId()) return;
  if (takeAnswer(book, answer, asked)) {
    showSettingsPage(book);
    showSettingsSearch(book);
    if (book !== processPages) showPresetHead();
  }
  state.project = await call('project', { project: project.id });
  // The lists' "* " follows what is now set over each preset.
  Presets.refreshMarks();
  await showStage();
  await refreshStrip();
}

// ---- search ----

// Every word typed must be in an option's label, its group, its page or
// its name, capitals not counting.
function searchSettings(book, text) {
  const words = text.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length || !book.data) return [];
  const found = [];
  for (const page of bookPages(book)) {
    for (const group of page.groups) {
      for (const line of group.lines) {
        for (const key of line.keys) {
          const option = book.data.options[key];
          if (!inMode(option)) continue;
          const hay = `${option.label} ${line.label || ''} ${key} ${key.replace(/_/g, ' ')} ${group.title} ${pageTitle(book, page.title)}`.toLowerCase();
          if (words.every((word) => hay.includes(word))) found.push({ key: nameIn(page, key), option, page: page.title, group: group.title, line: line.label });
        }
      }
    }
  }
  return found;
}

function showSettingsSearch(book) {
  const prefix = book.prefix;
  const box = $(`${prefix}-found`);
  const field = $(`${prefix}-search`);
  const text = field.value;
  const searching = text.trim() !== '';
  $(`${prefix}-tabs`).hidden = searching;
  $(`${prefix}-page`).hidden = searching;
  box.hidden = !searching;
  field.setAttribute('aria-expanded', String(searching));
  if (!searching) { book.found = []; return; }
  const found = searchSettings(book, text);
  book.found = found;
  book.at = Math.min(book.at, Math.max(0, found.length - 1));
  const answer = book.answer;
  box.replaceChildren(...(found.length ? found.map((hit, index) => {
    const hidden = answer && answer.hidden.has(hit.key);
    const row = el('button', { type: 'button', className: 'found-row', id: `${prefix}-found-${index}` },
      el('span', { className: 'found-label', textContent: hit.line ? `${hit.line} ${hit.option.label}` : hit.option.label }),
      el('span', { className: 'found-where', textContent: `${pageTitle(book, hit.page)} › ${hit.group}` }),
      el('span', { className: 'found-key', textContent: baseKey(hit.key) }));
    row.setAttribute('role', 'option');
    row.setAttribute('aria-selected', String(index === book.at));
    if (hidden) {
      row.classList.add('away');
      row.title = 'Not shown with the settings as they are: it depends on another setting.';
    }
    row.addEventListener('click', () => jumpToSetting(book, hit));
    return row;
  }) : [el('p', { className: 'quiet', textContent: 'No setting matches.' })]));
  field.setAttribute('aria-activedescendant', found.length ? `${prefix}-found-${book.at}` : '');
}

// Opens the page an option is on and goes to its row.
function jumpToSetting(book, hit) {
  $(`${book.prefix}-search`).value = '';
  showSettingsSearch(book);
  openSettingsPage(book, hit.page);
  const row = book.rows.get(hit.key);
  if (!row) return;
  // An option the rules hide has no row to show: its group is gone to.
  const target = row.node.hidden ? row.node.closest('.opt-group') : row.node;
  const shownTarget = target && !target.hidden ? target : $(`${book.prefix}-page`);
  shownTarget.scrollIntoView({ block: 'center' });
  if (row.node.hidden) return;
  row.node.classList.remove('found');
  // Read again so the mark starts anew when the same row is gone to twice.
  void row.node.offsetWidth;
  row.node.classList.add('found');
  setTimeout(() => row.node.classList.remove('found'), 1600);
  const face = row.node.querySelector('.lb-trigger') || row.control.input;
  if (face && !row.control.input.disabled) face.focus({ preventScroll: true });
}

// ---- the panel: a filament's or the printer's pages ----

// The reference opens these as tabs of their own ("Click to edit
// preset"). Here they take the sidebar's place, the plate staying in
// sight to be sliced, with a way back.
async function openPresetPanel(kind, filament = 1) {
  const book = kind === 'filament' ? filamentPages : printerPages;
  if (book !== presetPanel.book) $('preset-search').value = '';
  book.filament = filament;
  // What opened the panel, not a swatch in it that moved to another
  // filament: the keys go back there when it is closed.
  if (!presetPanel.book) presetPanel.opener = document.activeElement;
  presetPanel.book = book;
  $('side').classList.add('presets-open');
  $('plater').classList.add('presets-open');
  $('preset-panel').hidden = false;
  await showPresetPanel();
  $('preset-title').focus();
}

function closePresetPanel() {
  if (!presetPanel.book) return;
  presetPanel.book = null;
  $('preset-panel').hidden = true;
  $('side').classList.remove('presets-open');
  $('plater').classList.remove('presets-open');
  const opener = presetPanel.opener;
  if (opener && opener.isConnected) opener.focus();
}

// The panel's head: whose settings, over which preset, and for a
// filament the others to go to.
function showPresetHead() {
  const book = presetPanel.book;
  const answer = book && book.answer;
  if (!answer) return;
  const filament = book.kind === 'filament';
  $('preset-title').textContent = filament ? fmt(t('Filament %d'), book.filament) : t('Printer');
  const name = answer.name || (filament ? 'The project names no filament preset.' : 'The project names no printer.');
  $('preset-name').textContent = answer.source === 'project' ? `${name} (the project's own settings)` : name;
  $('preset-name').title = $('preset-name').textContent;
  const colours = (state.project && state.project.colours) || [];
  const count = filament ? answer.filaments : 0;
  $('preset-filaments').hidden = count < 2;
  $('preset-filaments').replaceChildren(...Array.from({ length: count }, (_, index) => {
    const number = index + 1;
    const swatch = el('button', { type: 'button', className: 'swatch', textContent: String(number), title: fmt(t('Filament %d'), number) });
    swatch.setAttribute('aria-pressed', String(number === book.filament));
    swatch.setAttribute('aria-label', fmt(t('Filament %d'), number));
    const colour = /^#[0-9a-f]{6}/i.test(colours[index] || '') ? colours[index].slice(0, 7) : null;
    if (colour) {
      swatch.style.background = colour;
      swatch.style.color = brightness(colour) > 0.55 ? '#1c2026' : '#ffffff';
    }
    swatch.addEventListener('click', () => attempt(() => openPresetPanel('filament', number)));
    return swatch;
  }));
}

// What holds for the panel's book, asked anew. A filament the project
// no longer has closes it.
async function showPresetPanel() {
  const book = presetPanel.book;
  if (!book || !state.project) return;
  if (book.kind === 'filament' && book.filament > state.project.filament_count) {
    closePresetPanel();
    return;
  }
  await showSettings(book);
  showPresetHead();
}

// The panel's cog: the preset its pages are of, saved, renamed, deleted
// or compared (presets.js).
document.addEventListener('scripts-loaded', () => {
  const cog = Presets.cogButton(() => ({
    kind: presetPanel.book ? presetPanel.book.kind : 'printer',
    filament: presetPanel.book ? presetPanel.book.filament : 1,
  }));
  cog.id = 'preset-cog';
  $('preset-reset').before(cog);
});

// ---- wiring, once the page stands ----

function wireSettings(book) {
  const prefix = book.prefix;
  const search = $(`${prefix}-search`);
  const current = () => (prefix === 'preset' ? presetPanel.book : book);
  search.addEventListener('input', () => { const each = current(); if (!each) return; each.at = 0; showSettingsSearch(each); });
  search.addEventListener('keydown', (event) => {
    const each = current();
    if (!each) return;
    const found = each.found;
    if (event.key === 'Escape' && search.value) {
      search.value = '';
      showSettingsSearch(each);
      event.preventDefault();
      event.stopPropagation();
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      if (!found.length) return;
      each.at = (each.at + (event.key === 'ArrowDown' ? 1 : found.length - 1)) % found.length;
      showSettingsSearch(each);
      const row = $(`${prefix}-found-${each.at}`);
      if (row) row.scrollIntoView({ block: 'nearest' });
      event.preventDefault();
    } else if (event.key === 'Enter' && found.length) {
      jumpToSetting(each, found[each.at]);
      event.preventDefault();
    }
  });
  // The tabs take the arrows, as a row of tabs does.
  $(`${prefix}-tabs`).addEventListener('keydown', (event) => {
    const each = current();
    if (!each) return;
    const pages = [...$(`${prefix}-tabs`).children].map((tab) => tab.dataset.page);
    const at = pages.indexOf(each.page);
    let to = -1;
    if (event.key === 'ArrowRight') to = (at + 1) % pages.length;
    else if (event.key === 'ArrowLeft') to = (at + pages.length - 1) % pages.length;
    else if (event.key === 'Home') to = 0;
    else if (event.key === 'End') to = pages.length - 1;
    if (to < 0 || !pages.length) return;
    event.preventDefault();
    openSettingsPage(each, pages[to], { focus: true });
  });
  $(`${prefix}-reset`).addEventListener('click', () => attempt(async () => {
    const each = current();
    if (!each) return;
    await setSettings(each, { reset: true });
    say('');
  }));
}

// Once every script of the page has run (i18n.js loads them).
document.addEventListener('scripts-loaded', () => {
  for (const id of ['process-advanced', 'preset-advanced']) {
    $(id).addEventListener('change', () => {
      settingsMode.advanced = $(id).checked;
      for (const other of ['process-advanced', 'preset-advanced']) $(other).checked = settingsMode.advanced;
      remembered.set('process.advanced', settingsMode.advanced ? '1' : '0');
      for (const book of [processPages, presetPanel.book]) {
        if (!book) continue;
        showSettingsPage(book);
        showSettingsSearch(book);
      }
    });
  }
  wireSettings(processPages);
  wireSettings(filamentPages);
  $('preset-back').addEventListener('click', closePresetPanel);
  $('preset-panel').addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !event.defaultPrevented && !event.target.closest('.lb, textarea')) {
      event.preventDefault();
      closePresetPanel();
    }
  });
  $('printer-edit').addEventListener('click', () => attempt(() => openPresetPanel('printer')));
});
