'use strict';

// The sidebar's process settings: the reference's pages "Quality",
// "Strength", "Speed", "Support", "Multimaterial" and "Others"
// (Tab.cpp `TabPrint::build`), drawn from data. Nothing here names an
// option: the pages, the groups, every option's label, tooltip, kind,
// unit, limits and list come from `process_pages` (made from the
// reference's source by tools/make_process_pages.py), and what holds for
// the project shown comes from `process`: each option's value by the
// preset, what is set over it, which options the reference's rules hide
// or grey out, and what the engine does with each.
//
// Loaded before app.js, whose `call`, `$`, `el`, `state`, `listbox`,
// `say` and the rest are used once the page runs.

const processPages = {
  data: null,          // { pages, options }, asked for once
  where: new Map(),    // key -> { page, group }
  built: new Map(),    // page title -> { node, groups: [{ node, lines: [{ node, rows }] }] }
  rows: new Map(),     // key -> a row of a built page
  answer: null,        // what `process` last said of the project shown
  page: null,          // the page open
  advanced: false,     // the "Advance" switch
  seen: new Map(),     // project id -> the preset its settings were last shown over
  asked: 0,            // answers asked for, counted
  taken: 0,            // the number of the one shown
  found: [],           // what the search lists
  at: 0,               // the row of that list the keys are on
};

// What the marks beside an option say.
const NOT_FOLLOWED = 'Not followed: with the settings as they are the engine does not read this setting, so changing it changes nothing in the print. It is not followed yet, or is read only when another setting turns it on.';
const WITH_A_NOTE = 'Followed with a note';
// The reference's own words (Tab.cpp `m_tt_value_revert`, Field.cpp `get_formatted_tooltip_text`).
const REVERT = 'Click to drop current modify and reset to saved value.';
const PARAMETER = 'parameter name';

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

async function loadProcessPages() {
  if (processPages.data) return processPages.data;
  const data = await call('process_pages');
  for (const page of data.pages) {
    for (const group of page.groups) {
      for (const line of group.lines) {
        for (const key of line.keys) processPages.where.set(key, { page: page.title, group: group.title });
      }
    }
  }
  processPages.advanced = remembered.get('process.advanced') === '1';
  $('process-advanced').checked = processPages.advanced;
  processPages.data = data;
  return data;
}

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
    if (option.min !== undefined && most !== undefined) return `${label}: Value ${shown} is out of range. The valid range is from ${option.min} to ${most}.`;
    return option.min !== undefined
      ? `${label}: Value ${shown} is out of range. It cannot be less than ${option.min}.`
      : `${label}: Value ${shown} is out of range. It cannot be more than ${most}.`;
  };
  const number = (written) => (/^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i.test(written.trim()) ? Number(written) : NaN);
  switch (option.type) {
    case 'int': {
      const value = number(text);
      if (!Number.isFinite(value)) return { error: `${label}: Invalid numeric.` };
      if (!Number.isInteger(value)) return { error: `${label}: Invalid numeric. It is a whole number.` };
      return { value: String(value), error: range(value, text, option.max) };
    }
    case 'float': {
      if (text.endsWith('%')) return { error: `${label} can't be a percentage` };
      const value = number(text);
      if (!Number.isFinite(value)) return { error: `${label}: Invalid numeric.` };
      return { value: String(value), error: range(value, text, option.max) };
    }
    case 'percent': {
      const value = number(text.replace(/%$/, ''));
      if (!Number.isFinite(value)) return { error: `${label}: Invalid numeric.` };
      return { value: `${value}%`, error: range(value, `${value}%`, option.max) };
    }
    case 'float_or_percent': {
      const percent = text.endsWith('%');
      const value = number(text.replace(/%$/, ''));
      if (!Number.isFinite(value)) return { error: `${label}: Invalid numeric.` };
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
  if (option.type !== 'float_or_percent') return unit === '%' && option.type === 'percent' ? '%' : unit;
  return String(value).trim().endsWith('%') ? '' : unit.replace(/ or %$/, '');
}

// ---- one option's row ----

// The control for an option, by its kind: a tick box, a number with its
// unit, a list (the page's own), a line of text, or a box of text.
// `commit(value)` sets the option (null gives it back to the preset).
function processControl(key, option, id, commit) {
  const field = el('span', { className: 'opt-field' });
  let read;
  let show;
  let input;
  if (option.type === 'bool') {
    input = el('input', { type: 'checkbox', id });
    input.addEventListener('change', () => commit(input.checked ? '1' : '0'));
    field.append(input);
    show = (value) => { input.checked = value === '1'; };
  } else if (option.type === 'enum') {
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
      if (input.options.length !== values.length) fill(input, values, value, labels);
      input.value = value;
      // The list's closed face follows what is done to the <select>'s
      // children and attributes; a value alone it would not see.
      input.dataset.shown = value;
    };
  } else if (option.multiline || option.full_width || option.type === 'strings') {
    input = el('textarea', { id, spellcheck: false, rows: Math.max(2, Math.min(8, Math.round((option.height || 5) / 3))) });
    if (option.code || option.type === 'strings') input.classList.add('code');
    input.addEventListener('change', () => commit(input.value === '' ? null : input.value));
    field.append(input);
    field.classList.add('wide');
    show = (value, preset) => { input.value = value; input.placeholder = preset; };
  } else {
    const numeric = option.type === 'int' || option.type === 'float';
    const texty = option.type === 'string';
    input = el('input', { id, type: numeric ? 'number' : 'text', autocomplete: 'off', spellcheck: false });
    if (numeric) input.step = 'any';
    if (!numeric && !texty) input.inputMode = 'decimal';
    const unit = el('span', { className: 'opt-unit' });
    const frame = texty ? input : el('span', { className: 'unit' }, input, unit);
    if (!texty && !numeric) input.classList.add('number');
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
      frame.append(more);
    }
    show = (value, preset) => {
      input.value = bare(value);
      input.placeholder = bare(preset);
      unit.textContent = unitFor(option, value);
    };
  }
  return { field, input, show };
}

function processRow(key, option) {
  const id = `po-${key}`;
  const row = el('div', { className: 'opt' });
  row.dataset.key = key;
  const label = el('label', { className: 'opt-label', htmlFor: id, textContent: option.label });
  const follow = el('span', { className: 'opt-follow', hidden: true });
  follow.setAttribute('role', 'img');
  const reset = el('button', { type: 'button', className: 'icon opt-reset', title: REVERT, ariaLabel: `${option.label}: reset to the preset's value` }, svg(REVERT_ICON));
  const commit = (value, error) => attempt(async () => {
    if (error) {
      // Refused here, with the reason: nothing is sent, and the field
      // goes back to what holds.
      showProcessRow(key, { force: true });
      throw new Error(error);
    }
    try {
      await setProcess({ key, value });
      say('');
    } catch (refused) {
      showProcessRow(key, { force: true });
      throw refused;
    }
  });
  reset.addEventListener('click', () => commit(null));
  const control = processControl(key, option, id, commit);
  row.append(el('span', { className: 'opt-head' }, label, follow), control.field, reset);
  if (control.field.classList.contains('wide')) row.classList.add('wide');
  // The list is the page's own; made once the <select> stands in its row with its label.
  if (option.type === 'enum') {
    const face = listbox(control.input).querySelector('.lb-trigger');
    label.addEventListener('click', (event) => { event.preventDefault(); face.focus(); });
  }
  return { key, option, node: row, label, follow, reset, control };
}

// Puts what holds for one option on its row: its value, whether it is
// set by hand, hidden or greyed out, and what the engine does with it.
// A field that is being typed in is left as it is, unless `force`d back
// to what holds (a value that was refused).
function showProcessRow(key, { force = false } = {}) {
  const row = processPages.rows.get(key);
  const answer = processPages.answer;
  if (!row || !answer) return;
  const option = row.option;
  const preset = answer.values[key] === undefined ? (option.default || '') : answer.values[key];
  const own = Object.prototype.hasOwnProperty.call(answer.set, key);
  const value = own ? answer.set[key] : preset;
  const input = row.control.input;
  const typing = document.activeElement === input && input.type !== 'checkbox' && input.value !== row.shown;
  if (force || !typing) {
    row.control.show(value, preset);
    row.shown = input.value;
  }
  row.node.classList.toggle('set', own);
  row.reset.hidden = !own;
  const off = answer.off.has(key) || Boolean(option.readonly);
  row.node.classList.toggle('off', off);
  row.control.input.disabled = off;
  const more = row.node.querySelector('.opt-more');
  if (more) more.disabled = off;
  // What the engine does with it. Nothing is said of one the rules have
  // greyed out: it is off, whoever reads it.
  const notes = answer.notes[key];
  const unread = !off && answer.unread.has(key);
  row.node.classList.toggle('unread', unread);
  row.node.classList.toggle('noted', Boolean(notes));
  const said = notes ? `${WITH_A_NOTE}: ${notes.join('\n')}` : (unread ? NOT_FOLLOWED : '');
  row.follow.hidden = !said;
  row.follow.title = said;
  row.follow.setAttribute('aria-label', said);
  const tip = [said, option.tooltip, `${PARAMETER}: ${key}`].filter(Boolean).join('\n\n');
  row.label.title = tip;
}

// ---- a page ----

// A page's groups and rows, built when the page is first opened.
function buildProcessPage(page) {
  const options = processPages.data.options;
  const node = el('div', { className: 'process-page' });
  const groups = page.groups.map((group) => {
    const head = el('h3', { textContent: group.title });
    const body = el('div', { className: 'opt-group' });
    const lines = group.lines.map((line) => {
      const rows = line.keys.map((key) => {
        const row = processRow(key, options[key]);
        processPages.rows.set(key, row);
        return row;
      });
      // Several options under one label: the label, then a row each.
      let box = rows[0].node;
      if (line.label) {
        box = el('div', { className: 'opt-line' },
          el('div', { className: 'opt-line-label', textContent: line.label, title: line.tooltip || '' }),
          ...rows.map((row) => row.node));
      }
      body.append(box);
      return { node: box, rows };
    });
    node.append(head, body);
    return { head, body, lines };
  });
  return { node, groups };
}

// Whether an option is shown in the mode the "Advance" switch is in.
// Developer options are the reference's developer mode's, which is a
// preference this app has not got.
const inMode = (option) => option.mode === 'simple' || (processPages.advanced && option.mode === 'advanced');

function showProcessPage() {
  const data = processPages.data;
  const answer = processPages.answer;
  if (!data || !answer) return;
  // A page with nothing to show in this mode is not among the tabs, as
  // the reference leaves it out.
  const offered = data.pages.filter((each) => each.groups.some((group) => group.lines.some((line) => line.keys.some((key) => inMode(data.options[key])))));
  const page = offered.find((each) => each.title === processPages.page) || offered[0] || data.pages[0];
  processPages.page = page.title;
  if (!processPages.built.has(page.title)) processPages.built.set(page.title, buildProcessPage(page));
  const built = processPages.built.get(page.title);
  let shown = 0;
  for (const group of built.groups) {
    let any = false;
    for (const line of group.lines) {
      let lineShown = false;
      for (const row of line.rows) {
        const visible = inMode(row.option) && !answer.hidden.has(row.key);
        row.node.hidden = !visible;
        if (visible) { showProcessRow(row.key); lineShown = true; }
      }
      line.node.hidden = !lineShown;
      any = any || lineShown;
      if (lineShown) shown += 1;
    }
    group.head.hidden = !any;
    group.body.hidden = !any;
  }
  $('process-none').hidden = shown !== 0;
  // Put in only when it is another page's turn: taking a page out and in
  // again would take the keys from the field they are in.
  if (built.node.parentNode !== $('process-page')) $('process-page').replaceChildren(built.node);
  $('process-page').setAttribute('aria-labelledby', `process-tab-${page.title}`);
  // The tabs, each with how many of its options are set by hand.
  $('process-tabs').replaceChildren(...offered.map((each) => {
    const count = answer.counts[each.title] || 0;
    const tab = el('button', { type: 'button', id: `process-tab-${each.title}`, className: 'process-tab', textContent: each.title });
    tab.dataset.page = each.title;
    tab.setAttribute('role', 'tab');
    tab.setAttribute('aria-selected', String(each.title === page.title));
    tab.tabIndex = each.title === page.title ? 0 : -1;
    if (count) {
      tab.append(el('span', { className: 'process-count', textContent: String(count) }));
      tab.title = count === 1 ? '1 setting changed' : `${count} settings changed`;
    }
    tab.addEventListener('click', () => openProcessPage(each.title));
    return tab;
  }));
  $('process-reset').hidden = answer.changed === 0;
  const refused = $('process-refused');
  refused.hidden = !answer.refused;
  refused.textContent = answer.refused ? `The engine will not slice with these settings: ${answer.refused}` : '';
}

function openProcessPage(title, { focus = false } = {}) {
  processPages.page = title;
  if (state.project) kept(state.project.id).processPage = title;
  showProcessPage();
  if (focus) {
    const tab = $(`process-tab-${title}`);
    if (tab) tab.focus();
  }
}

// ---- what holds for the project shown ----

// Answers are numbered as they are asked for; one that comes back after
// a later one has been taken is out of date and is let go.
function takeProcessAnswer(answer, asked) {
  if (asked < processPages.taken) return false;
  processPages.taken = asked;
  for (const list of ['hidden', 'off', 'unread']) answer[list] = new Set(answer[list]);
  processPages.answer = answer;
  return true;
}

// Asks what holds for the project shown and shows the page it was on.
// Called with the rest of the sidebar.
async function showProcess() {
  const project = state.project;
  const asked = ++processPages.asked;
  const [, answer] = await Promise.all([loadProcessPages(), call('process', { project: project.id })]);
  if (project.id !== activeId() || !takeProcessAnswer(answer, asked)) return;
  processPages.page = kept(project.id).processPage || processPages.page;
  // Another preset under what was set by hand: said as the reference says
  // it when a change of printer carries the changed settings over
  // (Tab.cpp `apply_print_settings_carry`).
  const before = processPages.seen.get(project.id);
  if (before !== undefined && before !== answer.name && before && answer.changed > 0) {
    say(answer.changed === 1
      ? `Kept 1 process setting from "${before}" (revertable).`
      : `Kept ${answer.changed} process settings from "${before}" (revertable).`);
  }
  processPages.seen.set(project.id, answer.name);
  showProcessPage();
  showProcessSearch();
}

// Sets an option over the preset, gives it back (value null), or gives
// all of them back (`reset`); then shows what came of it. The slices made
// before go, as with anything else the project is sliced with.
async function setProcess(args) {
  const project = state.project;
  const asked = ++processPages.asked;
  const answer = await call('process_set', { project: project.id, ...args });
  if (project.id !== activeId()) return;
  if (takeProcessAnswer(answer, asked)) {
    showProcessPage();
    showProcessSearch();
  }
  state.project = await call('project', { project: project.id });
  await showStage();
  await refreshStrip();
}

// ---- search ----

// Every word typed must be in an option's label, its group, its page or
// its name, capitals not counting.
function searchProcess(text) {
  const words = text.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length || !processPages.data) return [];
  const found = [];
  for (const page of processPages.data.pages) {
    for (const group of page.groups) {
      for (const line of group.lines) {
        for (const key of line.keys) {
          const option = processPages.data.options[key];
          if (!inMode(option)) continue;
          const hay = `${option.label} ${line.label || ''} ${key} ${key.replace(/_/g, ' ')} ${group.title} ${page.title}`.toLowerCase();
          if (words.every((word) => hay.includes(word))) found.push({ key, option, page: page.title, group: group.title, line: line.label });
        }
      }
    }
  }
  return found;
}

function showProcessSearch() {
  const box = $('process-found');
  const text = $('process-search').value;
  const searching = text.trim() !== '';
  $('process-tabs').hidden = searching;
  $('process-page').hidden = searching;
  box.hidden = !searching;
  $('process-search').setAttribute('aria-expanded', String(searching));
  if (!searching) { processPages.found = []; return; }
  const found = searchProcess(text);
  processPages.found = found;
  processPages.at = Math.min(processPages.at, Math.max(0, found.length - 1));
  const answer = processPages.answer;
  box.replaceChildren(...(found.length ? found.map((hit, index) => {
    const hidden = answer && answer.hidden.has(hit.key);
    const row = el('button', { type: 'button', className: 'found-row', id: `process-found-${index}` },
      el('span', { className: 'found-label', textContent: hit.line ? `${hit.line} ${hit.option.label}` : hit.option.label }),
      el('span', { className: 'found-where', textContent: `${hit.page} › ${hit.group}` }),
      el('span', { className: 'found-key', textContent: hit.key }));
    row.setAttribute('role', 'option');
    row.setAttribute('aria-selected', String(index === processPages.at));
    if (hidden) {
      row.classList.add('away');
      row.title = 'Not shown with the settings as they are: it depends on another setting.';
    }
    row.addEventListener('click', () => jumpToProcess(hit));
    return row;
  }) : [el('p', { className: 'quiet', textContent: 'No setting matches.' })]));
  $('process-search').setAttribute('aria-activedescendant', found.length ? `process-found-${processPages.at}` : '');
}

// Opens the page an option is on and goes to its row.
function jumpToProcess(hit) {
  $('process-search').value = '';
  showProcessSearch();
  openProcessPage(hit.page);
  const row = processPages.rows.get(hit.key);
  if (!row) return;
  // An option the rules hide has no row to show: its group is gone to.
  const target = row.node.hidden ? row.node.closest('.opt-group') : row.node;
  const shownTarget = target && !target.hidden ? target : $('process-page');
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

// ---- wiring, once the page stands ----

document.addEventListener('DOMContentLoaded', () => {
  $('process-advanced').addEventListener('change', () => {
    processPages.advanced = $('process-advanced').checked;
    remembered.set('process.advanced', processPages.advanced ? '1' : '0');
    showProcessPage();
    showProcessSearch();
  });
  $('process-reset').addEventListener('click', () => attempt(async () => {
    await setProcess({ reset: true });
    say('');
  }));
  const search = $('process-search');
  search.addEventListener('input', () => { processPages.at = 0; showProcessSearch(); });
  search.addEventListener('keydown', (event) => {
    const found = processPages.found;
    if (event.key === 'Escape' && search.value) {
      search.value = '';
      showProcessSearch();
      event.preventDefault();
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      if (!found.length) return;
      processPages.at = (processPages.at + (event.key === 'ArrowDown' ? 1 : found.length - 1)) % found.length;
      showProcessSearch();
      const row = $(`process-found-${processPages.at}`);
      if (row) row.scrollIntoView({ block: 'nearest' });
      event.preventDefault();
    } else if (event.key === 'Enter' && found.length) {
      jumpToProcess(found[processPages.at]);
      event.preventDefault();
    }
  });
  // The tabs take the arrows, as a row of tabs does.
  $('process-tabs').addEventListener('keydown', (event) => {
    const pages = [...$('process-tabs').children].map((tab) => tab.dataset.page);
    const at = pages.indexOf(processPages.page);
    let to = -1;
    if (event.key === 'ArrowRight') to = (at + 1) % pages.length;
    else if (event.key === 'ArrowLeft') to = (at + pages.length - 1) % pages.length;
    else if (event.key === 'Home') to = 0;
    else if (event.key === 'End') to = pages.length - 1;
    if (to < 0 || !pages.length) return;
    event.preventDefault();
    openProcessPage(pages[to], { focus: true });
  });
});
