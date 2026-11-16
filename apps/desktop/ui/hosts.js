'use strict';

// Sending a sliced plate to a printer, and the printer's own page in
// Device: "Physical Printer" (the connection), "Send G-code to printer
// host", "Print host upload queue", and Device's "Printer page" and
// "Storage". Everything goes through `call`; the requests to the printer
// are made by the session (src/hosts), never by this page. The page is
// told whether a key is set, never the key.

const hostsState = {
  // `print_host` for the project shown.
  info: null,
  // The queue as last asked for, and the jobs this page sent and has not
  // yet seen end (to say how each ended once).
  queue: { jobs: [], busy: false },
  watching: new Set(),
  selected: null,
  queueTimer: null,
  // Device: which part is shown, and what the frame was last pointed at.
  deviceTab: 'status',
  frameUrl: '',
  // A U1's toolheads as last read for the send dialog, the mapping shown;
  // whether Device shows the native page.
  toolheads: null,
  mapping: [],
  native: false,
};

// The printer's address may be framed in this document: the served page
// lists the origins its policy allows (serve.rs), the window allows any
// http or https frame (tauri.conf.json).
function framable(url) {
  const meta = document.querySelector('meta[name="edge-frames"]');
  if (!meta || meta.content === '__EDGE_FRAMES__') return true;
  try {
    const wanted = new URL(url).origin;
    return meta.content.split(' ').filter(Boolean).some((allowed) => new URL(allowed).origin === wanted);
  } catch {
    return false;
  }
}

async function refreshPrintHost() {
  const project = state.project;
  if (!project) {
    hostsState.info = null;
  } else {
    try {
      const info = await call('print_host', { project: project.id });
      if (!state.project || state.project.id !== project.id) return;
      hostsState.info = info;
    } catch {
      hostsState.info = null;
    }
  }
  $('printer-connect').disabled = !(hostsState.info && hostsState.info.printer);
  showButtons();
  if (state.tab === 'device') showPrinterPage();
}

// Whether "Print" can send the plate shown: a connection this app can
// send to. Why not, when it cannot.
function canSend() {
  const info = hostsState.info;
  if (!info || !info.printer) return [false, 'No printer is chosen for the project.'];
  if (!info.can_send) return [false, info.why_not || 'Please set up your printer connection to view the device.'];
  return [true, ''];
}

// ---- "Physical Printer" ----

let hostKeyCleared = false;
// The page's own list for the host type.
listbox($('host-type'), { bare: true });

function hostTypes(info) {
  const select = $('host-type');
  const current = info.connection.host_type;
  // The types this app can send to, and the one in force if it is not
  // one of them (said under the fields).
  const offered = info.host_types.filter((type) => type.built || type.value === current);
  fill(select, offered.map((type) => type.value), current, offered.map((type) => type.built ? t(type.label) : `${t(type.label)} (not yet)`));
  select.dataset.value = current;
}

function hostNote() {
  const info = hostsState.info;
  const type = $('host-type').value;
  const known = info.host_types.find((entry) => entry.value === type);
  const notes = [];
  if (known && !known.built) notes.push(`This app cannot send to a ${known.label} host yet.`);
  if (type === 'octoprint' && info.route === 'snapmaker') {
    notes.push('A Snapmaker U1 is sent to over its Moonraker on the local network, as the reference sends to one there; it asks for no key.');
  }
  if (type === 'elegoolink') notes.push('The Centauri Carbon is started over its websocket, on port 3030 of the same address.');
  notes.push(info.from === 'kept'
    ? 'Kept for this printer preset in this app\'s data folder, never in a project.'
    : 'As the printer preset gives it. OK keeps it for this printer preset in this app\'s data folder, never in a project.');
  $('host-note').textContent = notes.join(' ');
}

function typedConnection() {
  const typed = {
    host_type: $('host-type').value,
    print_host: $('host-address').value.trim(),
    print_host_webui: $('host-webui').value.trim(),
    printhost_cafile: $('host-cafile').value.trim(),
  };
  const key = $('host-key').value;
  if (key) typed.apikey = key;
  else if (hostKeyCleared) typed.apikey = null;
  return typed;
}

function showKeyField() {
  const info = hostsState.info;
  const set = info.connection.has_api_key && !hostKeyCleared;
  $('host-key').placeholder = set ? 'Set (type to change it)' : '';
  $('host-key-clear').hidden = !set;
}

async function openConnection() {
  await refreshPrintHost();
  const info = hostsState.info;
  if (!info || !info.printer) return;
  hostKeyCleared = false;
  $('host-printer').textContent = `For the printer preset "${info.printer}".`;
  hostTypes(info);
  $('host-address').value = info.connection.print_host;
  $('host-webui').value = info.connection.print_host_webui;
  $('host-key').value = '';
  $('host-cafile').value = info.connection.printhost_cafile;
  $('host-forget').hidden = info.from !== 'kept';
  $('host-result').hidden = true;
  showKeyField();
  hostNote();
  const dialog = $('host-dialog');
  dialog.returnValue = 'cancel';
  dialog.showModal();
  $('host-address').focus();
}

function hostResult(ok, text) {
  const result = $('host-result');
  result.hidden = false;
  result.className = ok ? 'ok' : 'error';
  result.textContent = ok ? `${t('Success!')}\n${text}` : text;
}

$('printer-connect').addEventListener('click', () => attempt(openConnection));
$('device-connect').addEventListener('click', () => attempt(openConnection));
$('device-setup').addEventListener('click', () => attempt(openConnection));
$('host-type').addEventListener('change', hostNote);
$('host-key').addEventListener('input', () => { if ($('host-key').value) hostKeyCleared = false; });
$('host-key-clear').addEventListener('click', () => {
  hostKeyCleared = true;
  $('host-key').value = '';
  showKeyField();
});
$('host-cafile-browse').addEventListener('click', () => attempt(async () => {
  const path = await pick('cafile', $('host-cafile').value);
  if (path) $('host-cafile').value = path;
}));
// "Test": what is typed, before it is kept, as the reference tests the
// preset being edited.
$('host-test').addEventListener('click', () => attempt(async () => {
  const button = $('host-test');
  button.disabled = true;
  const result = $('host-result');
  result.hidden = false;
  result.className = 'quiet';
  result.textContent = 'Testing…';
  try {
    const tested = await call('print_host_test', { project: activeId(), ...typedConnection() });
    hostResult(tested.ok, tested.message);
  } catch (error) {
    hostResult(false, error.message);
  } finally {
    button.disabled = false;
  }
}));
$('host-forget').addEventListener('click', () => attempt(async () => {
  hostsState.info = await call('print_host_set', { project: activeId(), forget: true });
  $('host-dialog').close('cancel');
  await refreshPrintHost();
}));
// OK keeps what is typed; a mistake keeps the dialog open with it said.
$('host-form').addEventListener('submit', (event) => {
  if (event.submitter && event.submitter.value !== 'ok') return;
  event.preventDefault();
  attempt(async () => {
    const before = hostsState.info && hostsState.info.device_url;
    try {
      hostsState.info = await call('print_host_set', { project: activeId(), ...typedConnection() });
    } catch (error) {
      hostResult(false, error.message);
      return;
    }
    $('host-dialog').close('ok');
    await refreshPrintHost();
    const url = hostsState.info.device_url;
    // The served page's policy names the printers it may frame when it is
    // loaded: one it does not name yet needs the page again (the session,
    // and every project, stay as they are).
    if (url && url !== before && !framable(url)) {
      say('The printer\'s page is shown after the page is loaded again.');
      location.reload();
    }
  });
});

// ---- the print button ----

const PRINT_ACTIONS = { bambu_print: 'Print plate', send: 'Print', sliced: 'Export plate sliced file', all_sliced: 'Export all sliced file', export: 'Export G-code file', bambu_connect: 'Send to Bambu Connect' };

// A Bambu Lab printer's project (its sliced file goes to Bambu Connect).
const isBambu = () => Boolean(hostsState.info && hostsState.info.bambu);

// The action the button does: the one picked last, kept from one run to
// the next only with Preferences' "Remember last print action" (prefs.js);
// "Print" only where it can send, as the reference does not bring back a
// send it cannot do.
function printAction() {
  const picked = Prefs.printAction();
  if (picked === 'send' && canSend()[0]) return 'send';
  if (picked === 'export' || picked === 'sliced' || picked === 'all_sliced') return picked;
  if ((picked === 'bambu_connect' || picked === 'bambu_print') && isBambu()) return picked;
  if (canSend()[0]) return 'send';
  // A Bambu Lab printer prints through the network plug-in ("Print plate").
  return isBambu() ? 'bambu_print' : 'export';
}

$('export-more').addEventListener('click', (event) => {
  const box = event.currentTarget.getBoundingClientRect();
  const action = printAction();
  const choose = (what) => () => { Prefs.choosePrintAction(what); showButtons(); };
  const [can, why] = canSend();
  showMenu(box.right - 220, box.bottom + 4, [
    // A Bambu Lab printer: "Print plate" through UltraNet (bambu.js).
    ...(isBambu() ? [[t('Print plate'), choose('bambu_print'), { checked: action === 'bambu_print' }]] : []),
    [t('Print'), choose('send'), { checked: action === 'send', title: can ? 'Send the plate to the printer' : why }],
    // MainFrame.cpp `available_print_actions`: a printer not Bambu Lab's
    // is offered both sliced files as well.
    [t('Export plate sliced file'), choose('sliced'), { checked: action === 'sliced' }],
    [t('Export all sliced file'), choose('all_sliced'), { checked: action === 'all_sliced' }],
    [t('Export G-code file'), choose('export'), { checked: action === 'export' }],
    // MainFrame.cpp l.2581: a Bambu Lab printer's sliced file to Bambu
    // Connect (menu.js `sendToBambuConnect`).
    ...(isBambu() ? [[t('Send to Bambu Connect'), choose('bambu_connect'), { checked: action === 'bambu_connect' }]] : []),
    null,
    [t('Print host upload queue'), () => attempt(openQueue)],
  ]);
});

// ---- "Send G-code to printer host" ----

const SPECIAL = /[#*;\\/:"<>|]/;

async function openSend() {
  const project = state.project;
  if (!project) return;
  await refreshPrintHost();
  const [can, why] = canSend();
  if (!can) {
    say(why, true);
    return;
  }
  const info = hostsState.info;
  const { name } = await call('print_send_name', { project: project.id, plate: project.plate ?? undefined });
  const field = $('send-name');
  field.value = name;
  // The unload box: Snapmaker's tool changers, ticked as the preset says.
  const unload = info.offers.unload;
  $('send-unload-row').hidden = !unload;
  $('send-unload').checked = Boolean(unload && unload.default);
  // The Centauri Carbon's options, as last chosen; the plate side as the
  // project's bed type says (the smooth side is the Cool Plate).
  const elegoo = info.offers.elegoo;
  $('send-elegoo').hidden = !elegoo;
  const bed = state.settled && state.settled.printer ? state.settled.printer.bed_types.in_force : '';
  const fileSide = bed === 'Cool Plate' ? 'b' : 'a';
  if (elegoo) {
    $('send-timelapse').checked = remembered.get('elegoolink_timelapse') === '1';
    $('send-leveling').checked = remembered.get('elegoolink_heated_bed_leveling') === '1';
    $(`send-plate-${fileSide}`).checked = true;
  }
  const plateWarn = () => {
    const side = $('send-plate-b').checked ? 'b' : 'a';
    $('send-plate-warn').hidden = side === fileSide;
  };
  $('send-plate-a').onchange = plateWarn;
  $('send-plate-b').onchange = plateWarn;
  plateWarn();
  // A U1: the toolheads for "Upload and Print", read from the printer
  // while the dialog is open; the preferences off, as its page starts them.
  const u1 = Boolean(info.toolheads && info.can_start);
  $('send-u1').hidden = !u1;
  hostsState.toolheads = null;
  hostsState.mapping = [];
  for (const id of ['send-bed-level', 'send-flow', 'send-time-lapse']) $(id).checked = false;
  if (u1) loadToolheads(project.id, project.plate ?? undefined);
  $('send-device').checked = remembered.get('open_device_tab_post_upload') !== '0';
  $('send-print').hidden = !info.can_start;
  $('send-to').textContent = `To ${info.printer}, at ${info.connection.print_host}.`;

  const dialog = $('send-dialog');
  dialog.returnValue = 'cancel';
  const closed = new Promise((resolve) => dialog.addEventListener('close', resolve, { once: true }));
  dialog.showModal();
  // The name's stem selected, as the reference selects it.
  const dot = field.value.lastIndexOf('.');
  field.setSelectionRange(0, dot > 0 ? dot : field.value.length);
  field.focus();
  await closed;
}

// ---- a U1's toolheads ----

const swatch = (color) => el('span', { className: 'swatch', style: color ? `background:${color}` : '' });

async function loadToolheads(project, plate, mapping) {
  const note = $('send-map-note');
  if (!mapping) {
    note.className = 'quiet';
    note.textContent = 'Reading what the toolheads hold…';
    $('send-map').replaceChildren();
  }
  let answer;
  try {
    answer = await call('printer_toolheads', { project, plate, mapping });
  } catch (error) {
    hostsState.toolheads = null;
    note.className = 'error';
    note.textContent = `What the toolheads hold could not be read: ${error.message}`;
    return;
  }
  hostsState.toolheads = answer;
  hostsState.mapping = answer.mapping;
  showToolheads();
}

function showToolheads() {
  const answer = hostsState.toolheads;
  if (!answer) return;
  const heads = answer.toolheads;
  const label = (head) => `${head.index + 1}: ${head.loaded ? [head.vendor, head.type, head.sub_type].filter(Boolean).join(' ') || '?' : t('Empty')}`;
  $('send-map').replaceChildren(...answer.filaments.filter((filament) => filament.used).map((filament) => {
    const select = el('select', { ariaLabel: `Toolhead for filament ${filament.index + 1}` });
    fill(select, heads.map((head) => String(head.index)), String(hostsState.mapping[filament.index]), heads.map(label));
    select.addEventListener('change', () => {
      hostsState.mapping = hostsState.mapping.slice();
      hostsState.mapping[filament.index] = Number(select.value);
      const project = state.project;
      attempt(() => loadToolheads(project.id, project.plate ?? undefined, hostsState.mapping));
    });
    const head = heads[hostsState.mapping[filament.index]];
    return el('tr', {},
      el('td', {}, swatch(filament.color), ` ${filament.index + 1} ${filament.type}`),
      el('td', { textContent: '→' }),
      el('td', {}, head ? swatch(head.loaded ? head.color : '') : '', ' ', select));
  }));
  const note = $('send-map-note');
  const lines = [...answer.warnings];
  if (answer.problem) lines.unshift(answer.problem);
  if (answer.state === 'printing' || answer.state === 'paused') lines.unshift(`${answer.printer} is ${answer.state === 'paused' ? 'paused mid-print' : 'printing'}.`);
  note.className = answer.problem ? 'warn' : 'quiet';
  note.textContent = lines.join('\n') || 'Each filament on the toolhead nearest its colour; change any before "Upload and Print".';
}

// The reference's two questions about the name, asked before it is sent.
async function nameAccepted(name) {
  if (SPECIAL.test(name)) {
    const yes = await confirmBox('EdgeSlicerRS', fmt(t('The filename \'%s\' contains special characters (#, *, ;, \\, /, :, ", <, >, or |) which may cause issues.Do you wish to continue?'), name), t('Yes'), t('No'));
    if (!yes) return false;
  }
  if (!name.toLowerCase().endsWith('.gcode')) {
    return confirmBox('EdgeSlicerRS', fmt(t('Upload filename doesn\'t end with "%s". Do you wish to continue?'), '.gcode'), t('Yes'), t('No'));
  }
  return true;
}

$('send-form').addEventListener('submit', (event) => {
  const how = event.submitter ? event.submitter.value : 'upload';
  if (how === 'cancel') return;
  event.preventDefault();
  attempt(async () => {
    const name = $('send-name').value.trim();
    if (!name) return;
    if (!(await nameAccepted(name))) return;
    const start = how === 'print';
    const options = {
      unload_at_end: !$('send-unload-row').hidden && $('send-unload').checked,
      timelapse: !$('send-elegoo').hidden && $('send-timelapse').checked,
      leveling: !$('send-elegoo').hidden && $('send-leveling').checked,
      smooth_plate: !$('send-elegoo').hidden && $('send-plate-b').checked,
    };
    if (!$('send-elegoo').hidden) {
      remembered.set('elegoolink_timelapse', options.timelapse ? '1' : '0');
      remembered.set('elegoolink_heated_bed_leveling', options.leveling ? '1' : '0');
      // The plate side too, as the reference keeps its `bedType`: a start
      // from Storage uses the last send's choices.
      remembered.set('elegoolink_smooth_plate', options.smooth_plate ? '1' : '0');
    }
    if (start && !$('send-u1').hidden) {
      options.mapping = hostsState.mapping.length ? hostsState.mapping : undefined;
      options.bed_level = $('send-bed-level').checked;
      options.flow_calibrate = $('send-flow').checked;
      options.time_lapse = $('send-time-lapse').checked;
      // An empty toolhead: the reference refuses it unless told to go on.
      const problem = hostsState.toolheads && hostsState.toolheads.problem;
      if (problem && / is empty/.test(problem)) {
        if (!(await confirmBox(t('Upload and Print'), `${problem}\nSend it all the same?`, t('Upload and Print'), t('Cancel')))) return;
        options.force = true;
      }
    }
    const toDevice = $('send-device').checked;
    remembered.set('open_device_tab_post_upload', toDevice ? '1' : '0');
    const project = state.project;
    const sent = await call('print_send', {
      project: project.id,
      plate: project.plate ?? undefined,
      name,
      start,
      options,
    });
    $('send-dialog').close('ok');
    hostsState.watching.add(sent.id);
    hostsState.queue = sent.queue;
    say(`${start ? 'Uploading and printing' : 'Uploading'} ${name}…`);
    watchQueue();
    if (toDevice) await selectTab('device');
    else await openQueue();
  });
});

// ---- "Print host upload queue" ----

const sizeText = (bytes) => (bytes >= 1e6 ? `${number(bytes / 1e6, 2)} MB` : `${number(bytes / 1e3, 1)} kB`);

function renderQueue() {
  const jobs = hostsState.queue.jobs || [];
  $('queue-none').hidden = jobs.length > 0;
  $('queue-table').hidden = jobs.length === 0;
  $('queue-body').replaceChildren(...jobs.map((job) => {
    const bar = el('span', { className: 'queue-bar' }, el('span', { style: `width:${job.progress}%` }));
    const progress = el('td', {}, bar, el('span', { className: 'num', textContent: ` ${job.progress}%` }));
    const status = job.stage && job.state === 'uploading' && job.progress >= 99 ? job.stage : job.said;
    const row = el('tr', { className: `queue-${job.state}${hostsState.selected === job.id ? ' on' : ''}` },
      el('td', { className: 'num', textContent: String(job.id) }),
      progress,
      el('td', { textContent: status }),
      el('td', { className: 'cut', textContent: job.host, title: job.host }),
      el('td', { className: 'num', textContent: sizeText(job.size) }),
      el('td', { className: 'cut', textContent: job.name, title: job.name }),
      el('td', { className: 'cut', textContent: job.message.replace(/\n/g, ' '), title: job.message }));
    row.tabIndex = 0;
    row.addEventListener('click', () => { hostsState.selected = job.id; renderQueue(); });
    row.addEventListener('keydown', (event) => { if (event.key === 'Enter' || event.key === ' ') { hostsState.selected = job.id; renderQueue(); } });
    return row;
  }));
  const chosen = jobs.find((job) => job.id === hostsState.selected);
  $('queue-cancel').disabled = !chosen || !['queued', 'uploading'].includes(chosen.state);
  $('queue-error').disabled = !chosen || chosen.state !== 'error';
}

async function openQueue() {
  hostsState.queue = await call('print_queue');
  renderQueue();
  const dialog = $('queue-dialog');
  if (!dialog.open) dialog.showModal();
  watchQueue();
}

// Asks how the uploads go while any is going or the queue is open; says
// how each this page sent has ended, once.
function watchQueue() {
  clearTimeout(hostsState.queueTimer);
  hostsState.queueTimer = setTimeout(async () => {
    try {
      hostsState.queue = await call('print_queue');
    } catch {
      return;
    }
    for (const job of hostsState.queue.jobs) {
      if (!hostsState.watching.has(job.id) || !['completed', 'error', 'canceled'].includes(job.state)) continue;
      hostsState.watching.delete(job.id);
      if (job.state === 'completed') say(job.start ? `${job.name} was uploaded to ${job.host} and the print was started.` : `${job.name} was uploaded to ${job.host}.`);
      if (job.state === 'canceled') say(`The upload of ${job.name} was canceled.`);
      if (job.state === 'error') {
        say(job.message.replace(/\n/g, ' '), true);
        if (!$('queue-dialog').open) inform(t('Error'), job.message);
      }
      if (state.tab === 'device' && hostsState.deviceTab === 'storage') showStorage();
    }
    if ($('queue-dialog').open) renderQueue();
    if (hostsState.queue.busy || $('queue-dialog').open || hostsState.watching.size) watchQueue();
  }, 400);
}

$('queue-close').addEventListener('click', () => $('queue-dialog').close());
$('queue-cancel').addEventListener('click', () => attempt(async () => {
  if (hostsState.selected === null) return;
  hostsState.queue = await call('print_cancel', { id: hostsState.selected });
  renderQueue();
}));
$('queue-error').addEventListener('click', () => {
  const job = hostsState.queue.jobs.find((entry) => entry.id === hostsState.selected);
  if (job) inform(t('Error'), job.message);
});

// ---- Device: the printer's page, and its files ----

// Called by `showDevice` (app.js): the printer's Device page. A family
// with a native page (Bambu through UltraNet, a U1, Klipper, OctoPrint:
// device.js) has "Status" first, Bambu "HMS" and "Update" too; a print
// host keeps its own page ("Printer page") and its files ("Storage"). The
// frame is pointed at the printer only here, when Device is looked at.
const DEVICE_TABS = ['status', 'storage', 'hms', 'update', 'page'];

function showPrinterPage() {
  const info = hostsState.info;
  const bambu = isBambu() && typeof showBambuDevice === 'function';
  if (bambu) showBambuDevice();
  else if (typeof hideBambuDevice === 'function') hideBambuDevice();
  const url = !bambu && info && info.printer ? info.device_url : null;
  const native = bambu || Boolean(url && info.native);
  const has = { status: native, storage: Boolean(url) || bambu, hms: bambu, update: bambu, page: Boolean(url) };
  const shown = bambu || Boolean(url);
  hostsState.native = native;
  $('device-bar').hidden = !shown;
  $('device-view').hidden = !shown;
  $('device-page').hidden = shown;
  $('device-setup').hidden = !(info && info.printer) || shown;
  if (!shown) {
    hostsState.frameUrl = '';
    $('device-frame').removeAttribute('src');
    if (typeof stopDevice === 'function') stopDevice();
    return;
  }
  if (!has[hostsState.deviceTab]) hostsState.deviceTab = native ? 'status' : 'page';
  $('device-address').textContent = url || '';
  $('device-address').title = url ? (info.from === 'kept' ? 'Kept for this printer preset' : 'As the printer preset gives it') : '';
  $('device-reload').hidden = !url;
  $('device-connect').hidden = !url;
  $('device-status').hidden = !native;
  for (const name of DEVICE_TABS) {
    const tab = $(`device-tab-${name}`);
    tab.hidden = !has[name];
    tab.setAttribute('aria-selected', String(hostsState.deviceTab === name));
  }
  const panes = { status: 'device-native', storage: 'device-storage', hms: 'device-hms', update: 'device-update', page: 'device-frame' };
  for (const [name, id] of Object.entries(panes)) $(id).hidden = hostsState.deviceTab !== name;
  // Nothing is asked of the printer until Device is looked at.
  if (state.tab !== 'device') return;
  if (hostsState.deviceTab === 'page' && hostsState.frameUrl !== url) {
    if (!framable(url)) {
      $('device-frame').removeAttribute('src');
      say('The printer\'s page is shown after the page is loaded again.', true);
      return;
    }
    hostsState.frameUrl = url;
    $('device-frame').src = url;
  }
  if (hostsState.deviceTab === 'storage') showStorage();
  if (native && typeof watchDevice === 'function') watchDevice(true);
}

for (const name of DEVICE_TABS) $(`device-tab-${name}`).addEventListener('click', () => { hostsState.deviceTab = name; showPrinterPage(); });
$('device-reload').addEventListener('click', () => {
  if (hostsState.deviceTab === 'storage') {
    showStorage();
    return;
  }
  const url = hostsState.frameUrl;
  if (!url) return;
  $('device-frame').src = 'about:blank';
  setTimeout(() => { $('device-frame').src = url; }, 50);
});
$('storage-refresh').addEventListener('click', () => showStorage());

let storageTurn = 0;
// The files as last listed, shown in the order "Sort by" says; the one
// selected (its name); whether an action is under way.
let storageFiles = [];
let storageSelected = '';
let storageBusy = false;
// The preview pictures, fetched by the session (the printer's key stays
// there) and handed over as data: by their path on the printer, `null`
// for one that could not be had. Pictures of files gone are dropped.
const storagePictures = new Map();
let storagePictureTurn = 0;

// PrinterStorage.cpp `format_duration`, `format_filament`.
function storageDuration(seconds) {
  if (!seconds || seconds <= 0) return '';
  if (seconds < 60) return '< 1m';
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days) return `${days}d ${hours}h`;
  if (hours) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

function storageFilament(file) {
  const parts = [];
  if (file.filament_mm) parts.push(`${(file.filament_mm / 1000).toFixed(2)} m`);
  if (file.filament_g) parts.push(file.filament_g < 10 ? `${file.filament_g.toFixed(1)} g` : `${file.filament_g.toFixed(0)} g`);
  return parts.join(', ');
}

// "PLA;PLA;PETG" -> "PLA, PETG" (`storage_types_text`).
function storageTypes(types) {
  const seen = [];
  for (const one of (types || '').split(';')) {
    const type = one.replace(/^[ "]+|[ "]+$/g, '');
    if (type && !seen.includes(type)) seen.push(type);
  }
  return seen.join(', ');
}

// `storage_modified_text`: "%Y-%m-%d %H:%M".
function storageModified(seconds) {
  if (!seconds || seconds <= 0) return '';
  const d = new Date(seconds * 1000);
  const two = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}`;
}

function storageNote(text, kind = 'quiet') {
  const note = $('storage-note');
  note.className = kind;
  note.textContent = text;
}

async function showStorage() {
  if (storageBusy) return;
  const turn = ++storageTurn;
  const info = hostsState.info;
  $('storage-title').textContent = info ? fmt(t('Files on %s'), info.printer) : t('Files on the printer');
  $('storage-kind').hidden = !isBambu();
  if (isBambu()) {
    showBambuStorage(turn);
    return;
  }
  if (info && !info.storage) {
    storageFiles = [];
    renderStorage();
    storageNote(`${t('Storage is not available for this printer.')}\n${info.storage_reason || ''}`.trim());
    return;
  }
  storageNote(t('Reading the printer\'s files...'));
  let files;
  try {
    ({ files } = await call('printer_files', { project: activeId() }));
  } catch (error) {
    if (turn !== storageTurn) return;
    storageFiles = [];
    renderStorage();
    storageNote(`${t('The printer\'s files could not be listed.')}\n${error.message}`, 'error');
    return;
  }
  if (turn !== storageTurn) return;
  storageFiles = files;
  const listed = new Set(files.map((file) => file.thumbnail).filter(Boolean));
  for (const path of [...storagePictures.keys()]) if (!listed.has(path)) storagePictures.delete(path);
  storageNote(files.length ? '' : t('There are no G-code files on the printer.'));
  renderStorage();
  loadStoragePictures();
}

// A Bambu Lab printer's card (MediaFilePanel.cpp), through the bridge's
// proposed `files_list` (ULTRANET_BRIDGE.md section 15): its timelapses,
// videos or models, listed and deleted; a bridge of protocol 1.0 says
// why it cannot.
async function showBambuStorage(turn) {
  $('storage-title').textContent = t('Storage');
  storageNote(t('Loading file list...'));
  let files;
  try {
    ({ files } = await call('device_files', { project: activeId(), kind: $('storage-kind').value }));
  } catch (error) {
    if (turn !== storageTurn) return;
    storageFiles = [];
    renderStorage();
    storageNote(error.message, 'error');
    return;
  }
  if (turn !== storageTurn) return;
  storageFiles = files.map((file) => ({
    name: file.name, path: file.path, size: file.size, modified: file.time, printable: false, bambu: true,
  }));
  storageNote(files.length ? '' : t('No files'));
  renderStorage();
}

$('storage-kind').addEventListener('change', () => showStorage());

function storageSorted() {
  const order = $('storage-sort').value;
  return storageFiles.slice().sort((a, b) => {
    if (order === 'oldest') return (a.modified || 0) - (b.modified || 0);
    if (order === 'name') return a.name.localeCompare(b.name);
    if (order === 'size') return (b.size || 0) - (a.size || 0);
    return (b.modified || 0) - (a.modified || 0);
  });
}

function storagePicture(file, className) {
  const data = file.thumbnail ? storagePictures.get(file.thumbnail) : null;
  const img = el('img', { alt: '', className });
  if (data) img.src = data;
  else img.hidden = true;
  return img;
}

function renderStorage() {
  const files = storageSorted();
  // The selection stays on the same file; with none, the first one, so
  // its details and Print / Delete are there without a click.
  if (!files.some((file) => file.name === storageSelected)) storageSelected = files.length ? files[0].name : '';
  $('storage-table').hidden = !files.length;
  $('storage-body').replaceChildren(...files.map((file) => {
    const row = el('tr', { className: file.name === storageSelected ? 'on' : '' },
      el('td', { className: 'storage-pic' }, storagePicture(file)),
      el('td', { className: 'cut', textContent: file.name, title: file.name }),
      el('td', { className: 'num', textContent: storageDuration(file.print_time) }),
      el('td', { className: 'cut', textContent: storageFilament(file), title: storageTypes(file.filament_type) }),
      el('td', { className: 'num', textContent: file.size === null ? '' : sizeText(file.size) }),
      el('td', { textContent: storageModified(file.modified) }));
    row.dataset.name = file.name;
    row.setAttribute('aria-selected', String(file.name === storageSelected));
    row.addEventListener('click', () => storageSelect(file.name));
    row.addEventListener('dblclick', () => attempt(() => storagePrint()));
    return row;
  }));
  renderStorageDetails();
}

function storageSelect(name) {
  storageSelected = name;
  for (const row of $('storage-body').children) {
    const on = row.dataset.name === name;
    row.classList.toggle('on', on);
    row.setAttribute('aria-selected', String(on));
    if (on) row.scrollIntoView({ block: 'nearest' });
  }
  renderStorageDetails();
}

// `update_details`: the picture, the name, what the slicer wrote into it.
function renderStorageDetails() {
  const file = storageFiles.find((one) => one.name === storageSelected);
  $('storage-details').hidden = !storageFiles.length;
  if (!file) {
    $('storage-detail-name').textContent = storageFiles.length ? t('Select a file to see its details.') : '';
    $('storage-detail-text').textContent = '';
    $('storage-preview').hidden = true;
    $('storage-print').disabled = true;
    $('storage-delete').disabled = true;
    return;
  }
  const slash = file.name.lastIndexOf('/');
  $('storage-detail-name').textContent = file.name.slice(slash + 1);
  const lines = [];
  const line = (label, value) => { if (value) lines.push(`${label}: ${value}`); };
  if (slash >= 0) line(t('Folder'), file.name.slice(0, slash));
  line(t('Print time'), storageDuration(file.print_time));
  line(t('Filament'), storageFilament(file));
  line(t('Filament type'), storageTypes(file.filament_type));
  if (file.layer_count) line(t('Layers'), String(file.layer_count));
  if (file.layer_height) line(t('Layer height'), `${file.layer_height.toFixed(2)} mm`);
  if (file.object_height) line(t('Height'), `${file.object_height.toFixed(1)} mm`);
  if (file.nozzle_diameter) line(t('Nozzle'), `${file.nozzle_diameter.toFixed(2)} mm`);
  line(t('Size'), file.size === null ? '' : sizeText(file.size));
  line(t('Modified'), storageModified(file.modified));
  line(t('Sliced with'), file.slicer);
  if (!file.printable) lines.push(t('This file type cannot be started from here.'));
  $('storage-print').hidden = Boolean(file.bambu);
  $('storage-detail-text').textContent = lines.join('\n');
  const picture = file.thumbnail ? storagePictures.get(file.thumbnail) : null;
  $('storage-preview').hidden = !picture;
  if (picture) $('storage-preview').src = picture;
  else $('storage-preview').removeAttribute('src');
  $('storage-print').disabled = storageBusy || !file.printable;
  $('storage-delete').disabled = storageBusy;
}

// The pictures, one at a time: the selected file's first, then the rows in
// the order shown (the reference fetches the visible rows and a margin).
async function loadStoragePictures() {
  const turn = ++storagePictureTurn;
  const files = storageSorted();
  const selected = files.find((file) => file.name === storageSelected);
  const order = selected ? [selected, ...files.filter((file) => file !== selected)] : files;
  for (const file of order.slice(0, 60)) {
    if (!file.thumbnail || storagePictures.has(file.thumbnail)) continue;
    let picture = null;
    try {
      ({ picture } = await call('printer_file_thumbnail', { project: activeId(), path: file.thumbnail }));
    } catch {
      picture = null;
    }
    if (turn !== storagePictureTurn) return;
    storagePictures.set(file.thumbnail, picture);
    for (const row of $('storage-body').children) {
      if (row.dataset.name === file.name && picture) {
        const img = row.querySelector('.storage-pic img');
        img.src = picture;
        img.hidden = false;
      }
    }
    if (file.name === storageSelected) renderStorageDetails();
  }
}

function storageSetBusy(busy) {
  storageBusy = busy;
  $('storage-refresh').disabled = busy;
  renderStorageDetails();
}

$('storage-print').addEventListener('click', () => attempt(() => storagePrint()));
$('storage-delete').addEventListener('click', () => attempt(() => storageDelete()));
// Enter prints, Delete deletes, the arrows move the selection, while the
// list has the focus; F5 lists again anywhere in the tab.
$('storage-table').addEventListener('keydown', (event) => {
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  const files = storageSorted();
  const at = files.findIndex((file) => file.name === storageSelected);
  if (event.key === 'Enter') {
    event.preventDefault();
    attempt(() => storagePrint());
  } else if (event.key === 'Delete') {
    event.preventDefault();
    attempt(() => storageDelete());
  } else if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && files.length) {
    event.preventDefault();
    const next = Math.max(0, Math.min(files.length - 1, at + (event.key === 'ArrowDown' ? 1 : -1)));
    storageSelect(files[next].name);
  }
});
$('device-storage').addEventListener('keydown', (event) => {
  if (event.key !== 'F5') return;
  event.preventDefault();
  if (!storageBusy) showStorage();
});
document.addEventListener('keydown', (event) => {
  if (event.key !== 'F5' || $('device-storage').hidden || state.tab !== 'device') return;
  if ($('device-storage').contains(document.activeElement)) return;
  // A dialog open over the tab keeps the key.
  if (document.querySelector('dialog[open]')) return;
  event.preventDefault();
  if (!storageBusy) showStorage();
});

// PrinterStoragePanel.cpp `on_delete`: the selected file.
async function storageDelete() {
  const file = storageFiles.find((one) => one.name === storageSelected);
  if (storageBusy || !file) return;
  if (file.bambu) {
    await bambuStorageDelete(file);
    return;
  }
  const printer = hostsState.info ? hostsState.info.printer : '';
  const yes = await confirmBox(t('Delete file'), fmt(t('Delete "%s" from %s?\nThis cannot be undone.'), file.name, printer), t('Delete'), t('Cancel'));
  if (!yes) return;
  storageSetBusy(true);
  storageNote(fmt(t('Deleting %s...'), file.name));
  let answer;
  try {
    answer = await call('printer_file_delete', { project: activeId(), name: file.name });
  } catch (error) {
    storageSetBusy(false);
    storageNote('');
    await inform(t('Delete file'), `${fmt(t('Could not delete "%s".'), file.name)}\n\n${error.message}`);
    return;
  }
  storageSetBusy(false);
  storageFiles = storageFiles.filter((one) => one.name !== file.name);
  renderStorage();
  storageNote(fmt(answer.gone ? t('%s was already gone from the printer.') : t('Deleted %s.'), file.name));
}

// MediaFilePanel.cpp l.580: the session asks, the file is deleted only
// once confirmed.
async function bambuStorageDelete(file) {
  const asked = await call('device_file_delete', { project: activeId(), path: file.path });
  if (!asked.confirm) return;
  if (!await confirmBox(asked.confirm.title, asked.confirm.lines.join('\n\n'), asked.confirm.ok, t('Cancel'))) return;
  storageSetBusy(true);
  storageNote(fmt(t('Deleting %s...'), file.name));
  try {
    await call('device_file_delete', { project: activeId(), path: file.path, confirmed: true });
  } catch (error) {
    storageSetBusy(false);
    storageNote('');
    await inform(t('Delete file'), `${fmt(t('Could not delete "%s".'), file.name)}\n\n${error.message}`);
    return;
  }
  storageSetBusy(false);
  storageFiles = storageFiles.filter((one) => one !== file);
  renderStorage();
  storageNote(fmt(t('Deleted %s.'), file.name));
}

// PrinterStoragePanel.cpp `on_print`: what the printer is doing first, a
// print never over a running or paused job; then the question with what
// the file holds; the printer is asked again just before it starts.
// The Centauri Carbon's choices of the last send, for a start from Storage
// (the reference's `start_options`); other printers ignore them.
function storageStartOptions() {
  return {
    timelapse: remembered.get('elegoolink_timelapse') === '1',
    leveling: remembered.get('elegoolink_heated_bed_leveling') === '1',
    smooth_plate: remembered.get('elegoolink_smooth_plate') === '1',
  };
}

async function storagePrint() {
  const file = storageFiles.find((one) => one.name === storageSelected);
  if (storageBusy || !file || !file.printable) return;
  const printer = hostsState.info ? hostsState.info.printer : '';
  storageSetBusy(true);
  storageNote(t('Asking the printer whether it is free...'));
  const options = storageStartOptions();
  let check;
  try {
    check = await call('printer_file_check', { project: activeId(), name: file.name, options });
  } catch (error) {
    storageSetBusy(false);
    storageNote('');
    await inform(t('Print'), `${t('The printer could not be asked whether it is free.')}\n\n${error.message}`);
    return;
  }
  storageSetBusy(false);
  storageNote('');
  if (check.busy) {
    const what = check.state === 'paused' ? t('The printer has a paused print.')
      : check.state === 'printing' ? t('The printer is printing.') : t('The printer is busy.');
    await inform(t('Print'), `${what}${check.current ? ` (${check.current})` : ''}\n${t('Finish or cancel that job before starting another one.')}`);
    return;
  }
  let details = '';
  if (file.print_time) details += `${t('Print time')}: ${storageDuration(file.print_time)}\n`;
  if (storageFilament(file)) details += `${t('Filament')}: ${storageFilament(file)}${file.filament_type ? ` (${storageTypes(file.filament_type)})` : ''}\n`;
  // What this printer adds before a start (English, as the reference's
  // printer messages on this tab): a Centauri Carbon's choices and CANVAS.
  if (check.advice) details += `${details ? '\n' : ''}${check.advice}\n\n`;
  const question = `${fmt(t('Start printing "%s" on %s?'), file.name, printer)}\n\n${details}${t('The file prints exactly as it was sliced. Make sure the build plate is clear and the filament the file was sliced for is loaded.')}`;
  if (!(await confirmBox(t('Print'), question, t('Print'), t('Cancel')))) return;
  storageSetBusy(true);
  storageNote(fmt(t('Starting %s...'), file.name));
  try {
    await call('printer_file_print', { project: activeId(), name: file.name, options });
  } catch (error) {
    storageSetBusy(false);
    storageNote('');
    await inform(t('Print'), `${fmt(t('The print of "%s" did not start.'), file.name)}\n\n${error.message}`);
    return;
  }
  storageSetBusy(false);
  storageNote(fmt(t('Print started: %s. Follow it on the printer page.'), file.name));
}

$('storage-sort').addEventListener('change', renderStorage);

// What was sent before the page was loaded again is still in the queue.
call('print_queue').then((queue) => {
  hostsState.queue = queue;
  if (queue.busy) watchQueue();
}, () => {});
