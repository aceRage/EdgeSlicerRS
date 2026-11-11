'use strict';

// The page talks to the engine through `call`. In the app that is Tauri's
// own channel; served to a browser (`edge-desktop --serve`) it is a POST
// to the same machine, carrying the token this page was served with.
const tauri = window.__TAURI__;
const token = document.querySelector('meta[name="edge-token"]').content;
const $ = (id) => document.getElementById(id);

// Counts the calls that change which projects are open or which is shown,
// as they start and as they end: a listing asked for before one of them
// and answered after is out of date.
let changes = 0;
const changing = /^project_(new|open|select|cycle|close|move)$/;

async function call(name, args = {}) {
  if (!changing.test(name)) return send(name, args);
  changes += 1;
  try {
    return await send(name, args);
  } finally {
    changes += 1;
  }
}

async function send(name, args) {
  if (tauri) return tauri.core.invoke('call', { name, args });
  const response = await fetch(`/api/${name}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-Edge-Token': token },
    body: JSON.stringify(args),
  });
  if (!response.ok) throw new Error(await response.text());
  const answer = await response.json();
  if ('error' in answer) throw new Error(answer.error);
  return answer.ok;
}

// What is answered in bytes (a mesh, a stretch of a print's moves) comes
// as an ArrayBuffer, by the same two ways.
async function sendBytes(name, args = {}) {
  if (tauri) return tauri.core.invoke('call_bytes', { name, args });
  const response = await fetch(`/bytes/${name}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-Edge-Token': token },
    body: JSON.stringify(args),
  });
  if (!response.ok) throw new Error(await response.text());
  return response.arrayBuffer();
}

function el(tag, props = {}, ...children) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

// What was last chosen, kept for next time where the browser allows it.
const remembered = {
  get(key) {
    try { return localStorage.getItem(`edge.${key}`); } catch { return null; }
  },
  set(key, value) {
    try { localStorage.setItem(`edge.${key}`, value); } catch { /* not kept */ }
  },
};

function say(text, isError = false) {
  const status = $('status');
  status.textContent = text;
  status.className = isError ? 'error' : 'quiet';
}

// Runs something the person asked for; what goes wrong is said.
async function attempt(work) {
  try {
    return await work();
  } catch (error) {
    say(error.message, true);
    return undefined;
  }
}

// ---- dialogs and menus ----

// A file or folder from the person: the system's own dialog in the app,
// a typed path in a browser, which may not see the disk for itself.
const asking = {
  model: 'Path of an STL or 3MF file on this machine',
  folder: 'Path of the folder of vendor profiles',
  gcode: 'Save the G-code as (a full path)',
};

function modal(dialog) {
  return new Promise((resolve) => {
    dialog.returnValue = 'cancel';
    dialog.addEventListener('close', () => resolve(dialog.returnValue === 'ok'), { once: true });
    dialog.showModal();
  });
}

async function askText(label, suggested = '') {
  $('ask-label').textContent = label;
  $('ask-text').value = suggested;
  $('ask-text').type = 'text';
  const promise = modal($('ask'));
  $('ask-text').select();
  return (await promise) ? $('ask-text').value.trim() || null : null;
}

function pick(what, suggested = '') {
  if (tauri) return tauri.core.invoke('pick', { what, suggested: suggested || null });
  return askText(asking[what], suggested);
}

// A question with two answers; with `options`, one of them is picked too
// (its index comes back, or null).
async function confirmBox(title, text, ok, cancel, options = null) {
  $('confirm-title').textContent = title;
  $('confirm-text').textContent = text;
  $('confirm-ok').textContent = ok;
  $('confirm-cancel').textContent = cancel;
  $('confirm-list').replaceChildren(...(options || []).map((option, index) => el('li', {},
    el('label', {}, el('input', { type: 'radio', name: 'confirm-pick', value: index, checked: index === 0 }), option))));
  const yes = await modal($('confirm'));
  if (!options) return yes;
  const picked = $('confirm-list').querySelector('input:checked');
  return yes && picked ? Number(picked.value) : null;
}

// A menu at a point: items are [label, action, { checked, disabled }] or null for a line.
function showMenu(x, y, items) {
  const menu = $('menu');
  menu.replaceChildren(...items.map((item) => {
    if (!item) return el('hr');
    const [label, action, more = {}] = item;
    const button = el('button', { type: 'button', textContent: label, disabled: Boolean(more.disabled) });
    button.setAttribute('role', 'checked' in more ? 'menuitemradio' : 'menuitem');
    if ('checked' in more) button.setAttribute('aria-checked', String(more.checked));
    button.addEventListener('click', () => { hideMenu(); action(); });
    return button;
  }));
  menu.hidden = false;
  const box = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(4, Math.min(x, window.innerWidth - box.width - 4))}px`;
  menu.style.top = `${Math.max(4, Math.min(y, window.innerHeight - box.height - 4))}px`;
  const first = menu.querySelector('button:not(:disabled)');
  if (first) first.focus();
}
function hideMenu() { $('menu').hidden = true; }
document.addEventListener('pointerdown', (event) => {
  if (!$('menu').hidden && !$('menu').contains(event.target)) hideMenu();
});

// ---- controls ----

// The page's own list in place of a <select>'s system popup. The <select>
// stays in the page, out of sight, and stays what the page fills, reads
// and listens to: the list shows its options as they are (groups, lines,
// what is off or hidden), and picking one sets its value and sends its
// 'change', as picking in the system's popup does. `trigger` is an
// element of the page's to be the closed face (it holds a `.lb-value`
// for the name of what is selected); without one the list makes its own
// and comes back wrapped with it, to stand where the <select> stood.
// `decorate(row, option)` may add to a row of the open list.
//
// The keys are those of a list that is only picked from (ARIA's
// select-only combobox): closed, Down, Up, Enter, Space, Home and End
// open it; open, the arrows, Home, End, Page Up and Page Down move,
// Enter and Space pick, Escape and Tab close it and leave things as
// they were; a letter typed goes to the next row that starts with what
// was typed. The keys never leave the closed face, so there is nothing
// to give them back to.
let listboxes = 0;
function listbox(select, { trigger = null, decorate = null, bare = false } = {}) {
  const id = `lb${++listboxes}`;
  const face = trigger || el('div', { className: `lb-trigger${bare ? ' bare' : ''}` });
  let shown = face.querySelector('.lb-value');
  if (!shown) {
    shown = el('span', { className: 'lb-value' });
    face.append(shown);
  }
  let wrap = null;
  if (trigger) {
    select.classList.add('lb-native');
  } else {
    wrap = el('span', { className: 'lb' });
    if (select.parentNode) select.replaceWith(wrap);
    wrap.append(select, face);
  }
  face.setAttribute('role', 'combobox');
  face.setAttribute('aria-haspopup', 'listbox');
  face.setAttribute('aria-expanded', 'false');
  // Named as the <select> is: by its own label, else by the <label> it is in or that points at it.
  const labels = [...(select.labels || [])];
  const named = select.getAttribute('aria-label')
    || labels.map((label) => [...label.childNodes].filter((node) => node.nodeType === 3).map((node) => node.textContent).join(' ').trim()).join(' ');
  if (named) face.setAttribute('aria-label', named);
  for (const label of labels) {
    label.addEventListener('click', (event) => {
      if (face.contains(event.target)) return;
      event.preventDefault();
      face.focus();
    });
  }

  let pop = null;
  let rows = [];
  let active = -1;
  let typed = '';
  let typedAt = 0;

  const sync = () => {
    const option = select.selectedOptions[0] || null;
    shown.textContent = option ? option.label : '';
    face.classList.toggle('placeholder', !option || option.disabled);
    face.setAttribute('aria-disabled', String(select.disabled));
    face.tabIndex = select.disabled ? -1 : 0;
    if (select.title) {
      face.title = select.title;
    } else {
      face.removeAttribute('title');
      delete face.dataset.tip;
    }
  };

  const setActive = (index) => {
    if (active >= 0 && rows[active]) rows[active].node.classList.remove('active');
    active = index;
    if (index < 0 || !rows[index]) {
      face.removeAttribute('aria-activedescendant');
      return;
    }
    rows[index].node.classList.add('active');
    rows[index].node.scrollIntoView({ block: 'nearest' });
    face.setAttribute('aria-activedescendant', rows[index].node.id);
  };

  // The next row from `from` going `by` that can be picked; `from` itself when there is none.
  const step = (from, by) => {
    for (let at = from + by; at >= 0 && at < rows.length; at += by) {
      if (!rows[at].option.disabled) return at;
    }
    return from;
  };
  const edge = (last) => (last ? step(rows.length, -1) : step(-1, 1));

  const outside = (event) => {
    if (!pop || pop.contains(event.target) || face.contains(event.target)) return;
    hide();
  };
  const moved = (event) => { if (pop && !pop.contains(event.target)) hide(); };

  function hide() {
    if (!pop) return;
    pop.remove();
    pop = null;
    rows = [];
    active = -1;
    face.setAttribute('aria-expanded', 'false');
    face.removeAttribute('aria-controls');
    face.removeAttribute('aria-activedescendant');
    document.removeEventListener('pointerdown', outside, true);
    document.removeEventListener('scroll', moved, true);
    window.removeEventListener('resize', hide);
  }

  function show() {
    if (pop || select.disabled) return;
    pop = el('div', { className: 'lb-pop', id: `${id}-list` });
    pop.setAttribute('role', 'listbox');
    if (named) pop.setAttribute('aria-label', named);
    rows = [];
    const row = (option, into) => {
      if (option.hidden) return;
      const node = el('div', { className: 'lb-option', id: `${id}-${rows.length}` }, el('span', { className: 'lb-text', textContent: option.label }));
      node.setAttribute('role', 'option');
      node.setAttribute('aria-selected', String(option.selected));
      if (option.disabled) node.setAttribute('aria-disabled', 'true');
      if (option.title) node.title = option.title;
      if (decorate) decorate(node, option);
      const index = rows.length;
      node.addEventListener('pointermove', () => { if (active !== index && !option.disabled) setActive(index); });
      node.addEventListener('click', () => choose(index));
      rows.push({ node, option });
      into.append(node);
    };
    for (const child of select.children) {
      if (child.tagName === 'OPTGROUP') {
        const title = el('div', { className: 'lb-group-label', id: `${id}-g${pop.children.length}`, textContent: child.label });
        const group = el('div', { className: 'lb-group' }, title);
        group.setAttribute('role', 'group');
        group.setAttribute('aria-labelledby', title.id);
        for (const option of child.children) row(option, group);
        pop.append(group);
      } else if (child.tagName === 'HR') {
        const line = el('div', { className: 'lb-sep' });
        line.setAttribute('role', 'separator');
        pop.append(line);
      } else {
        row(child, pop);
      }
    }
    // A press in the list does not take the keys from the closed face.
    pop.addEventListener('pointerdown', (event) => event.preventDefault());
    // Inside the dialog it was opened from, where there is one: the rest of the page is out of reach while a dialog is up.
    (face.closest('dialog') || document.body).append(pop);
    // Under the closed face, or over it where there is more room; never wider or taller than the window.
    const box = face.getBoundingClientRect();
    const below = window.innerHeight - box.bottom - 12;
    const above = box.top - 12;
    const over = below < 200 && above > below;
    pop.style.minWidth = `${Math.min(box.width, window.innerWidth - 16)}px`;
    pop.style.maxHeight = `${Math.max(96, Math.min(440, over ? above : below))}px`;
    pop.style.left = `${Math.max(8, Math.min(box.left, window.innerWidth - pop.offsetWidth - 8))}px`;
    if (over) pop.style.bottom = `${window.innerHeight - box.top + 4}px`;
    else pop.style.top = `${box.bottom + 4}px`;
    face.setAttribute('aria-expanded', 'true');
    face.setAttribute('aria-controls', pop.id);
    const on = rows.findIndex((known) => known.option.selected && !known.option.disabled);
    setActive(on >= 0 ? on : edge(false));
    // What is selected, in the middle of a list that scrolls.
    if (on >= 0) rows[on].node.scrollIntoView({ block: 'center' });
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('scroll', moved, true);
    window.addEventListener('resize', hide);
  }

  function choose(index) {
    const picked = rows[index];
    if (!picked || picked.option.disabled) return;
    const value = picked.option.value;
    hide();
    if (select.value !== value) {
      select.value = value;
      select.dispatchEvent(new Event('change', { bubbles: true }));
    }
    sync();
  }

  face.addEventListener('click', () => { if (pop) hide(); else show(); });
  face.addEventListener('focusout', hide);
  // A name cut short is given whole under the pointer, unless the <select> has a tooltip of its own.
  face.addEventListener('pointerover', () => {
    if (select.title || pop || face.dataset.tip !== undefined) return;
    if (shown.scrollWidth > shown.clientWidth) face.title = shown.textContent;
    else face.removeAttribute('title');
  });
  face.addEventListener('keydown', (event) => {
    if (select.disabled || event.ctrlKey || event.metaKey) return;
    const key = event.key;
    const letter = key.length === 1 && (key !== ' ' || (typed && Date.now() - typedAt < 700));
    if (letter) {
      // What was typed in the last moment, to the next row that starts with it; one letter again goes on to the next.
      event.preventDefault();
      typed = Date.now() - typedAt < 700 ? typed + key.toLowerCase() : key.toLowerCase();
      typedAt = Date.now();
      show();
      const same = [...typed].every((char) => char === typed[0]);
      const wanted = same ? typed[0] : typed;
      const from = same ? active + 1 : Math.max(active, 0);
      for (let n = 0; n < rows.length; n += 1) {
        const at = (from + n) % rows.length;
        if (!rows[at].option.disabled && rows[at].option.label.toLowerCase().startsWith(wanted)) { setActive(at); break; }
      }
      return;
    }
    if (!pop) {
      if (!['ArrowDown', 'ArrowUp', 'Enter', ' ', 'Home', 'End'].includes(key)) return;
      event.preventDefault();
      show();
      if (key === 'Home') setActive(edge(false));
      if (key === 'End') setActive(edge(true));
      return;
    }
    if (key === 'Escape') {
      // The list closes; what it stands in (a dialog, the page) does not hear of it.
      event.preventDefault();
      event.stopPropagation();
      hide();
    } else if (key === 'Tab') {
      hide();
    } else if (key === 'Enter' || key === ' ' || (key === 'ArrowUp' && event.altKey)) {
      event.preventDefault();
      choose(active);
    } else if (key === 'ArrowDown' || key === 'ArrowUp') {
      event.preventDefault();
      setActive(step(active, key === 'ArrowDown' ? 1 : -1));
    } else if (key === 'Home' || key === 'End') {
      event.preventDefault();
      setActive(edge(key === 'End'));
    } else if (key === 'PageDown' || key === 'PageUp') {
      event.preventDefault();
      let at = active;
      for (let n = 0; n < 10; n += 1) at = step(at, key === 'PageDown' ? 1 : -1);
      setActive(at);
    }
  });

  // Whatever the page does to the <select> shows in the closed face; an open list is closed, its rows being of what was.
  new MutationObserver(() => { hide(); sync(); }).observe(select, { childList: true, subtree: true, attributes: true });
  sync();
  return wrap || face;
}

// Tooltips of the page's own: what an element's `title` says is shown
// in the page's look, under the pointer after a moment and beside what
// the keys are on. The title is put aside while it is shown, so the
// system's tooltip does not come up beside it, and given back after.
// Where the browser cannot lay a thing over an open dialog (no popover),
// titles are left to the system.
const tips = { node: null, from: null, timer: null, watch: null };

function hideTip() {
  clearTimeout(tips.timer);
  if (tips.watch) tips.watch.disconnect();
  if (tips.from && tips.from.dataset.tip !== undefined) {
    if (!tips.from.hasAttribute('title')) tips.from.setAttribute('title', tips.from.dataset.tip);
    delete tips.from.dataset.tip;
  }
  if (tips.node) tips.node.remove();
  Object.assign(tips, { node: null, from: null, timer: null, watch: null });
}

function showTip(from, wait) {
  if (tips.from === from) return;
  hideTip();
  if (!from.title) return;
  tips.from = from;
  const put = () => {
    if (from.hasAttribute('title')) {
      from.dataset.tip = from.title;
      from.removeAttribute('title');
    }
  };
  put();
  // A title set again while it is shown is put aside too, and shown.
  tips.watch = new MutationObserver(() => {
    if (!from.title) return;
    put();
    if (tips.node) tips.node.textContent = from.dataset.tip;
  });
  tips.watch.observe(from, { attributes: true, attributeFilter: ['title'] });
  tips.timer = setTimeout(() => {
    if (!from.isConnected || !from.dataset.tip) { hideTip(); return; }
    const node = el('div', { className: 'tip', textContent: from.dataset.tip });
    node.setAttribute('role', 'tooltip');
    node.popover = 'manual';
    document.body.append(node);
    node.showPopover();
    tips.node = node;
    // Under what it is about, in the middle; over it where there is no room under; inside the window.
    const box = from.getBoundingClientRect();
    const own = node.getBoundingClientRect();
    const left = Math.max(6, Math.min(box.left + box.width / 2 - own.width / 2, window.innerWidth - own.width - 6));
    const under = box.bottom + 6 + own.height <= window.innerHeight - 6;
    node.style.left = `${left}px`;
    node.style.top = `${under ? box.bottom + 6 : Math.max(6, box.top - own.height - 6)}px`;
  }, wait);
}

if ('popover' in HTMLElement.prototype) {
  const titled = (target) => (target instanceof Element ? target.closest('[title]:not([title=""]), [data-tip]') : null);
  document.addEventListener('pointerover', (event) => {
    const from = titled(event.target);
    if (from) showTip(from, 450);
    else if (tips.from && !tips.from.contains(event.target)) hideTip();
  });
  document.addEventListener('pointerout', (event) => {
    if (tips.from && !(event.relatedTarget instanceof Node && tips.from.contains(event.relatedTarget))) hideTip();
  });
  document.addEventListener('focusin', (event) => {
    const from = titled(event.target);
    if (from && event.target.matches(':focus-visible')) showTip(from, 450);
  });
  document.addEventListener('focusout', hideTip);
  document.addEventListener('pointerdown', hideTip, true);
  document.addEventListener('keydown', hideTip, true);
  document.addEventListener('scroll', hideTip, true);
}

// ---- state ----

const state = {
  // The window's tab: prepare, preview or device. Device is the window's
  // own; the other two are remembered by each project.
  tab: 'prepare',
  // The strip: { active, max_open, projects }.
  listing: { active: 0, max_open: 8, projects: [] },
  // The project shown, in full, and what it would be sliced with.
  project: null,
  settled: null,
  // The printers added and the one the project shown is on.
  printers: null,
  vendors: [],
  sliceAll: false,
  // What the page keeps for each open project, by id.
  kept: new Map(),
};

// A project's own corner of the page: what was fetched for it and where
// its views were left.
function kept(id) {
  if (!state.kept.has(id)) {
    state.kept.set(id, {
      plateViews: new Map(),   // "plate|printer" -> what stands on the plate
      summaries: new Map(),    // plate -> { run, summary }
      layers: new Map(),       // "run:plate:index" -> the layer's lines
      layerAt: {},             // plate -> the layer its Preview is on
      viewType: 'Line Type',
      hidden: new Set(),
      camera: { prepare: null, preview: null },
      restored: false,
      thumb: null,
      thumbKey: '',
      wasSlicing: false,
      shown: null,
      wanted: 0,
    });
  }
  return state.kept.get(id);
}

const activeId = () => state.listing.active;
const here = () => kept(activeId());
const plateKey = (project = state.project) => (project && project.plate !== null ? project.plate : 0);

// ---- presets ----

const presetsByMachine = new Map();

async function presetsOf(vendor, machine) {
  const key = `${vendor}|${machine}`;
  if (!presetsByMachine.has(key)) presetsByMachine.set(key, await call('presets', { vendor, machine }));
  return presetsByMachine.get(key);
}

// A project that has no printer yet is given one: the one last selected,
// else the first of those added, with what was last used with it. It is
// what the project starts with, not a change to it.
async function withPresets(project) {
  if (project.choices.source !== 'presets' || project.choices.machine || !state.vendors.length) return project;
  return call('printer_select', { project: project.id, initial: true });
}

function fill(select, names, value, labels = null) {
  select.replaceChildren(...names.map((name, index) => new Option(labels ? labels[index] : name, name)));
  select.disabled = names.length === 0;
  if (value !== null && value !== undefined && names.includes(value)) select.value = value;
}

// ---- the project shown ----

let showing = 0;

// Fetches the project shown and puts all of it on the page.
async function showProject() {
  const turn = ++showing;
  let project = await call('project');
  project = await withPresets(project);
  if (turn !== showing) return;
  state.project = project;
  const mine = kept(project.id);
  if (!mine.restored) {
    // Where its Preview was left, as the session kept it.
    const view = project.view || {};
    if (typeof view.type === 'string') mine.viewType = view.type;
    if (view.layers && typeof view.layers === 'object') mine.layerAt = { ...view.layers };
    if (Array.isArray(view.hidden)) mine.hidden = new Set(view.hidden);
    if (Array.isArray(view.shown)) mine.options = new Set(view.shown);
    mine.restored = true;
  }
  // On Prepare or Preview: the tab the project was on. On Device: stay.
  if (state.tab !== 'device') state.tab = project.tab;
  await showSidebar();
  if (turn !== showing) return;
  showTabs();
  await showStage();
}

// Sets something for the project shown and shows what came of it.
async function setProject(args, { stage = true } = {}) {
  const project = await call('project_set', { project: activeId(), ...args });
  if (project.id !== activeId()) return;
  state.project = project;
  if (stage) {
    await showSidebar();
    await showStage();
  }
  await refreshStrip();
}

let noteTimer = null;
function noteView() {
  // Where Preview is: kept with the project in the session, a moment
  // after the last change.
  clearTimeout(noteTimer);
  const id = activeId();
  const mine = kept(id);
  noteTimer = setTimeout(() => {
    const view = { type: mine.viewType, layers: mine.layerAt, hidden: [...mine.hidden] };
    if (mine.options) view.shown = [...mine.options];
    call('project_set', { project: id, view }).catch(() => {});
  }, 400);
}

// ---- sidebar ----

const plateLabels = (types) => [types.choices.map((choice) => choice.value), types.choices.map((choice) => choice.label)];

async function showSidebar() {
  const project = state.project;
  const choices = project.choices;
  const fromPresets = choices.source === 'presets';
  const [settled, printers] = await Promise.all([
    call('settled', { project: project.id }),
    call('printers', { project: project.id }),
  ]);
  if (project.id !== activeId()) return;
  state.settled = settled;
  state.printers = printers;
  const { printer } = state.settled;

  // Printer: the list, the picture and the nozzles (printers.js).
  showPrinterBlock();
  const bed = $('bed');
  if (printer) {
    const types = printer.bed_types;
    let [values, labels] = plateLabels(types);
    // A plate in force that is not among those offered is shown as it is named.
    if (!values.includes(types.in_force)) {
      values = [types.in_force, ...values];
      labels = [types.in_force, ...labels];
    }
    fill(bed, values, types.in_force, labels);
    bed.disabled = !types.enabled;
  } else {
    fill(bed, [], null);
  }
  const facts = [];
  if (printer) {
    if (printer.bed) {
      const size = [printer.bed[2] - printer.bed[0], printer.bed[3] - printer.bed[1]];
      if (printer.height) size.push(printer.height);
      facts.push(`${size.map((side) => Math.round(side)).join(' × ')} mm`);
    }
  }
  $('printer-info').textContent = facts.join(' · ');

  // Filaments: one for each the file prints in, by number.
  const count = project.filament_count;
  const offered = fromPresets && choices.machine ? (await presetsOf(choices.vendor, choices.machine)).filaments : [];
  const colours = project.colours || [];
  $('filaments').replaceChildren(...Array.from({ length: count }, (_, index) => {
    const swatch = el('span', { className: 'swatch', textContent: String(index + 1) });
    const colour = /^#[0-9a-f]{6}/i.test(colours[index] || '') ? colours[index].slice(0, 7) : null;
    if (colour) {
      swatch.style.background = colour;
      swatch.style.color = brightness(colour) > 0.55 ? '#1c2026' : '#ffffff';
    }
    let name;
    if (fromPresets) {
      const list = el('select', { ariaLabel: `Filament ${index + 1}` });
      list.dataset.filament = index + 1;
      // A filament past the last chosen prints with the last, as the engine has it.
      fill(list, offered, choices.filaments[Math.min(index, choices.filaments.length - 1)]);
      list.addEventListener('change', filamentChanged);
      name = listbox(list, { bare: true });
    } else {
      // Cut short where it does not fit, and whole in its tooltip.
      const saved = (project.saved.filaments || [])[index] || 'not named';
      name = el('span', { className: 'value', textContent: saved, title: saved });
    }
    return el('div', { className: 'filament' }, swatch, name);
  }));

  // Process.
  $('process-row').hidden = !fromPresets;
  $('saved-process').hidden = fromPresets;
  $('saved-process').textContent = project.saved.process || 'The project names no process.';
  if (fromPresets) {
    const processes = choices.machine ? (await presetsOf(choices.vendor, choices.machine)).processes : [];
    fill($('process'), processes, choices.process);
  }
  // The pages of process settings, and what is set over the preset (process.js).
  await showProcess();
  showDevice();
}

function brightness(colour) {
  const parts = [1, 3, 5].map((at) => parseInt(colour.slice(at, at + 2), 16));
  return (0.299 * parts[0] + 0.587 * parts[1] + 0.114 * parts[2]) / 255;
}

// What is chosen for a printer (filaments, process, bed type) is kept
// with that printer by the session, for the next time it is selected.
async function filamentChanged() {
  const filaments = [...$('filaments').querySelectorAll('select')].map((select) => select.value);
  await setProject({ filaments });
}

// The sidebar's lists are the page's own (listbox); the printer's is set up with its card, in printers.js.
listbox($('process'));
listbox($('bed'), { bare: true });
$('process').addEventListener('change', () => attempt(() => setProject({ process: $('process').value })));
$('bed').addEventListener('change', () => attempt(() => setProject({ bed_type: $('bed').value })));
for (const title of document.querySelectorAll('.fold-title')) {
  title.addEventListener('click', () => {
    const folded = title.closest('.fold').classList.toggle('folded');
    title.setAttribute('aria-expanded', String(!folded));
  });
}
$('profiles').addEventListener('click', () => attempt(async () => {
  const dir = await pick('folder', $('profiles-dir').textContent);
  if (!dir) return;
  await loadLibrary(await call('set_profiles', { dir }));
  await showProject();
  await firstRun();
}));

async function loadLibrary(library) {
  $('profiles-dir').textContent = library.dir || 'none';
  $('profiles-dir').title = library.dir || '';
  state.vendors = library.vendors;
  presetsByMachine.clear();
}

// ---- tabs ----

function showTabs() {
  for (const tab of document.querySelectorAll('#tabs [role="tab"]')) {
    const selected = tab.dataset.tab === state.tab;
    tab.setAttribute('aria-selected', String(selected));
    tab.tabIndex = selected ? 0 : -1;
  }
  $('plater').hidden = state.tab === 'device';
  $('device').hidden = state.tab !== 'device';
  showButtons();
}

// Prepare and Preview are two views of the project's plate; Device is the
// window's. Going to Preview slices the plate when it is not sliced yet,
// as the reference does.
async function selectTab(tab) {
  if (tab === state.tab) return;
  state.tab = tab;
  showTabs();
  if (tab === 'device') {
    showDevice();
    return;
  }
  const project = state.project;
  if (project && project.tab !== tab) {
    project.tab = tab;
    call('project_set', { project: project.id, tab }).then(refreshStrip, () => {});
  }
  await showStage({ entering: true });
}

for (const tab of document.querySelectorAll('#tabs [role="tab"]')) {
  tab.addEventListener('click', () => attempt(() => selectTab(tab.dataset.tab)));
}

// ---- slicing ----

const sliceable = (project) => {
  if (!project || project.empty) return [];
  if (!project.plates.length) return [0];
  return project.plates.filter((plate) => plate.filaments.length).map((plate) => plate.id);
};
const isSliced = (project, plate) => (project.plates.length
  ? project.plates.some((listed) => listed.id === plate && listed.sliced)
  : project.sliced);
const slicingNow = () => Boolean(state.project && state.project.status.state === 'slicing');

function showButtons() {
  const project = state.project;
  const onPlater = state.tab !== 'device';
  const plates = sliceable(project);
  const busy = slicingNow();
  let can = false;
  if (project && onPlater && !busy) {
    can = state.sliceAll
      ? plates.some((plate) => !isSliced(project, plate))
      // The plate shown, unless it is sliced already or holds nothing.
      : plates.includes(plateKey()) && !project.sliced;
  }
  $('slice').textContent = state.sliceAll ? 'Slice all' : 'Slice plate';
  $('slice').disabled = !can;
  const summary = project && here().summaries.get(plateKey());
  const failed = summary && 'error' in summary.summary.gcode;
  $('export').disabled = !(project && onPlater && project.sliced && !busy && !failed);
}

// Slices the plate shown, or every plate not sliced yet, and goes to
// Preview, where the reference's slice button always ends up.
async function slice(all = state.sliceAll) {
  const project = state.project;
  if (!project || project.empty) return;
  try {
    await call('slice', all ? { project: project.id, all: true } : { project: project.id, plate: project.plate ?? undefined });
  } catch (error) {
    say(error.message, true);
    return;
  }
  say('');
  $('exported').textContent = '';
  await refreshProject();
  if (state.tab !== 'preview') await selectTab('preview');
  else await showStage();
  schedule(0);
}

$('slice').addEventListener('click', () => attempt(() => slice()));
$('slice-more').addEventListener('click', (event) => {
  const box = event.currentTarget.getBoundingClientRect();
  const choose = (all) => () => { state.sliceAll = all; showButtons(); };
  showMenu(box.right - 180, box.bottom + 4, [
    ['Slice all', choose(true), { checked: state.sliceAll }],
    ['Slice plate', choose(false), { checked: !state.sliceAll }],
  ]);
});
$('cancel').addEventListener('click', () => call('cancel', { project: activeId() }).catch(() => {}));

$('export').addEventListener('click', () => attempt(async () => {
  const project = state.project;
  const summary = here().summaries.get(plateKey());
  const stem = (summary && summary.summary.stem) || project.name || 'print';
  const name = project.plates.length > 1 ? `${stem}_plate_${project.plate}` : stem;
  const path = await pick('gcode', `${name}.gcode`);
  if (!path) return;
  const exported = $('exported');
  try {
    const written = await call('export', { project: project.id, plate: project.plate ?? undefined, path });
    exported.textContent = `Saved ${written.path} (${number(written.bytes / 1e6, 2)} MB).`;
    exported.className = 'quiet';
  } catch (error) {
    exported.textContent = error.message;
    exported.className = 'error';
  }
  tidyNotes();
}));

// The project shown, fetched again without touching the sidebar.
async function refreshProject() {
  const project = await call('project');
  if (project.id === activeId()) state.project = project;
  showButtons();
  showPlates();
}

let settledNote = null;

function showProgress() {
  const project = state.project;
  const status = project ? project.status : null;
  const box = $('progress');
  // Another project's progress is not this one's news.
  if (project && box.dataset.project !== String(project.id)) {
    clearTimeout(settledNote);
    box.hidden = true;
    box.classList.remove('settled');
    box.dataset.project = String(project.id);
  }
  if (status && status.state === 'slicing') {
    clearTimeout(settledNote);
    const part = (status.at + status.step / status.steps) / Math.max(status.of, 1);
    $('bar').style.width = `${Math.round(100 * Math.min(part, 1))}%`;
    const plate = status.of > 1 ? `Plate ${status.plate} (${status.at + 1} of ${status.of}): ` : '';
    $('progress-label').textContent = `${plate}${status.label}…`;
    box.classList.remove('settled');
    box.hidden = false;
  } else if (!box.hidden && !box.classList.contains('settled')) {
    // What became of it, for a moment.
    const said = { done: 'Slice ok.', cancelled: 'Slicing Canceled', failed: 'Slicing failed.' }[status ? status.state : ''];
    if (!said) { box.hidden = true; return; }
    $('progress-label').textContent = said;
    box.classList.add('settled');
    settledNote = setTimeout(() => { box.hidden = true; box.classList.remove('settled'); }, 3000);
  }
}

// ---- the strip ----

const strip = {
  collapsed: remembered.get('strip.collapsed') === '1',
  text: remembered.get('strip.text') === '1',
  width: Math.max(88, Math.min(200, Number(remembered.get('strip.width')) || 88)),
};

// 88 to 200 as dragged; names alone need 120; folded, a 24 wide rail
// (ProjectSessions.hpp `StripLayout`).
function showStripShape() {
  const node = $('strip');
  node.classList.toggle('collapsed', strip.collapsed);
  node.classList.toggle('text', strip.text && !strip.collapsed);
  const width = strip.collapsed ? 24 : Math.max(strip.width, strip.text ? 120 : 88);
  node.style.setProperty('--strip-width', `${width}px`);
  const fold = $('strip-fold');
  fold.textContent = strip.collapsed ? '»' : '«';
  fold.title = strip.collapsed ? 'Show the project strip' : 'Collapse the project strip';
  fold.setAttribute('aria-label', fold.title);
}

function setStrip(change) {
  Object.assign(strip, change);
  remembered.set('strip.collapsed', strip.collapsed ? '1' : '0');
  remembered.set('strip.text', strip.text ? '1' : '0');
  remembered.set('strip.width', String(strip.width));
  showStripShape();
  renderStrip();
}

function tooltipFor(tile) {
  const lines = [tile.name];
  if (tile.path) lines.push(tile.path);
  if (tile.dirty) lines.push('Unsaved changes');
  if (tile.slicing) lines.push('Slicing');
  return lines.join('\n');
}

// A tile's name on at most two lines of `width`: broken only after a
// space, _, - or ., never inside a word; what does not fit a line is cut
// short there with an ellipsis (ProjectSessions.hpp `wrap_name_two_lines`).
const measuring = document.createElement('canvas').getContext('2d');
function nameLines(name, width) {
  measuring.font = '11px system-ui, -apple-system, "Segoe UI", sans-serif';
  const fits = (text) => measuring.measureText(text).width <= width;
  if (fits(name)) return [name];
  const tokens = name.split(/(?<=[ _\-.])/);
  let first = '';
  let at = 0;
  while (at < tokens.length && fits(first + tokens[at])) first += tokens[at++];
  if (!first) return [name];
  const rest = tokens.slice(at).join('').trim();
  return rest ? [first.trimEnd(), rest] : [first.trimEnd()];
}

let stripSeen = '';

function renderStrip(force = false) {
  const listing = state.listing;
  const seen = JSON.stringify([listing, strip, [...state.kept].map(([id, mine]) => [id, mine.thumbKey])]);
  if (!force && seen === stripSeen) return;
  stripSeen = seen;
  $('projects').replaceChildren(...listing.projects.map((tile) => {
    const active = tile.id === listing.active;
    const sliced = tile.plates_with_objects > 0 && tile.plates_sliced >= tile.plates_with_objects;
    const item = el('li', { className: `tile${active ? ' active' : ''}${sliced ? ' is-sliced' : ''}`, title: tooltipFor(tile) });
    item.dataset.id = tile.id;
    item.setAttribute('aria-current', String(active));
    const thumb = el('canvas', { className: 'thumb', width: 192, height: 192 });
    drawThumb(thumb, tile);
    const wide = Math.max(strip.width, strip.text ? 120 : 88) - 21;
    const name = el('span', { className: 'name' },
      ...(strip.text ? [tile.name] : nameLines(tile.name, wide)).map((line) => el('span', { textContent: line })));
    item.append(thumb, name);
    if (tile.dirty) item.append(el('span', { className: 'dot' }));
    // A spinner while it slices, a tick once every plate that holds
    // something is sliced.
    if (tile.slicing) item.append(el('span', { className: 'badge slicing' }));
    else if (sliced) item.append(el('span', { className: 'badge sliced', textContent: '✓' }));
    const close = el('button', { className: 'close', type: 'button', title: 'Close project', textContent: '×' });
    close.setAttribute('aria-label', `Close ${tile.name}`);
    item.append(close);
    return item;
  }));
  // The badge sits in the thumbnail's corner, whatever the strip's width.
  for (const badge of $('projects').querySelectorAll('.badge')) {
    const thumb = badge.parentElement.querySelector('.thumb');
    if (!strip.text && !strip.collapsed) badge.style.top = `${thumb.offsetTop + thumb.offsetHeight - 18}px`;
  }
}

// A tile's picture: the plate the project is on, seen from above. A
// project with no file shows an empty bed; one not looked at yet, a page.
function drawThumb(canvas, tile) {
  const context = canvas.getContext('2d');
  const side = canvas.width;
  const style = getComputedStyle(document.documentElement);
  const muted = style.getPropertyValue('--strip-muted').trim();
  const accent = style.getPropertyValue('--accent').trim();
  context.clearRect(0, 0, side, side);
  const view = kept(tile.id).thumb;
  if (!view && !tile.empty) {
    // No picture yet: a file with a folded corner.
    const [x, y, w, h, fold] = [side * 0.25, side * 0.16, side * 0.5, side * 0.68, side * 0.16];
    context.beginPath();
    context.moveTo(x, y); context.lineTo(x + w - fold, y); context.lineTo(x + w, y + fold);
    context.lineTo(x + w, y + h); context.lineTo(x, y + h); context.closePath();
    context.moveTo(x + w - fold, y); context.lineTo(x + w - fold, y + fold); context.lineTo(x + w, y + fold);
    context.strokeStyle = muted;
    context.lineWidth = 3;
    context.stroke();
    return;
  }
  const objects = view ? view.objects : [];
  let box = view && view.bed;
  if (!box) {
    box = objects.length
      ? objects.map((object) => object.box).reduce((a, b) => [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[2], b[2]), Math.max(a[3], b[3])])
      : [0, 0, 1, 1];
  }
  const scale = (side * 0.84) / Math.max(box[2] - box[0], box[3] - box[1], 1e-6);
  const at = (x, y) => [side / 2 + (x - (box[0] + box[2]) / 2) * scale, side / 2 - (y - (box[1] + box[3]) / 2) * scale];
  if ((view && view.bed) || !objects.length) {
    const [x0, y1] = at(box[0], box[1]);
    const [x1, y0] = at(box[2], box[3]);
    context.fillStyle = 'rgba(128, 128, 128, 0.16)';
    context.strokeStyle = muted;
    context.lineWidth = 2;
    context.fillRect(x0, y0, x1 - x0, y1 - y0);
    context.beginPath();
    for (const third of [1 / 3, 2 / 3]) {
      context.moveTo(x0 + (x1 - x0) * third, y0); context.lineTo(x0 + (x1 - x0) * third, y1);
      context.moveTo(x0, y0 + (y1 - y0) * third); context.lineTo(x1, y0 + (y1 - y0) * third);
    }
    context.globalAlpha = 0.4;
    context.stroke();
    context.globalAlpha = 1;
    context.strokeRect(x0, y0, x1 - x0, y1 - y0);
  }
  context.fillStyle = accent;
  context.strokeStyle = accent;
  context.lineWidth = 2;
  context.lineJoin = 'round';
  for (const { outline } of objects) {
    context.beginPath();
    for (let i = 0; i < outline.length; i += 2) {
      const [x, y] = at(outline[i], outline[i + 1]);
      if (i) context.lineTo(x, y); else context.moveTo(x, y);
    }
    context.closePath();
    context.globalAlpha = 0.55;
    context.fill();
    context.globalAlpha = 1;
    context.stroke();
  }
}

// The picture of a project's tile, fetched when its plate or printer changed.
async function refreshThumbs() {
  for (const tile of state.listing.projects) {
    const mine = kept(tile.id);
    const key = `${tile.plate}|${tile.empty}|${tile.revision}|${tile.id === activeId() && state.settled && state.settled.printer ? state.settled.printer.name : mine.thumbPrinter || ''}`;
    if (mine.thumbKey === key || mine.thumbBusy) continue;
    mine.thumbBusy = true;
    try {
      mine.thumb = await call('plate_view', { project: tile.id });
      mine.thumbKey = key;
      mine.thumbPrinter = key.split('|')[3];
    } catch {
      mine.thumbKey = key;
    }
    mine.thumbBusy = false;
  }
  renderStrip();
}

async function applyListing(listing) {
  const before = state.listing.active;
  state.listing = listing;
  // What was kept for a project that is closed goes with it.
  for (const id of [...state.kept.keys()]) {
    if (!listing.projects.some((project) => project.id === id)) state.kept.delete(id);
  }
  renderStrip();
  if (listing.active !== before || !state.project || state.project.id !== listing.active) await showProject();
  refreshThumbs();
}

async function refreshStrip() {
  await applyListing(await call('projects'));
}

// At the most projects a window keeps, another is opened only once one is
// closed: asks which. False when none was.
async function makeRoom(listing) {
  const names = listing.projects.map((project) => project.name + (project.dirty ? ' *' : ''));
  const picked = await confirmBox(
    'Too many open projects',
    `${listing.max_open} projects are open, the most this window keeps. Close one to open another:`,
    'OK', 'Cancel', names,
  );
  if (picked === null) return false;
  return closeProject(listing.projects[picked].id);
}

async function newProject() {
  let listing = await call('project_new');
  if (listing.full) {
    if (!(await makeRoom(listing))) return;
    listing = await call('project_new');
    if (listing.full) return;
  }
  if (state.tab === 'device') state.tab = 'prepare';
  state.project = null;
  await applyListing(listing);
}

async function openProject(path = null) {
  const file = path || await pick('model', remembered.get('model') || '');
  if (!file) return;
  let listing = await call('project_open', { path: file });
  if (listing.full) {
    if (!(await makeRoom(listing))) return;
    listing = await call('project_open', { path: file });
    if (listing.full) return;
  }
  remembered.set('model', file);
  say('');
  if (!listing.already) state.kept.delete(listing.id);
  // A file opened is looked at: on Device, the window goes to it.
  if (state.tab === 'device') state.tab = 'prepare';
  state.project = null;
  await applyListing(listing);
}

async function selectProject(id) {
  if (id === activeId()) return;
  await applyListing(await call('project_select', { id }));
}

async function cycleProject(step) {
  if (state.listing.projects.length < 2) return;
  await applyListing(await call('project_cycle', { step }));
}

// Closes a project, asking first about what would be lost and about a
// slice that is running. False when it stays open.
async function closeProject(id) {
  const args = { id };
  for (;;) {
    const answer = await call('project_close', args);
    if (answer.closed) {
      if (id === activeId()) state.project = null;
      await applyListing(answer);
      return true;
    }
    if (answer.ask === 'unsaved') {
      // The reference offers to save; a project cannot be saved here yet.
      const sure = await confirmBox(
        'Close project',
        `"${answer.name}" has unsaved changes.\nA project cannot be saved yet: what was chosen for it is lost when it closes.`,
        "Don't save", 'Cancel',
      );
      if (!sure) return false;
      args.discard = true;
    } else {
      const sure = await confirmBox('Close project', `"${answer.name}" is slicing. Stop slicing and close it?`, 'Yes', 'No');
      if (!sure) return false;
      args.stop = true;
    }
  }
}

async function closeOthers(id) {
  await selectProject(id);
  for (const other of state.listing.projects.filter((project) => project.id !== id)) {
    if (!(await closeProject(other.id))) break;
  }
}

$('project-new').addEventListener('click', () => attempt(newProject));
$('project-open').addEventListener('click', () => attempt(() => openProject()));
$('strip-fold').addEventListener('click', () => setStrip({ collapsed: !strip.collapsed }));

// A press on a tile selects it on release; dragged up or down, it moves.
const press = { id: null, y: 0, moving: false, drop: -1 };

function dropIndexAt(y) {
  const tiles = [...$('projects').querySelectorAll('.tile')];
  let index = tiles.length;
  for (const [at, tile] of tiles.entries()) {
    const box = tile.getBoundingClientRect();
    if (y < box.top + box.height / 2) { index = at; break; }
  }
  return index;
}

function showDrop() {
  for (const line of $('projects').querySelectorAll('.drop')) line.remove();
  if (!press.moving || press.drop < 0) return;
  const tiles = $('projects').querySelectorAll('.tile');
  const line = el('li', { className: 'drop' });
  if (press.drop >= tiles.length) $('projects').append(line); else tiles[press.drop].before(line);
}

$('projects').addEventListener('pointerdown', (event) => {
  const tile = event.target.closest('.tile');
  if (!tile || event.button !== 0 || event.target.closest('.close')) return;
  Object.assign(press, { id: Number(tile.dataset.id), y: event.clientY, moving: false, drop: -1 });
  $('projects').setPointerCapture(event.pointerId);
});
$('projects').addEventListener('pointermove', (event) => {
  if (press.id === null) return;
  if (!press.moving && Math.abs(event.clientY - press.y) > 6 && state.listing.projects.length > 1) press.moving = true;
  if (!press.moving) return;
  const drop = dropIndexAt(event.clientY);
  if (drop !== press.drop) { press.drop = drop; showDrop(); }
});
$('projects').addEventListener('pointerup', (event) => {
  if (press.id === null) return;
  const { id, moving, drop } = press;
  press.id = null;
  press.moving = false;
  showDrop();
  if ($('projects').hasPointerCapture(event.pointerId)) $('projects').releasePointerCapture(event.pointerId);
  if (!moving) {
    attempt(() => selectProject(id));
    return;
  }
  // The gap it was dropped in, as a place among the others.
  const from = state.listing.projects.findIndex((project) => project.id === id);
  let to = drop;
  if (from < 0 || to < 0) return;
  if (to > from) to -= 1;
  to = Math.max(0, Math.min(state.listing.projects.length - 1, to));
  if (to !== from) attempt(async () => applyListing(await call('project_move', { id, index: to })));
});
$('projects').addEventListener('pointercancel', () => { press.id = null; press.moving = false; showDrop(); });
$('projects').addEventListener('click', (event) => {
  const close = event.target.closest('.close');
  if (close) attempt(() => closeProject(Number(close.closest('.tile').dataset.id)));
});
// The middle button closes.
$('projects').addEventListener('auxclick', (event) => {
  const tile = event.target.closest('.tile');
  if (tile && event.button === 1) {
    event.preventDefault();
    attempt(() => closeProject(Number(tile.dataset.id)));
  }
});
$('strip').addEventListener('contextmenu', (event) => {
  event.preventDefault();
  const tile = event.target.closest('.tile');
  if (tile) {
    const id = Number(tile.dataset.id);
    showMenu(event.clientX, event.clientY, [
      ['Close', () => attempt(() => closeProject(id))],
      ['Close others', () => attempt(() => closeOthers(id)), { disabled: state.listing.projects.length < 2 }],
    ]);
    return;
  }
  showMenu(event.clientX, event.clientY, [
    ['Show thumbnails', () => setStrip({ text: false }), { checked: !strip.text }],
    ['Show names only', () => setStrip({ text: true }), { checked: strip.text }],
    null,
    ['Default width', () => setStrip({ width: 88 }), { disabled: strip.collapsed || strip.width === 88 }],
    [strip.collapsed ? 'Show the project strip' : 'Collapse the project strip', () => setStrip({ collapsed: !strip.collapsed })],
  ]);
});

// The right edge is dragged to widen the strip.
const grip = { from: null, width: 0 };
$('strip-grip').addEventListener('pointerdown', (event) => {
  grip.from = event.clientX;
  grip.width = Math.max(strip.width, strip.text ? 120 : 88);
  $('strip-grip').setPointerCapture(event.pointerId);
  $('strip').classList.add('dragging');
});
$('strip-grip').addEventListener('pointermove', (event) => {
  if (grip.from === null) return;
  strip.width = Math.max(88, Math.min(200, Math.round(grip.width + event.clientX - grip.from)));
  showStripShape();
});
for (const end of ['pointerup', 'pointercancel']) {
  $('strip-grip').addEventListener(end, () => {
    if (grip.from === null) return;
    grip.from = null;
    $('strip').classList.remove('dragging');
    setStrip({});
  });
}

// ---- the stage: what Prepare and Preview show ----

const canvas = $('view');
const context = canvas.getContext('2d');

// By feature, the reference's own colours (libvgcode `DEFAULT_EXTRUSION_ROLES_COLORS`).
const featureColours = {
  'Inner wall': '#ffe64d',
  'Outer wall': '#ff7d38',
  'Overhang wall': '#1f1fff',
  'Sparse infill': '#b03029',
  'Internal solid infill': '#9654cc',
  'Top surface': '#f04040',
  'Ironing': '#ff8c69',
  'Bridge': '#4d80ba',
  'Gap infill': '#ffffff',
  'Skirt': '#00876e',
  'Support': '#00ff00',
  'Support interface': '#008000',
  'Prime tower': '#b3e3ab',
  'Bottom surface': '#665cc7',
  'Internal Bridge': '#4d80ba',
  'Brim': '#003b6e',
};
const spareColours = ['#ff7d38', '#4da3ff', '#3ddc5a', '#ffe64d', '#e06fd0', '#8fc4e8', '#f04a4a', '#b9e3a8'];
// From least to most (libvgcode `DEFAULT_RANGES_COLORS`).
const rangeColours = [[11, 44, 122], [19, 89, 133], [28, 136, 145], [4, 214, 15], [170, 242, 0], [252, 249, 3], [245, 206, 10], [227, 136, 32], [209, 104, 48], [194, 82, 60], [148, 38, 22]];

// The views the engine has what it takes to colour, by the reference's names.
const viewTypes = {
  'Line Type': { title: 'Line Type' },
  'Filament': { title: 'Filament' },
  'Layer Height': { title: 'Layer height (mm)', range: (summary) => summary.heights },
  'Line Width': { title: 'Line width (mm)', range: (summary) => summary.widths },
};

function rangeColour(value, range) {
  if (!range || range[1] - range[0] < 1e-9) return `rgb(${rangeColours[0].join(',')})`;
  const t = Math.max(0, Math.min(1, (value - range[0]) / (range[1] - range[0]))) * (rangeColours.length - 1);
  const low = Math.min(Math.floor(t), rangeColours.length - 2);
  const mix = rangeColours[low].map((part, at) => Math.round(part + (rangeColours[low + 1][at] - part) * (t - low)));
  return `rgb(${mix.join(',')})`;
}

// A colour as drawn on the dark stage: one too dark to see there is
// lifted towards grey, enough to show and still read as dark.
function onDark(colour) {
  if (brightness(colour) >= 0.3) return colour;
  const parts = [1, 3, 5].map((at) => parseInt(colour.slice(at, at + 2), 16));
  const lifted = parts.map((part) => Math.round(part + (150 - part) * 0.6));
  return `#${lifted.map((part) => part.toString(16).padStart(2, '0')).join('')}`;
}

// The colour each filament is set to, where the filaments in use are
// set to colours that can be told apart; else one from a fixed list.
function filamentColour(summary, filament) {
  const given = (number) => {
    const colour = (summary.colours || [])[number - 1] || '';
    return /^#[0-9a-f]{6}/i.test(colour) ? colour.slice(0, 7).toLowerCase() : null;
  };
  const used = summary.filaments.map(given);
  const distinct = used.every(Boolean) && new Set(used).size === used.length;
  return distinct ? given(filament) : spareColours[(filament - 1) % spareColours.length];
}

const number = (value, digits = 1) => value.toLocaleString(undefined, { maximumFractionDigits: digits, minimumFractionDigits: digits });

function duration(minutes) {
  const whole = Math.round(minutes);
  return whole >= 60 ? `${Math.floor(whole / 60)}h ${whole % 60}m` : `${whole}m`;
}

let staging = 0;

// Shows the plate of the project in the tab it is on. `entering` Preview,
// a plate that is not sliced is sliced.
async function showStage({ entering = false } = {}) {
  const turn = ++staging;
  const project = state.project;
  if (!project || state.tab === 'device') return;
  const mine = here();
  const preview = state.tab === 'preview';
  showPlates();
  showButtons();
  showProgress();
  canvas.setAttribute('aria-label', preview ? 'Layer preview' : 'The plate');

  // Prepare: the bed and what stands on it. Preview shows it too until
  // there are layers to show.
  const view = await plateView();
  if (turn !== staging) return;
  mine.plate = view;

  let summary = null;
  if (preview && project.sliced) {
    summary = await summaryOf(plateKey());
    if (turn !== staging) return;
  }
  mine.summary = summary;
  for (const id of ['legend', 'vslider']) $(id).hidden = !summary;
  if (!summary) $('notes').hidden = true;
  if (summary) {
    showLegend();
    showNotes(summary);
    showButtons();
    const last = summary.layers - 1;
    // In 3D every layer is shown, up to the one the slider is on.
    if (!stage3d.active) await showLayer(Math.min(mine.layerAt[plateKey()] ?? 0, last), turn);
  } else {
    mine.shown = null;
  }
  if (turn !== staging) return;
  $('empty').textContent = emptyText(project, preview);
  // Another printer's bed is another size: Prepare is fitted to it anew.
  const bed = JSON.stringify(view.bed);
  if (mine.fittedBed !== bed) {
    mine.fittedBed = bed;
    mine.camera.prepare = null;
  }
  // Preview is fitted to the part once there are layers of it to show.
  if (preview) {
    const layers = summary ? String(plateKey()) : null;
    if (layers !== mine.fittedLayers) mine.camera.preview = null;
    mine.fittedLayers = layers;
  }
  if (!stage3d.active && !mine.camera[state.tab]) fit(preview && summary ? 'part' : 'bed');
  draw();

  if (preview && entering && !project.sliced && !slicingNow() && !project.failed
      && sliceable(project).includes(plateKey())) {
    await slice(false);
  }
}

function emptyText(project, preview) {
  if (project.empty) return 'The project is empty. Open a model: an STL or a 3MF project.';
  if (!preview) return '';
  if (project.sliced || slicingNow()) return '';
  if (project.failed) return project.failed;
  if (!sliceable(project).includes(plateKey())) return 'There is nothing to print on this plate.';
  return '';
}

async function plateView() {
  const project = state.project;
  const mine = here();
  const printer = state.settled && state.settled.printer ? state.settled.printer.name : '';
  const key = `${plateKey()}|${project.empty}|${project.edit.revision}|${printer}`;
  if (!mine.plateViews.has(key)) {
    try {
      mine.plateViews.set(key, await call('plate_view', { project: project.id }));
    } catch (error) {
      say(error.message, true);
      return { bed: null, objects: [] };
    }
  }
  return mine.plateViews.get(key);
}

// What a plate's slice came to, fetched once for each slice.
async function summaryOf(plate) {
  const id = activeId();
  const mine = kept(id);
  const run = state.project.status.run;
  const known = mine.summaries.get(plate);
  if (known && known.run === run) return known.summary;
  try {
    const summary = await call('summary', { project: id, plate: plate || undefined });
    mine.summaries.set(plate, { run, summary });
    return summary;
  } catch (error) {
    say(error.message, true);
    return null;
  }
}

function showNotes(summary) {
  const gcode = summary.gcode;
  const failed = 'error' in gcode;
  $('gcode-error').hidden = !failed;
  $('gcode-error').textContent = failed ? gcode.error : '';
  const items = (texts) => texts.map((text) => el('li', { textContent: text }));
  // What is wrong with this print, what was settled for the person, and
  // what the profile asks for that the engine does not do yet: three
  // different things, kept apart.
  $('problems').replaceChildren(...items(summary.problems));
  $('remarks').replaceChildren(...items(summary.remarks));
  const gaps = summary.gaps.length;
  $('gaps').hidden = gaps === 0;
  $('gaps').querySelector('summary').textContent = `${gaps} setting${gaps === 1 ? '' : 's'} this engine does not follow yet`;
  $('gaps').querySelector('ul').replaceChildren(...items(summary.gaps));
  tidyNotes();
}

// The notes are there while there is one to read.
function tidyNotes() {
  const any = !$('gcode-error').hidden || !$('gaps').hidden || $('exported').textContent !== ''
    || $('problems').children.length > 0 || $('remarks').children.length > 0;
  $('notes').hidden = !(any && here().summary && state.tab === 'preview');
}

// The legend: what the colours stand for, each to show or hide, and what
// the print comes to.
function showLegend() {
  const mine = here();
  const summary = mine.summary;
  // In 3D the legend's rows are the 3D view's own (gl/stage.js).
  const own = stage3d.active ? stage3d.legend(mine, summary, filamentColour) : null;
  const types = own ? own.types : Object.keys(viewTypes);
  if (!types.includes(mine.viewType)) mine.viewType = 'Line Type';
  fill($('view-type'), types, mine.viewType);
  const rows = $('legend-rows');
  const head = (...names) => el('tr', {}, ...names.map((name) => el('th', { textContent: name })));
  const toggle = (key, name, colour) => {
    const off = mine.hidden.has(key);
    const eye = el('button', { type: 'button', className: 'eye', textContent: off ? '○' : '●', title: off ? 'Show' : 'Hide' });
    eye.setAttribute('aria-label', `${off ? 'Show' : 'Hide'} ${name}`);
    eye.setAttribute('aria-pressed', String(!off));
    const swatch = el('span', { className: 'swatch' });
    swatch.style.background = colour;
    const row = el('tr', { className: `row${off ? ' off' : ''}` }, el('td', {}, swatch, name), el('td', {}, eye));
    row.addEventListener('click', () => {
      if (mine.hidden.has(key)) mine.hidden.delete(key); else mine.hidden.add(key);
      showLegend();
      draw();
      noteView();
    });
    return row;
  };
  const type = viewTypes[mine.viewType];
  if (own) {
    rows.replaceChildren(...own.rows(() => { showLegend(); draw(); noteView(); }));
  } else if (mine.viewType === 'Line Type') {
    rows.replaceChildren(head('Line Type', 'Display'),
      ...summary.roles.map((role) => toggle(role, role, featureColours[role] || '#ffffff')));
  } else if (mine.viewType === 'Filament') {
    rows.replaceChildren(head('Filament', 'Display'),
      ...summary.filaments.map((filament) => toggle(`f${filament}`, `Filament ${filament}`, filamentColour(summary, filament))));
  } else {
    // From most to least, as the reference lists a range.
    const range = type.range(summary) || [0, 0];
    const steps = range[1] - range[0] < 1e-9 ? 1 : 6;
    rows.replaceChildren(head(type.title), ...Array.from({ length: steps }, (_, index) => {
      const value = steps === 1 ? range[0] : range[1] - ((range[1] - range[0]) * index) / (steps - 1);
      const swatch = el('span', { className: 'swatch' });
      swatch.style.background = rangeColour(value, range);
      return el('tr', {}, el('td', {}, swatch, number(value, 2)));
    }));
  }

  const gcode = summary.gcode;
  const figures = [];
  if (!('error' in gcode)) {
    const weight = gcode.grams === null ? '' : `  ${number(gcode.grams, 2)} g`;
    figures.push(['Total Filament:', `${number(gcode.filament_m, 2)} m${weight}`]);
    if (gcode.tool_changes > 0) figures.push(['Filament change times:', String(gcode.tool_changes)]);
    figures.push(['Total time:', duration(gcode.minutes)]);
    const how = { klipper: 'Planned as Klipper plans it; heating is not counted.', acceleration: 'With acceleration; heating is not counted.' };
    $('timed').textContent = how[gcode.timed] || 'At the speeds set, before acceleration.';
  } else {
    $('timed').textContent = '';
  }
  $('figures').replaceChildren(...figures.flatMap(([name, value]) => [el('dt', { textContent: name }), el('dd', { textContent: value })]));
}

$('view-type').addEventListener('change', () => {
  const mine = here();
  mine.viewType = $('view-type').value;
  showLegend();
  draw();
  noteView();
});
$('legend-fold').addEventListener('click', () => {
  const folded = $('legend').classList.toggle('folded');
  $('legend-fold').textContent = folded ? '▸' : '▾';
  $('legend-fold').setAttribute('aria-expanded', String(!folded));
});

async function showLayer(index, turn = staging) {
  const mine = here();
  const summary = mine.summary;
  if (!summary) return;
  if (stage3d.active) { stage3d.setLayer(index); return; }
  const project = state.project;
  const plate = plateKey();
  index = Math.max(0, Math.min(summary.layers - 1, Math.round(index)));
  mine.layerAt[plate] = index;
  placeThumb(index, summary);
  const wanted = ++mine.wanted;
  const key = `${project.status.run}:${plate}:${index}`;
  let layer = mine.layers.get(key);
  if (!layer) {
    try {
      layer = await call('layer', { project: project.id, plate: plate || undefined, index });
    } catch (error) {
      say(error.message, true);
      return;
    }
    // Lines of one feature, width and filament are stroked together.
    const groups = new Map();
    let [minX, minY, maxX, maxY] = [Infinity, Infinity, -Infinity, -Infinity];
    for (const stroke of layer.strokes) {
      const group = `${stroke.r}|${stroke.w}|${stroke.f}`;
      if (!groups.has(group)) groups.set(group, { role: stroke.r, width: stroke.w, filament: stroke.f, strokes: [] });
      groups.get(group).strokes.push(stroke);
      for (let i = 0; i < stroke.p.length; i += 2) {
        minX = Math.min(minX, stroke.p[i]); maxX = Math.max(maxX, stroke.p[i]);
        minY = Math.min(minY, stroke.p[i + 1]); maxY = Math.max(maxY, stroke.p[i + 1]);
      }
    }
    layer.groups = [...groups.values()];
    layer.box = layer.strokes.length ? [minX, minY, maxX, maxY] : null;
    if (mine.layers.size > 80) mine.layers.delete(mine.layers.keys().next().value);
    mine.layers.set(key, layer);
  }
  // A slower answer for a layer, or a project, since left is dropped.
  if (wanted !== mine.wanted || turn !== staging || project.id !== activeId()) return;
  mine.shown = layer;
  placeThumb(index, summary, layer);
  draw();
}

// The slider's thumb and the label beside it: the layer's number over its
// height, as the reference labels its own.
function placeThumb(index, summary, layer = null) {
  const part = summary.layers > 1 ? index / (summary.layers - 1) : 0;
  $('vthumb').style.bottom = `${100 * part}%`;
  $('vfill').style.height = `${100 * part}%`;
  const label = $('layer-label');
  label.style.bottom = `${100 * part}%`;
  label.textContent = `${index + 1}\n${layer ? layer.z.toFixed(2) : ''}`;
  const track = $('vtrack');
  track.setAttribute('aria-valuemin', '1');
  track.setAttribute('aria-valuemax', String(summary.layers));
  track.setAttribute('aria-valuenow', String(index + 1));
  if (layer) track.setAttribute('aria-valuetext', `Layer ${index + 1} of ${summary.layers}, top at ${layer.z} mm`);
}

function stepLayer(by) {
  const mine = here();
  if (!mine.summary) return;
  if (stage3d.active) { stage3d.stepLayer(by); return; }
  showLayer((mine.layerAt[plateKey()] ?? 0) + by);
  noteView();
}

const track = $('vtrack');
let sliding = false;
function slideTo(event) {
  const mine = here();
  if (!mine.summary) return;
  const box = track.getBoundingClientRect();
  const part = 1 - Math.max(0, Math.min(1, (event.clientY - box.top) / box.height));
  showLayer(part * (mine.summary.layers - 1));
  noteView();
}
track.addEventListener('pointerdown', (event) => {
  if (event.button !== 0) return;
  sliding = true;
  track.setPointerCapture(event.pointerId);
  slideTo(event);
});
track.addEventListener('pointermove', (event) => { if (sliding) slideTo(event); });
for (const end of ['pointerup', 'pointercancel']) track.addEventListener(end, () => { sliding = false; });
// The wheel moves a layer at a time; five with Shift or Ctrl held.
$('vslider').addEventListener('wheel', (event) => {
  event.preventDefault();
  stepLayer((event.deltaY < 0 ? 1 : -1) * (event.shiftKey || event.ctrlKey || event.metaKey ? 5 : 1));
}, { passive: false });
$('vslider').addEventListener('contextmenu', (event) => {
  event.preventDefault();
  showMenu(event.clientX, event.clientY, [['Jump to Layer', () => attempt(jumpToLayer)]]);
});

async function jumpToLayer() {
  const mine = here();
  if (state.tab !== 'preview' || !mine.summary) return;
  const typed = await askText(`Please enter the layer number (1 - ${mine.summary.layers}):`, String((mine.layerAt[plateKey()] ?? 0) + 1));
  const layer = Number(typed);
  if (typed === null || !Number.isFinite(layer)) return;
  await showLayer(layer - 1);
  noteView();
}

// The plates of the project, to pick from, with how each stands: sliced,
// slicing or failed. Two or more plates only.
function showPlates() {
  const project = state.project;
  const bar = $('plates');
  if (!project || project.plates.length < 2) {
    bar.hidden = true;
    return;
  }
  const status = project.status;
  const buttons = project.plates.map((plate) => {
    const holds = plate.filaments.length > 0;
    const slicing = status.state === 'slicing' && status.plate === plate.id;
    const how = plate.sliced ? 'sliced' : plate.failed ? 'failed' : slicing ? 'slicing' : 'unsliced';
    const button = el('button', { type: 'button', className: `${how}${plate.id === project.plate ? ' current' : ''}` },
      el('span', { className: 'number', textContent: String(plate.id) }));
    const said = { sliced: 'sliced', failed: `slicing failed: ${plate.failed}`, slicing: 'slicing', unsliced: holds ? 'not sliced' : 'empty' }[how];
    button.title = `Plate ${plate.id}${plate.name ? ` (${plate.name})` : ''}: ${said}`;
    button.setAttribute('aria-pressed', String(plate.id === project.plate));
    const mark = { sliced: '✓', failed: '!', slicing: '…' }[how];
    if (mark) button.append(el('span', { className: 'mark', textContent: mark }));
    if (!holds) button.append(el('small', { textContent: 'empty' }));
    button.addEventListener('click', () => attempt(() => selectPlate(plate.id)));
    return button;
  });
  // All of them at once: slice what is not sliced; once all are, the totals.
  const holding = sliceable(project);
  const done = holding.filter((plate) => isSliced(project, plate)).length;
  const all = el('button', { type: 'button', className: 'all', disabled: status.state === 'slicing' && done < holding.length },
    el('span', { textContent: done < holding.length ? 'Slice all' : 'All Plates' }),
    el('small', { textContent: done < holding.length ? `${done}/${holding.length}` : 'Stats' }));
  all.addEventListener('click', () => attempt(() => (done < holding.length ? slice(true) : showStats())));
  bar.replaceChildren(all, ...buttons);
  bar.hidden = false;
}

async function selectPlate(id) {
  if (id === state.project.plate) return;
  // The plate left is not shown as the one gone to while that is fetched.
  const mine = here();
  Object.assign(mine, { summary: null, shown: null, plate: null });
  mine.camera.preview = null;
  draw();
  await setProject({ plate: id }, { stage: false });
  await showStage({ entering: true });
}

async function showStats() {
  const project = state.project;
  const rows = [];
  let [metres, grams, minutes, weighed] = [0, 0, 0, true];
  for (const plate of sliceable(project)) {
    const summary = await summaryOf(plate);
    if (!summary || 'error' in summary.gcode) continue;
    const gcode = summary.gcode;
    metres += gcode.filament_m;
    minutes += gcode.minutes;
    if (gcode.grams === null) weighed = false; else grams += gcode.grams;
    rows.push(el('tr', {}, el('td', { textContent: `Plate ${plate}` }),
      el('td', { textContent: `${number(gcode.filament_m, 2)} m` }),
      el('td', { textContent: gcode.grams === null ? '' : `${number(gcode.grams, 2)} g` }),
      el('td', { textContent: duration(gcode.minutes) })));
  }
  const table = $('stats-table');
  table.replaceChildren(
    el('thead', {}, el('tr', {}, ...['', 'Filament', '', 'Total time'].map((name) => el('th', { textContent: name })))),
    el('tbody', {}, ...rows),
    el('tfoot', {}, el('tr', {}, el('td', { textContent: 'Total' }),
      el('td', { textContent: `${number(metres, 2)} m` }),
      el('td', { textContent: weighed ? `${number(grams, 2)} g` : '' }),
      el('td', { textContent: duration(minutes) }))),
  );
  await modal($('stats'));
}

// ---- drawing ----

// The stage in 3D (gl/): Prepare's scene and Preview's print. Where the
// browser has no WebGL2 it stays off, and the view from above below is
// what the stage shows.
const stage3d = window.EdgeGL.stage;
stage3d.init({
  canvas: $('view3d'),
  stage: $('stage'),
  note: $('gl-note'),
  call,
  bytes: sendBytes,
  menu: showMenu,
  say,
  remembered,
  open: () => state.listing.projects.map((project) => project.id),
  plate: (id) => attempt(() => selectPlate(id)),
  jump: () => attempt(jumpToLayer),
  legend: () => { if (state.tab === 'preview' && state.project && here().summary) showLegend(); },
  noted: () => noteView(),
  filamentColour: (summary, filament) => filamentColour(summary, filament),
});

// The tools that change what stands on the plates (scene-edit.js) and
// the object list (objects.js). An edit answers with the project as it
// then is, which `editApplied` puts on the rest of the page.
function editApplied(project) {
  if (project.id !== activeId()) return;
  const before = state.project;
  state.project = project;
  const mine = here();
  // What was fetched for the model as it was is of no use any more.
  mine.plateViews.clear();
  Object.assign(mine, { summary: null, shown: null });
  showPlates();
  showButtons();
  $('empty').textContent = emptyText(project, state.tab === 'preview');
  // More or fewer filaments, or a project that is no longer empty: the
  // sidebar lists them.
  const listed = (shown) => (shown ? `${shown.empty}|${shown.filament_count}|${shown.used}` : '');
  if (listed(before) !== listed(project)) attempt(showSidebar);
  // In Preview the plate is shown as it now is: not sliced.
  if (state.tab === 'preview') attempt(() => showStage());
  attempt(refreshStrip);
}
window.EdgeGL.editor.init({
  stage: stage3d,
  stageNode: $('stage'),
  call,
  say,
  pick,
  menu: showMenu,
  el,
  listbox,
  remembered,
  project: () => state.project,
  tab: () => state.tab,
  settled: () => state.settled,
  applied: editApplied,
  plate: (id) => attempt(() => selectPlate(id)),
});
window.EdgeGL.objectList.init({
  el,
  listbox,
  project: () => state.project,
  plate: (id) => attempt(() => selectPlate(id)),
});

// Where the tab's view of the plate is; until it has been fitted, a
// stand-in that is not kept.
function camera() {
  return here().camera[state.tab] || { scale: 2, x: 40, y: 400 };
}

// The view to move: fitted first if it never was.
function ownCamera() {
  const mine = here();
  if (!mine.camera[state.tab]) fit('bed');
  if (!mine.camera[state.tab]) mine.camera[state.tab] = { scale: 2, x: 40, y: 400 };
  mine.camera[state.tab].fitted = null;
  return mine.camera[state.tab];
}

function fit(what) {
  if (stage3d.active) { stage3d.fit(what); return; }
  const mine = here();
  const preview = state.tab === 'preview';
  const bed = (mine.summary && preview && mine.summary.bed) || (mine.plate && mine.plate.bed);
  let part = null;
  if (preview && mine.shown && mine.shown.box) part = mine.shown.box;
  else if (mine.plate && mine.plate.objects.length) {
    part = mine.plate.objects.map((object) => object.box)
      .reduce((a, b) => [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[2], b[2]), Math.max(a[3], b[3])]);
  }
  const box = what === 'bed' ? bed || part : part || bed;
  if (!box) return;
  const { width, height } = canvas.getBoundingClientRect();
  if (!width || !height) return;
  const margin = box === bed ? 1.1 : 1.5;
  const [w, h] = [Math.max(box[2] - box[0], 1) * margin, Math.max(box[3] - box[1], 1) * margin];
  // Room is left at the right for the layer slider, and for the legend
  // where the stage is wide enough to spare it.
  let aside = 0;
  if (preview && mine.summary) {
    aside = 64;
    const legend = $('legend').offsetWidth;
    if (what !== 'bed' && legend && width - legend > 520) aside = legend + 96;
  }
  const usable = Math.max(width - aside, 50);
  const scale = Math.min(usable / w, height / h);
  mine.camera[state.tab] = {
    scale,
    x: usable / 2 - scale * (box[0] + box[2]) / 2,
    y: height / 2 + scale * (box[1] + box[3]) / 2,
    // Fitted, and not moved since: it is fitted again when the stage changes size.
    fitted: what,
  };
  draw();
}

function drawBed(bed, hair, scale) {
  context.fillStyle = '#1b1f26';
  context.fillRect(bed[0], bed[1], bed[2] - bed[0], bed[3] - bed[1]);
  for (const [step, colour] of [[10, '#232831'], [50, '#2d333e']]) {
    if (step * scale < 6) continue;
    context.beginPath();
    for (let x = Math.ceil(bed[0] / step) * step; x <= bed[2]; x += step) { context.moveTo(x, bed[1]); context.lineTo(x, bed[3]); }
    for (let y = Math.ceil(bed[1] / step) * step; y <= bed[3]; y += step) { context.moveTo(bed[0], y); context.lineTo(bed[2], y); }
    context.strokeStyle = colour;
    context.lineWidth = hair;
    context.stroke();
  }
  context.strokeStyle = '#4a5261';
  context.lineWidth = hair;
  context.strokeRect(bed[0], bed[1], bed[2] - bed[0], bed[3] - bed[1]);
}

function draw() {
  if (stage3d.active) {
    // The stage in 3D shows the same things from what it is told here.
    const project = state.project;
    stage3d.update({ project, settled: state.settled, tab: state.tab, kept: project ? here() : null, summary: project && state.tab === 'preview' ? here().summary : null });
    return;
  }
  const { width, height } = canvas.getBoundingClientRect();
  const ratio = window.devicePixelRatio || 1;
  if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
  }
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, width, height);
  if (!state.project || state.tab === 'device') return;
  const mine = here();
  const preview = state.tab === 'preview';
  const summary = preview ? mine.summary : null;
  const view = camera();
  // Millimetres, with +y up as on the bed.
  context.save();
  context.translate(view.x, view.y);
  context.scale(view.scale, -view.scale);
  const hair = 1 / view.scale;
  const bed = (summary && summary.bed) || (mine.plate && mine.plate.bed);
  if (bed) drawBed(bed, hair, view.scale);

  const layer = summary ? mine.shown : null;
  if (layer) {
    context.lineCap = 'round';
    context.lineJoin = 'round';
    const type = mine.viewType;
    const range = viewTypes[type].range ? viewTypes[type].range(summary) : null;
    for (const group of layer.groups) {
      let colour;
      if (type === 'Filament') {
        if (mine.hidden.has(`f${group.filament}`)) continue;
        colour = onDark(filamentColour(summary, group.filament));
      } else if (type === 'Line Type') {
        if (mine.hidden.has(group.role)) continue;
        colour = onDark(featureColours[group.role] || '#ffffff');
      } else {
        colour = rangeColour(type === 'Line Width' ? group.width : layer.height, range);
      }
      context.beginPath();
      for (const { p, c } of group.strokes) {
        context.moveTo(p[0], p[1]);
        for (let i = 2; i < p.length; i += 2) context.lineTo(p[i], p[i + 1]);
        if (c) context.closePath();
      }
      context.strokeStyle = colour;
      // Never thinner than a pixel, so a zoomed-out bed still shows its part.
      context.lineWidth = Math.max(group.width * 0.92, hair);
      context.stroke();
    }
    context.restore();
    return;
  }

  // Not sliced, or Prepare: each object's outline from above.
  const objects = mine.plate ? mine.plate.objects : [];
  const accent = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim();
  context.lineJoin = 'round';
  for (const { outline } of objects) {
    context.beginPath();
    for (let i = 0; i < outline.length; i += 2) {
      if (i) context.lineTo(outline[i], outline[i + 1]); else context.moveTo(outline[i], outline[i + 1]);
    }
    context.closePath();
    context.globalAlpha = preview ? 0.12 : 0.3;
    context.fillStyle = accent;
    context.fill();
    context.globalAlpha = preview ? 0.5 : 1;
    context.strokeStyle = accent;
    context.lineWidth = 1.5 * hair;
    context.stroke();
    context.globalAlpha = 1;
  }
  context.restore();
  // Names, upright, where there is room for them.
  context.font = '12px system-ui, -apple-system, "Segoe UI", sans-serif';
  context.textAlign = 'center';
  context.textBaseline = 'middle';
  context.fillStyle = '#e6e8ec';
  for (const { name, box, height: tall } of objects) {
    const wide = (box[2] - box[0]) * view.scale;
    const label = name || '';
    const [x, y] = [view.x + view.scale * (box[0] + box[2]) / 2, view.y - view.scale * (box[1] + box[3]) / 2];
    if (label && context.measureText(label).width < wide - 8) context.fillText(label, x, y - 8);
    const size = `${number(box[2] - box[0])} × ${number(box[3] - box[1])} × ${number(tall)} mm`;
    if (!preview && context.measureText(size).width < wide - 8) {
      context.globalAlpha = 0.7;
      context.fillText(size, x, label ? y + 8 : y);
      context.globalAlpha = 1;
    }
  }
}

// Wheel zooms about the pointer; dragging moves the view.
canvas.addEventListener('wheel', (event) => {
  event.preventDefault();
  const rect = canvas.getBoundingClientRect();
  const [px, py] = [event.clientX - rect.left, event.clientY - rect.top];
  const view = ownCamera();
  const factor = Math.exp(-event.deltaY * 0.0015);
  const scale = Math.max(0.2, Math.min(400, view.scale * factor));
  view.x = px - (px - view.x) * (scale / view.scale);
  view.y = py - (py - view.y) * (scale / view.scale);
  view.scale = scale;
  draw();
}, { passive: false });

let drag = null;
canvas.addEventListener('pointerdown', (event) => {
  canvas.focus({ preventScroll: true });
  drag = { x: event.clientX, y: event.clientY };
  canvas.setPointerCapture(event.pointerId);
  canvas.classList.add('dragging');
});
canvas.addEventListener('pointermove', (event) => {
  if (!drag) return;
  const view = ownCamera();
  view.x += event.clientX - drag.x;
  view.y += event.clientY - drag.y;
  drag = { x: event.clientX, y: event.clientY };
  draw();
});
for (const end of ['pointerup', 'pointercancel']) {
  canvas.addEventListener(end, () => {
    drag = null;
    canvas.classList.remove('dragging');
  });
}
new ResizeObserver(() => {
  if (stage3d.active) return;
  const view = state.project && state.tab !== 'device' ? here().camera[state.tab] : undefined;
  if (view === null) fit(state.tab === 'preview' && here().summary ? 'part' : 'bed');
  else if (view && view.fitted) fit(view.fitted);
  else draw();
}).observe(canvas);
for (const button of document.querySelectorAll('[data-fit]')) {
  button.addEventListener('click', () => fit(button.dataset.fit));
}

// ---- Device ----

// The printer of the project shown, as its preset or the project
// describes it. Nothing is asked of the printer itself: this build has no
// connection to one.
function showDevice() {
  const printer = state.settled ? state.settled.printer : null;
  const facts = [];
  if (!printer) {
    $('device-name').textContent = 'No printer';
    $('device-source').textContent = 'No printer is chosen for the project shown. Choose one in Prepare.';
    $('device-facts').replaceChildren();
    $('device-connection').textContent = 'Please set up your printer connection to view the device.';
    $('device-offline').textContent = '';
    return;
  }
  $('device-name').textContent = printer.name || printer.model || 'Printer';
  $('device-source').textContent = printer.source === 'preset'
    ? `The printer preset chosen for "${state.project.name}".`
    : `The printer "${state.project.name}" was saved with.`;
  const add = (name, value) => { if (value !== null && value !== undefined && value !== '') facts.push([name, String(value)]); };
  add('Vendor', printer.vendor);
  add('Model', printer.model);
  const nozzles = printer.nozzle_diameter || [];
  if (nozzles.length) add(nozzles.length > 1 ? 'Nozzles' : 'Nozzle', nozzles.map((size) => `${size} mm`).join(', '));
  if (printer.bed) add('Bed', `${number(printer.bed[2] - printer.bed[0], 0)} × ${number(printer.bed[3] - printer.bed[1], 0)} mm`);
  if (printer.height) add('Height', `${number(printer.height, 0)} mm`);
  add('G-code flavor', printer.gcode_flavor);
  add('Bed type', (printer.bed_types.choices.find((choice) => choice.value === printer.bed_types.in_force) || {}).label || printer.bed_types.in_force);
  add('Host Type', printer.host_type);
  add('Hostname, IP or URL', printer.print_host);
  add('Device UI', printer.print_host_webui);
  if (printer.print_host || printer.print_host_webui) add('API Key / Password', printer.has_api_key ? 'set in the preset' : 'none');
  $('device-facts').replaceChildren(...facts.flatMap(([name, value]) => [el('dt', { textContent: name }), el('dd', { textContent: value })]));
  const address = printer.print_host_webui || printer.print_host;
  $('device-connection').textContent = address
    ? `The preset gives this printer's address as ${address}.`
    : 'Please set up your printer connection to view the device.';
  $('device-offline').textContent = address
    ? 'This build does not connect to a printer yet: nothing is sent to that address and nothing is read from it. Export the G-code file and send it with the printer\'s own page.'
    : 'This build does not connect to a printer yet. Neither the preset nor the project gives the printer\'s address, and it cannot be set here.';
}

// ---- keeping up with what the session is doing ----

let timer = null;
function schedule(after) {
  clearTimeout(timer);
  timer = setTimeout(tick, after);
}

// The strip's marks for every project, shown or not, twice a second; the
// shown project's progress more often while it slices.
async function tick() {
  let busy = false;
  try {
    const seen = changes;
    const listing = await call('projects');
    busy = listing.projects.some((project) => project.slicing);
    if (seen !== changes) {
      schedule(150);
      return;
    }
    const id = listing.active;
    const mine = kept(id);
    const tile = listing.projects.find((project) => project.id === id);
    const was = mine.wasSlicing;
    mine.wasSlicing = Boolean(tile && tile.slicing);
    if (listing.active === state.listing.active) {
      state.listing = listing;
      renderStrip();
      refreshThumbs();
      if (state.project && (tile.slicing || was || slicingNow())) {
        const before = state.project.status.state;
        await refreshProject();
        showProgress();
        // A slice that ended while it was looked at: what came of it.
        if (state.project.id === id && before === 'slicing' && state.project.status.state !== 'slicing') {
          if (state.project.status.state === 'failed') say(state.project.status.error, true);
          await showStage();
          await refreshStrip();
        }
      }
    } else {
      await applyListing(listing);
    }
  } catch (error) {
    say(error.message, true);
  }
  schedule(busy ? 150 : 500);
}

// ---- keys ----

document.addEventListener('keydown', (event) => {
  if (document.querySelector('dialog[open]')) return;
  if (event.key === 'Escape' && !$('menu').hidden) { hideMenu(); return; }
  const command = event.ctrlKey || event.metaKey;
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  const typing = event.target.matches('input, select, textarea');
  // New and Open, and round the open projects, as the reference binds them.
  if (command && !event.shiftKey && !event.altKey && key === 'n') { event.preventDefault(); attempt(newProject); return; }
  if (command && !event.shiftKey && !event.altKey && key === 'o') { event.preventDefault(); attempt(() => openProject()); return; }
  if (event.ctrlKey && !event.altKey && (key === 'PageDown' || key === 'PageUp')) {
    event.preventDefault();
    attempt(() => cycleProject(key === 'PageDown' ? 1 : -1));
    return;
  }
  if (event.metaKey && event.shiftKey && ['[', ']', '{', '}'].includes(event.key)) {
    event.preventDefault();
    attempt(() => cycleProject(event.key === ']' || event.key === '}' ? 1 : -1));
    return;
  }
  // Slice plate.
  if (command && !event.shiftKey && !event.altKey && key === 'r') {
    event.preventDefault();
    if (!$('slice').disabled) attempt(() => slice(false));
    return;
  }
  if (typing || command || event.altKey || state.tab === 'device') return;
  // Tab goes between Prepare and Preview while the plate has the keys
  // (it was clicked), as in the reference; anywhere else it moves on.
  if (key === 'Tab' && !event.shiftKey && (event.target === canvas || event.target === $('view3d'))) {
    event.preventDefault();
    attempt(() => selectTab(state.tab === 'prepare' ? 'preview' : 'prepare'));
    return;
  }
  if (key === 'z') { fit('part'); return; }
  if (state.tab !== 'preview' || !here().summary) return;
  if (key === 'ArrowUp' || key === 'ArrowDown') {
    event.preventDefault();
    stepLayer((key === 'ArrowUp' ? 1 : -1) * (event.shiftKey ? 5 : 1));
  } else if (event.shiftKey && key === 'g') {
    event.preventDefault();
    attempt(jumpToLayer);
  }
});
// Ctrl with an arrow moves five layers, as Shift does.
document.addEventListener('keydown', (event) => {
  if (!(event.ctrlKey || event.metaKey) || event.altKey || document.querySelector('dialog[open]')) return;
  if (event.target.matches('input, select, textarea') || state.tab !== 'preview' || !here().summary) return;
  if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
    event.preventDefault();
    stepLayer(event.key === 'ArrowUp' ? 5 : -5);
  }
});

// ---- start ----

(async () => {
  showStripShape();
  initPrinters();
  try {
    await loadLibrary(await call('library'));
    await applyListing(await call('projects'));
    const { model } = await call('startup');
    if (model) await openProject(model);
    // With no printer added yet, the printer selection opens by itself.
    await firstRun();
  } catch (error) {
    say(error.message, true);
  }
  schedule(500);
})();
