'use strict';

// The page talks to the engine through `call`. In the app that is Tauri's
// own channel; served to a browser (`edge-desktop --serve`) it is a POST
// to the same machine, carrying the token this page was served with.
const tauri = window.__TAURI__;
const token = document.querySelector('meta[name="edge-token"]').content;
const $ = (id) => document.getElementById(id);

async function call(name, args = {}) {
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

// A file or folder from the person: the system's own dialog in the app,
// a typed path in a browser, which may not see the disk for itself.
const asking = {
  model: 'Path of an STL or 3MF file on this machine',
  folder: 'Path of the folder of vendor profiles',
  gcode: 'Save the G-code as (a full path)',
};
let answerAsk = null;

function pick(what, suggested = '') {
  if (tauri) return tauri.core.invoke('pick', { what, suggested: suggested || null });
  if (answerAsk) answerAsk(null);
  $('ask-label').textContent = asking[what];
  $('ask-text').value = suggested;
  $('ask').hidden = false;
  $('ask-text').focus();
  $('ask-text').select();
  return new Promise((resolve) => {
    answerAsk = (value) => {
      answerAsk = null;
      $('ask').hidden = true;
      resolve(value);
    };
  });
}

$('ask').addEventListener('submit', (event) => {
  event.preventDefault();
  if (answerAsk) answerAsk($('ask-text').value.trim() || null);
});
$('ask-cancel').addEventListener('click', () => answerAsk && answerAsk(null));

// What was last chosen, kept for next time where the browser allows it.
const remembered = {
  get(key) {
    try { return localStorage.getItem(`edge.${key}`); } catch { return null; }
  },
  set(key, value) {
    try { localStorage.setItem(`edge.${key}`, value); } catch { /* not kept */ }
  },
};

const state = {
  model: null,
  // The filament presets made for the printer chosen.
  filamentNames: [],
  // The one the printer's own profile usually prints with.
  usualFilament: null,
  summary: null,
  layer: 0,
  shown: null,
  layers: new Map(),
  wanted: 0,
  hidden: new Set(),
  view: { scale: 4, x: 0, y: 0 },
  fitted: false,
};

function say(text, isError = false) {
  const status = $('status');
  status.textContent = text;
  status.className = isError ? 'error' : 'quiet';
}

function fill(select, names, keep) {
  select.replaceChildren(...names.map((name) => new Option(name, name)));
  select.disabled = names.length === 0;
  if (keep && names.includes(keep)) select.value = keep;
}

// ---- model and presets ----

async function loadLibrary(library) {
  $('profiles-dir').textContent = library.dir || 'none';
  fill($('vendor'), library.vendors, remembered.get('vendor'));
  await vendorChanged();
}

async function vendorChanged() {
  const vendor = $('vendor').value;
  if (!vendor) {
    for (const id of ['machine', 'process']) fill($(id), []);
    state.filamentNames = [];
    buildFilaments();
    return;
  }
  remembered.set('vendor', vendor);
  try {
    const { machines } = await call('machines', { vendor });
    fill($('machine'), machines, remembered.get(`machine.${vendor}`));
    await machineChanged();
  } catch (error) {
    say(error.message, true);
  }
}

async function machineChanged() {
  const [vendor, machine] = [$('vendor').value, $('machine').value];
  if (!machine) {
    fill($('process'), []);
    state.filamentNames = [];
    buildFilaments();
    return;
  }
  remembered.set(`machine.${vendor}`, machine);
  try {
    const usual = await call('presets', { vendor, machine });
    fill($('process'), usual.processes, remembered.get(`process.${machine}`) || usual.usual_process);
    state.filamentNames = usual.filaments;
    state.usualFilament = usual.usual_filament;
    buildFilaments();
  } catch (error) {
    say(error.message, true);
  }
}

// The filaments the model prints in, by number; one for a bare model.
const filamentsUsed = () => (state.model && state.model.used.length ? state.model.used : [1]);

// One choice of filament preset for each filament the model prints in.
function buildFilaments() {
  const machine = $('machine').value;
  const used = filamentsUsed();
  $('filaments').replaceChildren(...used.map((number) => {
    const label = document.createElement('label');
    const select = document.createElement('select');
    select.dataset.filament = number;
    fill(select, state.filamentNames, remembered.get(`filament.${machine}.${number}`) || remembered.get(`filament.${machine}`) || state.usualFilament);
    label.append(used.length === 1 && number === 1 ? 'Filament ' : `Filament ${number} `, select);
    return label;
  }));
}

// A preset for filament 1, 2 and so on up to the highest in use; one the
// model does not print in takes the first chosen.
function filamentsChosen() {
  const selects = [...$('filaments').querySelectorAll('select')];
  if (!selects.length || !selects[0].value) return [];
  const byNumber = new Map(selects.map((select) => [Number(select.dataset.filament), select.value]));
  const highest = Math.max(...byNumber.keys());
  return Array.from({ length: highest }, (_, index) => byNumber.get(index + 1) || selects[0].value);
}

function sourceChanged() {
  $('presets').hidden = $('source').value !== 'presets';
}

async function openModel(path) {
  try {
    const model = await call('open', { path });
    state.model = model;
    remembered.set('model', model.path);
    $('model-name').textContent = model.name;
    $('model-name').title = model.path;
    const plates = $('plate');
    plates.replaceChildren(...model.plates.map((id) => new Option(`Plate ${id}`, id)));
    if (model.plate !== null) plates.value = model.plate;
    $('plate-row').hidden = model.plates.length < 2;
    buildFilaments();
    // A project brings its settings; a bare model needs presets.
    const source = $('source');
    source.options[0].disabled = !model.has_settings;
    source.value = model.has_settings ? 'project' : 'presets';
    sourceChanged();
    const named = [model.printer, model.process, ...(model.filaments || [])].filter(Boolean);
    $('project-presets').hidden = named.length === 0;
    $('project-presets').textContent = named.length ? `Saved with ${[...new Set(named)].join(', ')}.` : '';
    $('slice').disabled = false;
    say('');
  } catch (error) {
    say(error.message, true);
  }
}

// ---- slicing ----

let polling = null;

async function slice() {
  if (!state.model) return;
  const args = { path: state.model.path };
  if (!$('plate-row').hidden) args.plate = Number($('plate').value);
  if ($('source').value === 'presets') {
    for (const id of ['vendor', 'machine', 'process']) args[id] = $(id).value;
    args.filaments = filamentsChosen();
    remembered.set(`process.${args.machine}`, args.process);
    for (const select of $('filaments').querySelectorAll('select')) {
      remembered.set(`filament.${args.machine}.${select.dataset.filament}`, select.value);
    }
    if (args.filaments.length) remembered.set(`filament.${args.machine}`, args.filaments[0]);
  }
  if ($('bed').value) args.bed_type = $('bed').value;
  try {
    await call('slice', args);
  } catch (error) {
    say(error.message, true);
    return;
  }
  clearResult();
  $('slice').disabled = true;
  $('cancel').hidden = false;
  $('progress').hidden = false;
  clearInterval(polling);
  polling = setInterval(poll, 120);
  poll();
}

async function poll() {
  let status;
  try {
    status = await call('status');
  } catch (error) {
    say(error.message, true);
    return;
  }
  $('bar').style.width = `${Math.round((100 * status.step) / status.steps)}%`;
  if (status.state === 'slicing') {
    say(`${status.label}…`);
    return;
  }
  clearInterval(polling);
  polling = null;
  $('slice').disabled = false;
  $('cancel').hidden = true;
  $('progress').hidden = true;
  if (status.state === 'done') {
    say('');
    await showResult();
  } else if (status.state === 'cancelled') {
    say('Slicing was stopped.');
  } else if (status.state === 'failed') {
    say(status.error, true);
  }
}

function clearResult() {
  state.summary = null;
  state.shown = null;
  state.layers.clear();
  state.fitted = false;
  for (const id of ['result', 'legend', 'layerbar', 'fit']) $(id).hidden = true;
  $('empty').hidden = false;
  $('exported').textContent = '';
  draw();
}

const number = (value, digits = 1) => value.toLocaleString(undefined, { maximumFractionDigits: digits, minimumFractionDigits: digits });

function duration(minutes) {
  const whole = Math.round(minutes);
  return whole >= 60 ? `${Math.floor(whole / 60)} h ${whole % 60} min` : `${whole} min`;
}

async function showResult() {
  const summary = await call('summary');
  state.summary = summary;
  const figures = [['Layers', `${summary.layers} at ${summary.layer_height} mm`]];
  const gcode = summary.gcode;
  const failed = 'error' in gcode;
  if (!failed) {
    const weight = gcode.grams === null ? '' : `, ${number(gcode.grams)} g`;
    figures.push(['Filament', `${number(gcode.filament_m, 2)} m${weight}`]);
    const how = { klipper: 'planned as Klipper plans it; heating not counted', acceleration: 'with acceleration; heating not counted' };
    figures.push(['Time', `${duration(gcode.minutes)}, ${how[gcode.timed] || 'at the speeds set, before acceleration'}`]);
    figures.push(['Travel', `${number(gcode.travel_m)} m, ${gcode.retractions} retractions`]);
    if (gcode.tool_changes > 0) figures.push(['Tool changes', String(gcode.tool_changes)]);
  }
  $('figures').replaceChildren(...figures.flatMap(([name, value]) => {
    const [term, detail] = [document.createElement('dt'), document.createElement('dd')];
    term.textContent = name;
    detail.textContent = value;
    return [term, detail];
  }));
  $('gcode-error').hidden = !failed;
  $('gcode-error').textContent = failed ? gcode.error : '';
  $('export').disabled = failed;
  const items = (texts) => texts.map((text) => Object.assign(document.createElement('li'), { textContent: text }));
  // What is wrong with this print, what was settled for the person, and
  // what the profile asks for that the engine does not do yet: three
  // different things, kept apart.
  $('problems').replaceChildren(...items(summary.problems));
  $('remarks').replaceChildren(...items(summary.remarks));
  const gaps = summary.gaps.length;
  $('gaps').hidden = gaps === 0;
  $('gaps').querySelector('summary').textContent = `${gaps} setting${gaps === 1 ? '' : 's'} this engine does not follow yet`;
  $('gaps').querySelector('ul').replaceChildren(...items(summary.gaps));
  $('result').hidden = false;

  const slider = $('layer');
  slider.max = summary.layers - 1;
  slider.value = 0;
  $('colour').value = 'feature';
  $('colour').closest('label').hidden = summary.filaments.length < 2;
  state.hidden.clear();
  buildLegend();
  for (const id of ['legend', 'layerbar', 'fit']) $(id).hidden = false;
  $('empty').hidden = true;
  await showLayer(0);
}

// ---- drawing ----

// By feature, the colours slicers have taught people to expect.
const featureColours = {
  'Outer wall': '#ff7d38',
  'Inner wall': '#ffe64d',
  'Overhang wall': '#4d6bff',
  'Top surface': '#f04a4a',
  'Bottom surface': '#7a6bd9',
  'Internal solid infill': '#b06fe0',
  'Sparse infill': '#c2453a',
  'Bridge': '#5fa8ff',
  'Internal Bridge': '#8fc4e8',
  'Gap infill': '#f2f2f2',
  'Support': '#3ddc5a',
  'Support interface': '#1f9e3c',
  'Skirt': '#2bb39a',
  'Brim': '#1d8f7c',
  'Prime tower': '#b9e3a8',
};
const spareColours = ['#ff7d38', '#4da3ff', '#3ddc5a', '#ffe64d', '#e06fd0', '#8fc4e8', '#f04a4a', '#b9e3a8'];

// The colour each filament is set to, where the filaments in use are
// set to colours that can be told apart; else one from a fixed list.
function filamentColour(filament) {
  const given = (number) => {
    const colour = (state.summary.colours || [])[number - 1] || '';
    return /^#[0-9a-f]{6}/i.test(colour) ? colour.slice(0, 7).toLowerCase() : null;
  };
  const used = state.summary.filaments.map(given);
  const distinct = used.every(Boolean) && new Set(used).size === used.length;
  return distinct ? onDark(given(filament)) : spareColours[(filament - 1) % spareColours.length];
}

// A colour as drawn on the dark stage: one too dark to see there is
// lifted towards grey, enough to show and still read as dark.
function onDark(colour) {
  const parts = [1, 3, 5].map((at) => parseInt(colour.slice(at, at + 2), 16));
  const brightness = (0.299 * parts[0] + 0.587 * parts[1] + 0.114 * parts[2]) / 255;
  if (brightness >= 0.3) return colour;
  const lifted = parts.map((part) => Math.round(part + (150 - part) * 0.6));
  return `#${lifted.map((part) => part.toString(16).padStart(2, '0')).join('')}`;
}

const byFilament = () => $('colour').value === 'filament';

function buildLegend() {
  const entries = byFilament()
    ? state.summary.filaments.map((filament) => [`f${filament}`, `Filament ${filament}`, filamentColour(filament)])
    : state.summary.roles.map((role) => [role, role, featureColours[role] || '#ffffff']);
  $('legend').replaceChildren(...entries.map(([key, name, colour]) => {
    const label = document.createElement('label');
    const box = Object.assign(document.createElement('input'), { type: 'checkbox', checked: !state.hidden.has(key) });
    box.addEventListener('change', () => {
      if (box.checked) state.hidden.delete(key); else state.hidden.add(key);
      draw();
    });
    const swatch = Object.assign(document.createElement('span'), { className: 'swatch' });
    swatch.style.background = colour;
    label.append(box, swatch, name);
    return label;
  }));
}

async function showLayer(index) {
  const summary = state.summary;
  if (!summary) return;
  index = Math.max(0, Math.min(summary.layers - 1, index));
  state.layer = index;
  $('layer').value = index;
  const wanted = ++state.wanted;
  let layer = state.layers.get(index);
  if (!layer) {
    try {
      layer = await call('layer', { index });
    } catch (error) {
      say(error.message, true);
      return;
    }
    // Lines of one feature, width and filament are stroked together.
    const groups = new Map();
    let [minX, minY, maxX, maxY] = [Infinity, Infinity, -Infinity, -Infinity];
    for (const stroke of layer.strokes) {
      const key = `${stroke.r}|${stroke.w}|${stroke.f}`;
      if (!groups.has(key)) groups.set(key, { role: stroke.r, width: stroke.w, filament: stroke.f, strokes: [] });
      groups.get(key).strokes.push(stroke);
      for (let i = 0; i < stroke.p.length; i += 2) {
        minX = Math.min(minX, stroke.p[i]); maxX = Math.max(maxX, stroke.p[i]);
        minY = Math.min(minY, stroke.p[i + 1]); maxY = Math.max(maxY, stroke.p[i + 1]);
      }
    }
    layer.groups = [...groups.values()];
    layer.box = layer.strokes.length ? [minX, minY, maxX, maxY] : null;
    if (state.layers.size > 80) state.layers.delete(state.layers.keys().next().value);
    state.layers.set(index, layer);
  }
  // A slower answer for a layer since left is dropped.
  if (wanted !== state.wanted) return;
  state.shown = layer;
  $('layer-label').textContent = `Layer ${index + 1} of ${summary.layers} · top at ${layer.z} mm · ${layer.height} mm thick`;
  if (!state.fitted) fit('part');
  draw();
}

const canvas = $('view');
const context = canvas.getContext('2d');

function fit(what) {
  const bed = state.summary && state.summary.bed;
  const box = what === 'bed' && bed ? bed : (state.shown && state.shown.box) || bed;
  if (!box) return;
  const { width, height } = canvas.getBoundingClientRect();
  const margin = what === 'bed' ? 1.06 : 1.5;
  const [w, h] = [Math.max(box[2] - box[0], 1) * margin, Math.max(box[3] - box[1], 1) * margin];
  // Room is left at the bottom for the layer bar.
  const usable = Math.max(height - 70, 50);
  const scale = Math.min(width / w, usable / h);
  state.view = {
    scale,
    x: width / 2 - scale * (box[0] + box[2]) / 2,
    y: usable / 2 + scale * (box[1] + box[3]) / 2,
  };
  state.fitted = true;
  draw();
}

function draw() {
  const { width, height } = canvas.getBoundingClientRect();
  const ratio = window.devicePixelRatio || 1;
  if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
  }
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, width, height);
  const layer = state.shown;
  if (!layer || !state.summary) return;
  const view = state.view;
  // Millimetres, with +y up as on the bed.
  context.translate(view.x, view.y);
  context.scale(view.scale, -view.scale);
  const hair = 1 / view.scale;

  const bed = state.summary.bed;
  if (bed) {
    context.fillStyle = '#1b1f26';
    context.fillRect(bed[0], bed[1], bed[2] - bed[0], bed[3] - bed[1]);
    for (const [step, colour] of [[10, '#232831'], [50, '#2d333e']]) {
      if (step * view.scale < 6) continue;
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

  context.lineCap = 'round';
  context.lineJoin = 'round';
  const filaments = byFilament();
  for (const group of layer.groups) {
    if (state.hidden.has(filaments ? `f${group.filament}` : group.role)) continue;
    context.beginPath();
    for (const { p, c } of group.strokes) {
      context.moveTo(p[0], p[1]);
      for (let i = 2; i < p.length; i += 2) context.lineTo(p[i], p[i + 1]);
      if (c) context.closePath();
    }
    context.strokeStyle = filaments ? filamentColour(group.filament) : featureColours[group.role] || '#ffffff';
    // Never thinner than a pixel, so a zoomed-out bed still shows its part.
    context.lineWidth = Math.max(group.width * 0.92, hair);
    context.stroke();
  }
}

// Wheel zooms about the pointer; dragging moves the view.
canvas.addEventListener('wheel', (event) => {
  event.preventDefault();
  const rect = canvas.getBoundingClientRect();
  const [px, py] = [event.clientX - rect.left, event.clientY - rect.top];
  const view = state.view;
  const factor = Math.exp(-event.deltaY * 0.0015);
  const scale = Math.max(0.2, Math.min(400, view.scale * factor));
  view.x = px - (px - view.x) * (scale / view.scale);
  view.y = py - (py - view.y) * (scale / view.scale);
  view.scale = scale;
  draw();
}, { passive: false });

let drag = null;
canvas.addEventListener('pointerdown', (event) => {
  drag = { x: event.clientX, y: event.clientY };
  canvas.setPointerCapture(event.pointerId);
  canvas.classList.add('dragging');
});
canvas.addEventListener('pointermove', (event) => {
  if (!drag) return;
  state.view.x += event.clientX - drag.x;
  state.view.y += event.clientY - drag.y;
  drag = { x: event.clientX, y: event.clientY };
  draw();
});
for (const end of ['pointerup', 'pointercancel']) {
  canvas.addEventListener(end, () => {
    drag = null;
    canvas.classList.remove('dragging');
  });
}
new ResizeObserver(draw).observe(canvas);

// ---- controls ----

$('open').addEventListener('click', async () => {
  const path = await pick('model', remembered.get('model') || '');
  if (path) await openModel(path);
});
$('plate').addEventListener('change', async () => {
  // Another plate may print in other filaments.
  try {
    const model = await call('open', { path: state.model.path, plate: Number($('plate').value) });
    state.model.used = model.used;
    buildFilaments();
  } catch (error) {
    say(error.message, true);
  }
});
$('vendor').addEventListener('change', vendorChanged);
$('machine').addEventListener('change', machineChanged);
$('source').addEventListener('change', sourceChanged);
$('slice').addEventListener('click', slice);
$('cancel').addEventListener('click', () => call('cancel').catch(() => {}));
$('layer').addEventListener('input', () => showLayer(Number($('layer').value)));
$('up').addEventListener('click', () => showLayer(state.layer + 1));
$('down').addEventListener('click', () => showLayer(state.layer - 1));
$('colour').addEventListener('change', () => {
  state.hidden.clear();
  buildLegend();
  draw();
});
for (const button of document.querySelectorAll('[data-fit]')) {
  button.addEventListener('click', () => fit(button.dataset.fit));
}
document.addEventListener('keydown', (event) => {
  if (!state.summary || event.target.matches('input[type="text"], select')) return;
  const steps = { ArrowUp: 1, ArrowDown: -1, PageUp: 10, PageDown: -10 };
  if (event.key in steps) {
    event.preventDefault();
    showLayer(state.layer + steps[event.key]);
  }
});

$('export').addEventListener('click', async () => {
  const path = await pick('gcode', `${state.summary.stem || 'print'}.gcode`);
  if (!path) return;
  try {
    const written = await call('export', { path });
    $('exported').textContent = `Saved ${written.path} (${number(written.bytes / 1e6, 2)} MB).`;
    $('exported').className = 'quiet';
  } catch (error) {
    $('exported').textContent = error.message;
    $('exported').className = 'error';
  }
});

$('profiles').addEventListener('click', async () => {
  const dir = await pick('folder', $('profiles-dir').textContent);
  if (!dir) return;
  try {
    await loadLibrary(await call('set_profiles', { dir }));
  } catch (error) {
    say(error.message, true);
  }
});

(async () => {
  sourceChanged();
  try {
    await loadLibrary(await call('library'));
    const { model } = await call('startup');
    if (model) await openModel(model);
  } catch (error) {
    say(error.message, true);
  }
})();
