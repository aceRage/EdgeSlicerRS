'use strict';

// A Bambu Lab printer on the local network, through the UltraNet bridge
// (src/api/bambu_lan.rs, ULTRANET_BRIDGE.md): Device's panel for it
// (StatusPanel.cpp: "Printing Progress", the temperatures, the AMS,
// "Pause" / "Resume" / "Stop"), "Connect the printer using IP and access
// code" (ReleaseNote.cpp `InputIpAddressDialog`) and "Send print job to"
// (SelectMachine.cpp). Everything goes through `call`; the page never
// holds an access code after it was typed: the session keeps it, and
// says only whether there is one.

const bambuState = {
  printers: null,
  timer: null,
  // The send dialog: its last answer, the trays picked by hand (null:
  // the reference's own offer), its refresh, the send under way.
  dialog: null,
  picked: null,
  sendTimer: null,
  task: null,
  // Device was looked at once with this printer: it was connected then.
  dialled: new Set(),
  // The account: its list refreshed once this run by itself, the wait for
  // the sign-in window, the account's printer the add dialog is for.
  accountRefreshed: false,
  accountTimer: null,
  addingAccount: null,
};

// The printer's colour "RRGGBBAA" as CSS.
const trayColour = (colour) => (colour && colour.length >= 6 ? `#${colour.slice(0, 6)}` : '');

function bambuOptions(select, printers, selected) {
  const list = printers || [];
  fill(select, list.map((p) => p.dev_id), selected || '', list.map((p) => (p.model_name ? `${p.name} (${p.model_name.replace('Bambu Lab ', '')})` : p.name)));
}

async function bambuPrinters() {
  bambuState.printers = await call('bambu_printers', {});
  return bambuState.printers;
}

// ---- The Bambu Lab account (MainFrame.cpp's Account menu, WebUserLoginDialog.cpp) ----

// "Bambu Lab: <name>" or "Bambu Lab: signed out", its buttons, what the
// session says, and the account's printers (never a code: the session
// keeps them).
function drawBambuAccount(printers) {
  const account = printers && printers.account;
  const box = $('bambu-account');
  box.hidden = !account || (!account.offered && !account.signed_in);
  if (box.hidden) return;
  const busy = account.phase !== 'idle';
  $('bambu-account-who').textContent = `${t('Bambu Lab')}: ${account.signed_in ? account.name || '' : t('signed out')}`;
  $('bambu-sign-in').hidden = account.signed_in;
  $('bambu-sign-in').disabled = busy || !account.can_sign_in || !account.offered;
  $('bambu-sign-out').hidden = !account.signed_in;
  $('bambu-account-refresh').hidden = !account.signed_in;
  $('bambu-account-refresh').disabled = busy;
  const said = account.said || (account.phase === 'working' ? t('connecting...') : '');
  $('bambu-account-said').hidden = !said;
  $('bambu-account-said').textContent = said;
  $('bambu-account-said').className = account.said ? 'error' : 'quiet';
  const list = $('bambu-account-printers');
  const mine = printers.account_printers || [];
  list.hidden = !mine.length;
  list.replaceChildren(...mine.map((p) => el('li', {},
    el('span', { className: 'name', textContent: p.model_name ? `${p.name} (${p.model_name.replace('Bambu Lab ', '')})` : p.name }),
    el('span', { className: `quiet${p.online ? ' online' : ''}`, textContent: t(p.online ? 'Online' : 'Offline') }),
    el('span', { className: 'tag', textContent: t('Bambu Lab'), title: t('Sign in to your Bambu Lab account to see your cloud printers') }),
    p.added ? el('span', { className: 'quiet', textContent: p.dev_id })
      : el('button', { type: 'button', textContent: t('Add'), onclick: () => attempt(() => openBambuAdd(p)) }))));
  // While the sign-in window is open or the bridge is asked: looked at again.
  clearTimeout(bambuState.accountTimer);
  if (busy) {
    bambuState.accountTimer = setTimeout(() => attempt(async () => {
      const next = await bambuPrinters();
      drawBambuAccount(next);
      if (next.account && next.account.phase === 'idle') await showBambuDevice();
    }), 1000);
  }
}

$('bambu-sign-in').addEventListener('click', () => attempt(async () => {
  await call('bambu_account_sign_in', {});
  drawBambuAccount(await bambuPrinters());
}));
$('bambu-sign-out').addEventListener('click', () => attempt(async () => {
  await call('bambu_account_sign_out', {});
  await showBambuDevice();
}));
$('bambu-account-refresh').addEventListener('click', () => attempt(async () => {
  $('bambu-account-refresh').disabled = true;
  await call('bambu_account_refresh', {});
  await showBambuDevice();
}));

// What the bridge's state says, when it says anything; and "Retry" where
// it gave up or was refused.
function bambuBridgeNote(bridge) {
  const shown = Boolean(bridge && bridge.said);
  $('bambu-bridge').hidden = !shown;
  if (!shown) return;
  $('bambu-bridge-text').textContent = bridge.said;
  $('bambu-retry').hidden = !['failed', 'gave_up', 'refused'].includes(bridge.state);
}

// Where the access codes are kept, when it is not the system's store as
// it should be (the store refused, or the codes could not be moved).
function bambuSecretsNote(printers) {
  const notice = printers && printers.secrets && printers.secrets.notice;
  $('bambu-secrets').hidden = !notice;
  $('bambu-secrets-text').textContent = notice || '';
}

// ---- Device ----

// Called by `showPrinterPage` (hosts.js) for a Bambu Lab printer's
// project: which printer, its session and the bridge, over its Device
// page (device.js).
async function showBambuDevice() {
  $('device-bambu').hidden = false;
  let printers;
  try {
    printers = await bambuPrinters();
  } catch (error) {
    say(error.message, true);
    return;
  }
  bambuOptions($('bambu-printer'), printers.printers, printers.selected);
  const none = !printers.printers.length;
  $('bambu-none').hidden = !none;
  $('bambu-remove').disabled = none;
  $('bambu-connect').disabled = none;
  bambuBridgeNote(printers.bridge);
  bambuSecretsNote(printers);
  drawBambuAccount(printers);
  // The account's list refreshed once a run, by itself only where the store
  // is at hand (reading it may make the system ask the person).
  if (printers.account && printers.account.signed_in && printers.secrets.at_hand && printers.bridge.state !== 'absent'
      && !bambuState.accountRefreshed && state.tab === 'device') {
    bambuState.accountRefreshed = true;
    attempt(async () => drawBambuAccount(await bambuPrinters(await call('bambu_account_refresh', {}))));
  }
  if (none || state.tab !== 'device') return;
  // Looked at: the printer is dialled, once, as the reference dials the
  // printer it selects. A bridge that is absent is not started. Only where
  // the code is at hand: the system's store is read at the person's first
  // Connect or send, never because a page was looked at (reading it may
  // make the system ask them).
  const selected = printers.printers.find((p) => p.selected);
  if (selected && selected.has_access_code && printers.secrets.at_hand && printers.bridge.state !== 'absent' && !bambuState.dialled.has(selected.dev_id)) {
    bambuState.dialled.add(selected.dev_id);
    $('bambu-session').textContent = t('connecting...');
    try {
      drawBambu(await call('bambu_connect', { dev_id: selected.dev_id }));
    } catch (error) {
      say(error.message, true);
    }
  }
  watchBambu();
}

function hideBambuDevice() {
  $('device-bambu').hidden = true;
  clearTimeout(bambuState.timer);
}

function watchBambu() {
  clearTimeout(bambuState.timer);
  if (state.tab !== 'device' || $('device-bambu').hidden) return;
  bambuState.timer = setTimeout(async () => {
    try {
      drawBambu(await call('bambu_state', { redial: true }));
    } catch (error) {
      $('bambu-session').textContent = error.message;
    }
    watchBambu();
  }, 2000);
}

const SESSION_WORDS = { idle: '', connecting: 'connecting...', connected: '', lost: 'Offline' };

// The header's part of the printer's state: its session (the rest is the
// Device page's, device.js).
function drawBambu(s) {
  if (!s || !s.machine) {
    if (s && s.error) $('bambu-session').textContent = s.error;
    if (s && s.bridge) bambuBridgeNote(s.bridge);
    return;
  }
  bambuBridgeNote(s.bridge);
  $('bambu-session').textContent = s.said || (SESSION_WORDS[s.session] ? t(SESSION_WORDS[s.session]) : '');
  $('bambu-session').className = s.session === 'failed' || s.session === 'lost' ? 'error' : 'quiet';
  $('bambu-connect').hidden = s.session === 'connected' || s.session === 'connecting';
}

$('bambu-printer').addEventListener('change', () => attempt(async () => {
  await call('bambu_select', { dev_id: $('bambu-printer').value });
  bambuState.dialled.delete($('bambu-printer').value);
  await showBambuDevice();
  if (typeof watchDevice === 'function') watchDevice(true);
}));
$('bambu-connect').addEventListener('click', () => attempt(async () => {
  $('bambu-session').textContent = t('connecting...');
  drawBambu(await call('bambu_connect', {}));
  bambuSecretsNote(await bambuPrinters());
}));
$('bambu-retry').addEventListener('click', () => attempt(async () => {
  bambuBridgeNote((await call('bambu_bridge', { retry: true })));
  drawBambu(await call('bambu_connect', {}));
}));
$('bambu-add').addEventListener('click', () => attempt(openBambuAdd));
$('bambu-remove').addEventListener('click', () => attempt(async () => {
  const printer = (bambuState.printers.printers || []).find((p) => p.dev_id === $('bambu-printer').value);
  if (!printer) return;
  if (!await confirmBox(t('Remove'), `${printer.name} (${printer.dev_id})`, t('Remove'), t('Cancel'))) return;
  await call('bambu_printer_remove', { dev_id: printer.dev_id });
  await showBambuDevice();
}));
// ---- "Connect the printer using IP and access code" ----

function bambuAddReady() {
  const ip = $('bambu-ip').value.trim();
  const code = $('bambu-code').value;
  const ipOk = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.test(ip) && ip.split('.').every((n) => Number(n) <= 255);
  // A printer of the account may be added with its address alone: the
  // session has the code the account gave.
  const account = bambuState.addingAccount;
  const accountCode = account && account.has_access_code && code === '' && $('bambu-sn').value.trim().toUpperCase() === account.dev_id;
  // `on_text`: an IP, eight letters or digits, fifteen characters of serial, a name.
  $('bambu-add-ok').disabled = !(ipOk && (accountCode || /^[A-Za-z0-9]{8}$/.test(code)) && $('bambu-sn').value.trim().length === 15 && $('bambu-name').value.trim());
}
for (const id of ['bambu-ip', 'bambu-code', 'bambu-sn', 'bambu-name']) $(id).addEventListener('input', bambuAddReady);

// `account`: a printer of the account, whose serial, name and model are
// filled in (the cloud gives no address, DeviceManager.cpp l.8695).
async function openBambuAdd(account) {
  const printers = await bambuPrinters();
  bambuState.addingAccount = account && account.dev_id ? account : null;
  const known = bambuState.addingAccount && printers.models.some((m) => m.id === account.model) ? account.model : null;
  fill($('bambu-model'), printers.models.map((m) => m.id), known || (printers.models.find((m) => m.id === 'C12') || printers.models[0] || {}).id || '', printers.models.map((m) => m.name));
  for (const id of ['bambu-ip', 'bambu-code', 'bambu-sn', 'bambu-name']) $(id).value = '';
  if (bambuState.addingAccount) {
    $('bambu-sn').value = account.dev_id;
    $('bambu-name').value = account.name || '';
  }
  $('bambu-add-result').hidden = true;
  bambuAddReady();
  const dialog = $('bambu-add-dialog');
  const done = new Promise((resolve) => dialog.addEventListener('close', resolve, { once: true }));
  dialog.showModal();
  $('bambu-ip').focus();
  await done;
  // The code is taken out of the page as soon as the dialog closes.
  $('bambu-code').value = '';
  bambuState.addingAccount = null;
}

$('bambu-add-form').addEventListener('submit', async (event) => {
  if (event.submitter && event.submitter.value === 'cancel') return;
  event.preventDefault();
  const result = $('bambu-add-result');
  try {
    await call('bambu_printer_add', {
      ip: $('bambu-ip').value.trim(),
      access_code: $('bambu-code').value,
      serial: $('bambu-sn').value.trim(),
      model: $('bambu-model').value,
      name: $('bambu-name').value.trim(),
    });
  } catch (error) {
    result.hidden = false;
    result.className = 'error';
    result.textContent = error.message;
    return;
  }
  $('bambu-code').value = '';
  result.hidden = false;
  result.className = 'quiet';
  result.textContent = t('connecting...');
  const added = $('bambu-sn').value.trim().toUpperCase();
  bambuState.dialled.add(added);
  let answer = null;
  try {
    answer = await call('bambu_connect', { dev_id: added });
  } catch (error) {
    answer = { error: error.message };
  }
  $('bambu-add-dialog').close('ok');
  if (state.tab === 'device') await showBambuDevice();
  if (answer) drawBambu(answer);
});

// The access code asked again ("The printer refused the access code.
// Re-enter it"): typed in a password field, kept by the session alone.
async function askBambuCode(devId) {
  $('ask-label').textContent = t('Input access code');
  $('ask-text').value = '';
  $('ask-text').type = 'password';
  const promise = modal($('ask'));
  $('ask-text').focus();
  const ok = await promise;
  const code = $('ask-text').value;
  $('ask-text').value = '';
  $('ask-text').type = 'text';
  if (!ok || !code) return false;
  await call('bambu_printer_code', { dev_id: devId, access_code: code });
  bambuState.dialled.delete(devId);
  await call('bambu_connect', { dev_id: devId });
  return true;
}

// ---- "Send print job to" ----

async function openBambuSend() {
  const project = activeId();
  if (!project) return;
  bambuState.picked = null;
  bambuState.task = null;
  $('bambu-send-progress').hidden = true;
  $('bambu-send-status').textContent = t('Synchronizing device information');
  $('bambu-send-status').className = 'quiet';
  $('bambu-send-filaments').replaceChildren();
  $('bambu-send-go').disabled = true;
  $('bambu-send-cancel').textContent = t('Cancel');
  $('bambu-send-dialog').showModal();
  await refreshBambuSend(true);
}

// The dialog asked again: as the reference's timer does while it is open.
async function refreshBambuSend(connect = false) {
  clearTimeout(bambuState.sendTimer);
  const dialog = $('bambu-send-dialog');
  if (!dialog.open || bambuState.task) return;
  const args = { project: activeId(), connect };
  if ($('bambu-send-printer').value) args.dev_id = $('bambu-send-printer').value;
  if (bambuState.dialog) {
    args.use_ams = $('bambu-opt-ams').checked;
    args.timelapse = $('bambu-opt-timelapse').checked;
  }
  if (bambuState.picked) args.mapping = bambuState.picked;
  let answer;
  try {
    answer = await call('bambu_send_prepare', args);
  } catch (error) {
    $('bambu-send-status').textContent = error.message;
    $('bambu-send-status').className = 'error';
    return;
  }
  if (!dialog.open || bambuState.task) return;
  const first = !bambuState.dialog;
  bambuState.dialog = answer;
  bambuOptions($('bambu-send-printer'), answer.printers, answer.printer);
  if (!answer.printers || !answer.printers.length) $('bambu-send-printer').replaceChildren(el('option', { value: '', textContent: t('No available printers') }));
  $('bambu-send-bed').textContent = answer.bed ? `${t('Plate')}: ${t(answer.bed)}` : '';
  $('bambu-send-code').hidden = !answer.refused;
  if (answer.options) {
    const o = answer.options;
    if (first) {
      $('bambu-opt-bed').checked = o.bed_leveling.value;
      $('bambu-opt-flow').checked = o.flow_cali.value;
      $('bambu-opt-timelapse').checked = o.timelapse.value;
      $('bambu-opt-ams').checked = o.use_ams.value;
    }
    $('bambu-opt-ams-row').hidden = !o.use_ams.shown;
  }
  drawBambuFilaments(answer);
  const status = answer.status;
  $('bambu-send-status').textContent = status.message;
  $('bambu-send-status').className = status.warning ? 'warn' : 'quiet';
  $('bambu-send-go').disabled = !status.can_send;
  bambuState.sendTimer = setTimeout(() => refreshBambuSend(false), 2000);
}

function drawBambuFilaments(answer) {
  const trays = answer.trays || [];
  const useAms = $('bambu-opt-ams').checked && !$('bambu-opt-ams-row').hidden;
  $('bambu-send-filaments').replaceChildren(...(answer.filaments || []).map((filament, at) => {
    const select = el('select', { disabled: !useAms, title: t('Filament') });
    fill(select, ['-1', ...trays.map((tray) => String(tray.tray))], String(filament.tray),
      ['-', ...trays.map((tray) => `${tray.name} ${tray.exists ? tray.type : t('Empty')}`)]);
    select.addEventListener('change', () => {
      bambuState.picked = (answer.filaments || []).map((f) => f.tray);
      bambuState.picked[at] = Number(select.value);
      refreshBambuSend(false);
    });
    return el('div', { className: 'bambu-filament' },
      el('span', { className: 'swatch', style: `background:${filament.color.slice(0, 7)}` }),
      el('span', { className: 'bambu-type', textContent: filament.type }),
      el('span', { className: 'quiet', textContent: '→' }),
      el('span', { className: 'swatch', style: filament.tray_color ? `background:${trayColour(filament.tray_color)}` : '' }),
      select);
  }));
}

$('bambu-send-printer').addEventListener('change', () => attempt(async () => {
  bambuState.picked = null;
  await call('bambu_select', { dev_id: $('bambu-send-printer').value });
  await refreshBambuSend(true);
}));
$('bambu-send-refresh').addEventListener('click', () => attempt(() => refreshBambuSend(true)));
for (const id of ['bambu-opt-ams', 'bambu-opt-timelapse']) $(id).addEventListener('change', () => { bambuState.picked = null; refreshBambuSend(false); });
$('bambu-send-code').addEventListener('click', () => attempt(async () => {
  if (await askBambuCode($('bambu-send-printer').value)) await refreshBambuSend(false);
}));

$('bambu-send-cancel').addEventListener('click', () => attempt(async () => {
  if (bambuState.task && !bambuState.task.done) {
    await call('bambu_task_cancel', { task: bambuState.task.id });
    return;
  }
  clearTimeout(bambuState.sendTimer);
  bambuState.dialog = null;
  $('bambu-send-dialog').close();
}));
$('bambu-send-dialog').addEventListener('close', () => {
  clearTimeout(bambuState.sendTimer);
  bambuState.dialog = null;
});

// "Send": the checks first (`on_ok_btn`), then the send and its progress.
$('bambu-send-go').addEventListener('click', () => attempt(async () => {
  const project = activeId();
  const args = {
    project,
    dev_id: $('bambu-send-printer').value,
    options: {
      bed_leveling: $('bambu-opt-bed').checked,
      flow_cali: $('bambu-opt-flow').checked,
      timelapse: $('bambu-opt-timelapse').checked,
      use_ams: $('bambu-opt-ams').checked,
    },
  };
  if (bambuState.picked) args.mapping = bambuState.picked;
  args.pictures = await window.slicedPictures(project, false).catch(() => ({}));
  let answer = await call('bambu_send', args);
  if (answer.confirm) {
    const text = answer.confirm.lines.join('\n');
    if (answer.confirm.blocking) {
      await inform(answer.confirm.title, text);
      return;
    }
    if (!await confirmBox(answer.confirm.title, text, t('Confirm'), t('Cancel'))) return;
    answer = await call('bambu_send', { ...args, confirmed: true });
  }
  clearTimeout(bambuState.sendTimer);
  bambuState.task = { id: answer.task, done: false };
  $('bambu-send-go').disabled = true;
  $('bambu-send-progress').hidden = false;
  $('bambu-send-detail').textContent = '';
  followBambuSend();
}));

async function followBambuSend() {
  const task = bambuState.task;
  if (!task) return;
  const p = await call('bambu_task', { task: task.id });
  $('bambu-send-bar').style.width = `${Math.max(0, p.percent)}%`;
  $('bambu-send-message').textContent = p.message;
  $('bambu-send-message').className = p.done && !p.ok ? 'error' : '';
  if (!p.done) {
    setTimeout(followBambuSend, 250);
    return;
  }
  task.done = true;
  $('bambu-send-cancel').textContent = t('Close');
  if (!p.ok) {
    $('bambu-send-detail').textContent = p.detail || '';
    $('bambu-send-code').hidden = !p.refused;
    bambuState.task = null;
    $('bambu-send-go').disabled = false;
    return;
  }
  // "Successfully sent. Will automatically jump to the device page in 3s".
  setTimeout(async () => {
    if ($('bambu-send-dialog').open) $('bambu-send-dialog').close();
    bambuState.task = null;
    await selectTab('device');
  }, 3000);
}
