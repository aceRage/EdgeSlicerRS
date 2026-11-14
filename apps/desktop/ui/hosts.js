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
  deviceTab: 'page',
  frameUrl: '',
  // A U1's toolheads as last read for the send dialog, the mapping shown,
  // and the Device tab's watch of the printer's state.
  toolheads: null,
  mapping: [],
  statusTimer: null,
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

const PRINT_ACTIONS = { send: 'Print', sliced: 'Export plate sliced file', all_sliced: 'Export all sliced file', export: 'Export G-code file', bambu_connect: 'Send to Bambu Connect' };

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
  if (picked === 'bambu_connect' && isBambu()) return picked;
  return canSend()[0] ? 'send' : 'export';
}

$('export-more').addEventListener('click', (event) => {
  const box = event.currentTarget.getBoundingClientRect();
  const action = printAction();
  const choose = (what) => () => { Prefs.choosePrintAction(what); showButtons(); };
  const [can, why] = canSend();
  showMenu(box.right - 220, box.bottom + 4, [
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

// Called by `showDevice` (app.js): with a connection, the printer's page
// and its files in place of what the preset says of it. The frame is
// pointed at the printer only here, when Device is looked at.
function showPrinterPage() {
  const info = hostsState.info;
  const url = info && info.printer ? info.device_url : null;
  $('device-bar').hidden = !url;
  $('device-view').hidden = !url;
  $('device-page').hidden = Boolean(url);
  $('device-setup').hidden = !(info && info.printer) || Boolean(url);
  if (!url) {
    hostsState.frameUrl = '';
    $('device-frame').removeAttribute('src');
    return;
  }
  $('device-address').textContent = url;
  $('device-address').title = info.from === 'kept' ? 'Kept for this printer preset' : 'As the printer preset gives it';
  // Shown for every printer page, with the reference's reason where it has no storage.
  $('device-tab-storage').hidden = false;

  for (const [tab, name] of [[$('device-tab-page'), 'page'], [$('device-tab-storage'), 'storage']]) {
    tab.setAttribute('aria-selected', String(hostsState.deviceTab === name));
  }
  $('device-frame').hidden = hostsState.deviceTab !== 'page';
  $('device-storage').hidden = hostsState.deviceTab !== 'storage';
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
  watchStatus();
}

// The printer's state over the page, asked every few seconds while
// Device is looked at (the reference's phone hub and its Printers tab ask
// Moonraker the same way, SnapmakerLan.cpp `status`).
const clock = (seconds) => {
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return h ? `${h}h${m}m` : `${m}m`;
};

function watchStatus() {
  clearTimeout(hostsState.statusTimer);
  const info = hostsState.info;
  const shown = state.tab === 'device' && info && info.printer && info.status;
  $('device-status').hidden = !shown;
  if (!shown) return;
  const project = activeId();
  hostsState.statusTimer = setTimeout(async () => {
    let status;
    try {
      status = await call('printer_status', { project });
    } catch (error) {
      status = { online: false, said: t('Offline'), error: error.message };
    }
    if (state.tab !== 'device' || activeId() !== project) return;
    const line = $('device-status');
    line.className = status.online ? `state-${status.state}` : 'error';
    if (!status.known && status.known !== undefined) {
      line.hidden = true;
      return;
    }
    if (!status.online) {
      line.textContent = status.said;
      line.title = status.error || '';
    } else {
      const parts = [status.said];
      if (status.filename) parts.push(status.filename);
      if (status.state === 'printing' || status.state === 'paused' || status.state === 'complete') parts.push(`${status.percent}%`);
      if (status.left) parts.push(`${clock(status.left)} left`);
      if (status.total_layers) parts.push(`${t('Layer')} ${status.layer}/${status.total_layers}`);
      if (status.bed) parts.push(`${t('Bed')} ${Math.round(status.bed[0])}/${Math.round(status.bed[1])} °C`);
      // The toolheads being heated, with their number; else the first.
      const nozzles = (status.nozzles || []).map((n, i) => [i, n]).filter(([, n]) => n[1] > 0);
      const shown = nozzles.length ? nozzles : (status.nozzles || []).slice(0, 1).map((n) => [0, n]);
      for (const [i, n] of shown) parts.push(`${status.nozzles.length > 1 ? `T${i + 1}` : t('Nozzle')} ${Math.round(n[0])}/${Math.round(n[1])} °C`);
      line.textContent = parts.join(' · ');
      line.title = status.message || '';
    }
    watchStatus();
  }, $('device-status').textContent ? 3000 : 50);
}

$('device-tab-page').addEventListener('click', () => { hostsState.deviceTab = 'page'; showPrinterPage(); });
$('device-tab-storage').addEventListener('click', () => { hostsState.deviceTab = 'storage'; showPrinterPage(); });
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
// The files as last listed, shown in the order "Sort by" says.
let storageFiles = [];

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

function storageNote(text, kind = 'quiet') {
  const note = $('storage-note');
  note.className = kind;
  note.textContent = text;
}

async function showStorage() {
  const turn = ++storageTurn;
  const info = hostsState.info;
  $('storage-title').textContent = info ? fmt(t('Files on %s'), info.printer) : t('Files on the printer');
  if (info && !info.storage) {
    $('storage-body').replaceChildren();
    storageNote(info.storage_reason || t('Storage is not available for this printer.'));
    return;
  }
  storageNote(t('Reading the printer\'s files...'));
  try {
    ({ files: storageFiles } = await call('printer_files', { project: activeId() }));
  } catch (error) {
    if (turn !== storageTurn) return;
    $('storage-body').replaceChildren();
    storageNote(`${t('The printer\'s files could not be listed.')}\n${error.message}`, 'error');
    return;
  }
  if (turn !== storageTurn) return;
  storageNote(storageFiles.length ? '' : t('There are no G-code files on the printer.'));
  renderStorage();
}

function renderStorage() {
  const order = $('storage-sort').value;
  const files = storageFiles.slice().sort((a, b) => {
    if (order === 'oldest') return (a.modified || 0) - (b.modified || 0);
    if (order === 'name') return a.name.localeCompare(b.name);
    if (order === 'size') return (b.size || 0) - (a.size || 0);
    return (b.modified || 0) - (a.modified || 0);
  });
  const printer = hostsState.info ? hostsState.info.printer : '';
  $('storage-body').replaceChildren(...files.map((file) => {
    const print = el('button', { type: 'button', textContent: t('Print'), title: t('Start printing this file on the printer (Enter)'), disabled: !file.printable });
    const remove = el('button', { type: 'button', textContent: t('Delete'), title: t('Delete this file from the printer (Delete)') });
    print.addEventListener('click', () => attempt(() => storagePrint(file, printer)));
    remove.addEventListener('click', () => attempt(() => storageDelete(file, printer)));
    const modified = file.modified ? new Date(file.modified * 1000).toLocaleString() : '';
    const types = file.filament_type ? ` (${file.filament_type.split(';').filter(Boolean).join(', ')})` : '';
    return el('tr', {},
      el('td', { className: 'cut', textContent: file.name, title: file.name }),
      el('td', { className: 'num', textContent: storageDuration(file.print_time) }),
      el('td', { className: 'cut', textContent: storageFilament(file) + (storageFilament(file) ? types : ''), title: file.filament_type || '' }),
      el('td', { className: 'num', textContent: file.size === null ? '' : sizeText(file.size) }),
      el('td', { textContent: modified }),
      el('td', { className: 'storage-actions' }, print, remove));
  }));
}

// PrinterStoragePanel.cpp `on_delete`.
async function storageDelete(file, printer) {
  const yes = await confirmBox(t('Delete file'), fmt(t('Delete "%s" from %s?\nThis cannot be undone.'), file.name, printer), t('Delete'), t('Cancel'));
  if (!yes) return;
  storageNote(fmt(t('Deleting %s...'), file.name));
  let answer;
  try {
    answer = await call('printer_file_delete', { project: activeId(), name: file.name });
  } catch (error) {
    storageNote('');
    await inform(t('Delete file'), `${fmt(t('Could not delete "%s".'), file.name)}\n\n${error.message}`);
    return;
  }
  storageFiles = storageFiles.filter((one) => one.name !== file.name);
  renderStorage();
  storageNote(fmt(answer.gone ? t('%s was already gone from the printer.') : t('Deleted %s.'), file.name));
}

// PrinterStoragePanel.cpp `on_print`: what the printer is doing first, a
// print never over a running or paused job; then the question with what
// the file holds; the printer is asked again just before it starts.
async function storagePrint(file, printer) {
  storageNote(t('Asking the printer whether it is free...'));
  let check;
  try {
    check = await call('printer_file_check', { project: activeId(), name: file.name });
  } catch (error) {
    storageNote('');
    await inform(t('Print'), `${t('The printer could not be asked whether it is free.')}\n\n${error.message}`);
    return;
  }
  storageNote('');
  if (check.busy) {
    const what = check.state === 'paused' ? t('The printer has a paused print.') : t('The printer is printing.');
    await inform(t('Print'), `${what}${check.current ? ` (${check.current})` : ''}\n${t('Finish or cancel that job before starting another one.')}`);
    return;
  }
  let details = '';
  if (file.print_time) details += `${t('Print time')}: ${storageDuration(file.print_time)}\n`;
  if (storageFilament(file)) details += `${t('Filament')}: ${storageFilament(file)}${file.filament_type ? ` (${file.filament_type})` : ''}\n`;
  const question = `${fmt(t('Start printing "%s" on %s?'), file.name, printer)}\n\n${details}${t('The file prints exactly as it was sliced. Make sure the build plate is clear and the filament the file was sliced for is loaded.')}`;
  if (!(await confirmBox(t('Print'), question, t('Print'), t('Cancel')))) return;
  storageNote(fmt(t('Starting %s...'), file.name));
  try {
    await call('printer_file_print', { project: activeId(), name: file.name });
  } catch (error) {
    storageNote('');
    await inform(t('Print'), `${fmt(t('The print of "%s" did not start.'), file.name)}\n\n${error.message}`);
    return;
  }
  storageNote(fmt(t('Print started: %s. Follow it on the printer page.'), file.name));
}

$('storage-sort').addEventListener('change', renderStorage);

// What was sent before the page was loaded again is still in the queue.
call('print_queue').then((queue) => {
  hostsState.queue = queue;
  if (queue.busy) watchQueue();
}, () => {});
