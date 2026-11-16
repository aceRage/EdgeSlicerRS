'use strict';

// The Device page of every printer family (src/device, src/api/device.rs),
// on the Bambu monitor's layout (StatusPanel.cpp): "Printing Progress" with
// Pause / Resume / Stop, the camera where the family's API gives a plain
// stream, the temperatures, fans, speed and lamp, the axes and the
// extruder, the AMS or the toolheads; and Bambu's "HMS" and "Update".
// Everything is drawn from what the session says the printer has; a
// control it has not got is not drawn. Every command is asked first: the
// session answers the question, the page shows it, and sends the command
// again confirmed. Nothing here talks to a printer; `call` does, through
// the session.

const deviceState = {
  // The last answer of `device_read`: what the printer has and shows.
  caps: null,
  reading: null,
  timer: null,
  // Which project, and whether its caps were asked afresh.
  project: null,
  fresh: false,
  busy: false,
  // Bambu's camera through the bridge: started, and the frame poll.
  frames: false,
  frameTimer: null,
  frameSeq: null,
};

const heaterText = (h) => (h ? `${Math.round(h.now)} / ${Math.round(h.target)} °C` : t('N/A'));

// The printer's state, every few seconds while one of its native tabs is
// looked at. `refresh` asks what the printer has again (Device opened).
function watchDevice(refresh = false) {
  clearTimeout(deviceState.timer);
  const project = activeId();
  if (deviceState.project !== project) {
    deviceState.project = project;
    deviceState.caps = null;
    deviceState.reading = null;
    refresh = true;
  }
  const looking = () => state.tab === 'device' && hostsState.native && activeId() === project;
  if (!looking()) {
    stopCamera();
    return;
  }
  deviceState.timer = setTimeout(async () => {
    let answer;
    try {
      answer = await call('device_read', { project, refresh });
    } catch (error) {
      answer = { online: false, said: t('Offline'), error: error.message };
    }
    if (!looking()) return;
    drawDevice(answer);
    watchDevice(false);
  }, refresh ? 0 : 2500);
}

function stopDevice() {
  clearTimeout(deviceState.timer);
  stopCamera();
}

// The stream is let go as soon as Device is not looked at.
function stopCamera() {
  stopFrames();
  if (!$('dv-camera').dataset.src) return;
  delete $('dv-camera').dataset.src;
  $('dv-camera').removeAttribute('src');
}

// Bambu's camera (section 15 of ULTRANET_BRIDGE.md): the bridge is told
// to start it, the latest frame is asked for twice a second and shown as
// data; told to stop when Status is not looked at.
function startFrames() {
  if (deviceState.frames) return;
  deviceState.frames = true;
  deviceState.frameSeq = null;
  const project = activeId();
  call('device_camera', { project, action: 'start' }).then(() => pollFrame(project), (error) => {
    deviceState.frames = false;
    $('dv-camera-note').hidden = false;
    $('dv-camera-note').textContent = error.message;
  });
}

function pollFrame(project) {
  clearTimeout(deviceState.frameTimer);
  if (!deviceState.frames) return;
  deviceState.frameTimer = setTimeout(async () => {
    let frame = null;
    try {
      frame = await call('device_camera', { project, action: 'frame' });
    } catch {
      frame = null;
    }
    if (!deviceState.frames) return;
    if (frame && frame.picture && frame.seq !== deviceState.frameSeq) {
      deviceState.frameSeq = frame.seq;
      $('dv-camera-img').src = frame.picture;
    }
    pollFrame(project);
  }, 500);
}

function stopFrames() {
  clearTimeout(deviceState.frameTimer);
  if (!deviceState.frames) return;
  deviceState.frames = false;
  $('dv-camera-img').removeAttribute('src');
  call('device_camera', { project: deviceState.project, action: 'stop' }).catch(() => {});
}

// ---- drawing ----

function drawDevice(answer) {
  if (answer.native === false) return;
  if (answer.caps) deviceState.caps = answer.caps;
  const caps = deviceState.caps;
  const online = Boolean(answer.reading);
  const r = online ? answer.reading : null;
  deviceState.reading = r;
  // The state over the tabs, as the reference's printer header says it.
  const line = $('device-status');
  line.hidden = false;
  line.className = online ? `state-${r.state}` : 'error';
  line.textContent = online ? [r.said, r.file].filter(Boolean).join(' · ') : answer.said;
  line.title = online ? (r.message || '') : (answer.error || '');
  $('device-native').classList.toggle('offline', !online);
  drawProgress(caps, r, answer);
  if (!caps) return;
  drawHeaters(caps, r);
  drawFans(caps, r);
  drawSpeed(caps, r);
  $('dv-light-row').hidden = !caps.light;
  if (caps.light && document.activeElement !== $('dv-light')) $('dv-light').checked = Boolean(r && r.light);
  drawAxes(caps, r);
  drawSlots(caps, r);
  drawCamera(caps, r);
  drawHms(r);
  drawVersions(r);
  drawUpdate(caps, r);
  for (const control of $('device-native').querySelectorAll('.dv-control')) control.disabled = !online || deviceState.busy;
}

function drawProgress(caps, r, answer) {
  $('dv-file').textContent = (r && r.file) || t('N/A');
  const percent = r && r.percent !== null && r.percent !== undefined ? r.percent : null;
  $('dv-bar').style.width = `${percent ?? 0}%`;
  $('dv-percent').textContent = percent === null ? t('N/A') : `${percent}%`;
  $('dv-said').textContent = r ? r.said : answer.said;
  // A printer that says nothing of layers has no layer line.
  $('dv-layer').textContent = r && r.total_layers
    ? t('Layer: %d/%d').replace('%d', r.layer ?? 0).replace('%d', r.total_layers)
    : '';
  $('dv-left').textContent = r && r.left_text ? `-${r.left_text}` : '';
  const message = r ? (r.message === r.said ? '' : r.message) : answer.error;
  $('dv-message').hidden = !message;
  $('dv-message').textContent = message || '';
  $('dv-message').className = r ? 'quiet' : 'error';
  const state = r ? r.state : 'offline';
  const pause = $('dv-pause');
  pause.hidden = !(caps && (caps.pause || caps.resume));
  pause.textContent = state === 'paused' ? t('Resume') : t('Pause');
  pause.disabled = deviceState.busy || !(state === 'paused' ? caps && caps.resume : caps && caps.pause && state === 'printing');
  $('dv-stop').hidden = !(caps && caps.stop);
  $('dv-stop').disabled = deviceState.busy || !['printing', 'paused', 'preparing'].includes(state);
}

function drawHeaters(caps, r) {
  const box = $('dv-heaters');
  if (box.dataset.for !== JSON.stringify(caps.heaters)) {
    box.dataset.for = JSON.stringify(caps.heaters);
    box.replaceChildren(...caps.heaters.map((heater) => {
      const now = el('span', { className: 'num dv-now' });
      now.dataset.heater = heater.id;
      const children = [el('span', { className: 'dv-label', textContent: heater.label }), now];
      if (heater.settable) {
        const input = el('input', { type: 'number', min: 0, max: heater.max, step: 1, className: 'dv-control dv-target', title: `0 - ${heater.max} °C` });
        input.dataset.heater = heater.id;
        input.setAttribute('aria-label', heater.label);
        input.addEventListener('change', () => attempt(async () => {
          const target = Number(input.value);
          input.blur();
          await deviceCommand({ action: 'temperature', heater: heater.id, target });
        }));
        children.push(input, el('span', { className: 'quiet', textContent: '°C' }));
      }
      return el('div', { className: 'dv-row' }, ...children);
    }));
  }
  for (const heater of caps.heaters) {
    const reading = r ? r.heaters.find((h) => h.id === heater.id) : null;
    const now = box.querySelector(`.dv-now[data-heater="${heater.id}"]`);
    // A heater that is only read shows its target beside it.
    if (now) now.textContent = reading ? (heater.settable || !reading.target ? `${Math.round(reading.now)} °C` : heaterText(reading)) : t('N/A');
    const input = box.querySelector(`input[data-heater="${heater.id}"]`);
    // As TempInput: the target is not written over while it is typed in.
    if (input && document.activeElement !== input) input.value = reading ? Math.round(reading.target) : '';
  }
}

function drawFans(caps, r) {
  const box = $('dv-fans');
  if (box.dataset.for !== JSON.stringify(caps.fans)) {
    box.dataset.for = JSON.stringify(caps.fans);
    box.replaceChildren(...caps.fans.map((fan) => {
      const now = el('span', { className: 'num dv-now' });
      now.dataset.fan = fan.id;
      // The reference's fan popup: off to full in tens.
      const select = el('select', { className: 'dv-control', title: fan.label });
      select.dataset.fan = fan.id;
      fill(select, ['', ...Array.from({ length: 11 }, (_, i) => String(i * 10))], '', ['…', ...Array.from({ length: 11 }, (_, i) => `${i * 10} %`)]);
      select.addEventListener('change', () => attempt(async () => {
        if (select.value === '') return;
        const percent = Number(select.value);
        select.value = '';
        await deviceCommand({ action: 'fan', fan: fan.id, percent });
      }));
      return el('div', { className: 'dv-row' }, el('span', { className: 'dv-label', textContent: fan.label }), now, select);
    }));
  }
  for (const fan of caps.fans) {
    const reading = r ? r.fans.find((f) => f.id === fan.id) : null;
    const now = box.querySelector(`.dv-now[data-fan="${fan.id}"]`);
    if (now) now.textContent = reading ? `${reading.percent} %` : t('N/A');
  }
}

function drawSpeed(caps, r) {
  const speed = caps.speed;
  $('dv-speed-row').hidden = !speed;
  if (!speed) return;
  const box = $('dv-speed-box');
  if (box.dataset.for !== JSON.stringify(speed)) {
    box.dataset.for = JSON.stringify(speed);
    let control;
    if (speed.kind === 'levels') {
      control = el('select', { id: 'dv-speed', className: 'dv-control', title: t('This only takes effect during printing') });
      fill(control, ['', ...speed.levels.map((l) => String(l.value))], '', ['…', ...speed.levels.map((l) => l.label)]);
    } else {
      control = el('input', { id: 'dv-speed', type: 'number', min: speed.min, max: speed.max, step: 10, className: 'dv-control', title: `${speed.min} - ${speed.max} %` });
    }
    control.addEventListener('change', () => attempt(async () => {
      if (control.value === '') return;
      const value = Number(control.value);
      control.value = '';
      control.blur();
      await deviceCommand({ action: 'speed', value });
    }));
    box.replaceChildren(control);
  }
  const now = r ? r.speed : null;
  const level = speed.kind === 'levels' && now !== null ? speed.levels.find((l) => l.value === now) : null;
  $('dv-speed-now').textContent = now === null || now === undefined ? t('N/A') : (level ? level.label : `${now} %`);
}

// StatusPanel's ring: Y up, X left and right, Y down, 10 mm out, 1 mm in,
// home in the middle; the bed or Z beside it; the extruder.
function drawAxes(caps, r) {
  const jog = caps.jog;
  $('dv-axes').hidden = !jog;
  if (!jog) return;
  if (!$('dv-ring').childElementCount) {
    const move = (axis, mm, text) => {
      const button = el('button', { type: 'button', className: 'dv-control dv-step', textContent: text, title: `${axis.toUpperCase()} ${mm > 0 ? '+' : ''}${mm} mm` });
      button.addEventListener('click', () => attempt(() => deviceCommand({ action: 'jog', axis, mm })));
      return button;
    };
    const home = el('button', { type: 'button', className: 'dv-control dv-home', textContent: '⌂', title: t('Go Home') });
    home.addEventListener('click', () => attempt(() => deviceCommand({ action: 'home' })));
    const blank = () => el('span');
    $('dv-ring').replaceChildren(
      blank(), blank(), move('y', 10, 'Y +10'), blank(), blank(),
      blank(), blank(), move('y', 1, '+1'), blank(), blank(),
      move('x', -10, '-X 10'), move('x', -1, '-1'), home, move('x', 1, '+1'), move('x', 10, 'X +10'),
      blank(), blank(), move('y', -1, '-1'), blank(), blank(),
      blank(), blank(), move('y', -10, '-Y 10'), blank(), blank());
    $('dv-z').replaceChildren(move('z', -10, '↑ 10'), move('z', -1, '↑ 1'), move('z', 1, '↓ 1'), move('z', 10, '↓ 10'));
    const extrude = (mm, text) => {
      const button = el('button', { type: 'button', className: 'dv-control dv-step', textContent: text, title: `E ${mm > 0 ? '+' : ''}${mm} mm` });
      button.addEventListener('click', () => attempt(() => deviceCommand({ action: 'extrude', mm })));
      return button;
    };
    $('dv-e').replaceChildren(extrude(-10, '↑'), extrude(10, '↓'));
  }
  $('dv-z-label').textContent = jog.z_label;
  $('dv-e-box').hidden = !jog.extrude;
  // As the reference, the axes are off while a print runs.
  const running = r && ['printing', 'preparing'].includes(r.state);
  $('dv-axes').classList.toggle('dv-off', Boolean(running));
}

function drawSlots(caps, r) {
  $('dv-slots-card').hidden = !caps.slots;
  if (!caps.slots) return;
  $('dv-slots-title').textContent = caps.slots === 'ams' ? t('AMS') : t('Toolheads');
  const groups = r ? r.slots : [];
  const units = groups.map((group) => el('div', { className: 'dv-unit' },
    el('span', { className: 'quiet dv-unit-name', textContent: group.name === $('dv-slots-title').textContent ? '' : group.name }),
    el('div', { className: 'bambu-unit' }, ...group.slots.map((slot) => el('div', {
      className: `bambu-tray${slot.active ? ' active' : ''}${slot.loaded ? '' : ' empty'}`,
      title: [slot.name, slot.loaded ? slot.type : t('Empty'), slot.detail, slot.remain >= 0 ? `${slot.remain}%` : ''].filter(Boolean).join(' '),
    },
    el('span', { className: 'swatch', style: slot.loaded && slot.color ? `background:${slot.color}` : '' }),
    el('span', { className: 'bambu-tray-name', textContent: slot.name }),
    el('span', { className: 'bambu-tray-type', textContent: slot.loaded ? (slot.type || '?') : t('Empty') }))))));
  $('dv-slots').replaceChildren(...(units.length ? units : [el('p', { className: 'quiet', textContent: t('N/A') })]));
}

// The camera: a plain stream the printer names, in a frame of its own
// (no scripts: the sandbox allows its own origin only, which a browser
// needs to show the stream), only where the page may frame its address.
function drawCamera(caps, r) {
  const unavailable = caps.unavailable && caps.unavailable.camera;
  if (caps.camera_frames || unavailable) {
    // Bambu: the frames, or why there are none.
    $('dv-camera-card').hidden = false;
    $('dv-camera').hidden = true;
    $('dv-camera-img').hidden = !caps.camera_frames;
    $('dv-camera-note').hidden = !unavailable;
    $('dv-camera-note').textContent = unavailable || '';
    if (caps.camera_frames && r && hostsState.deviceTab === 'status') startFrames();
    else stopFrames();
    return;
  }
  $('dv-camera').hidden = false;
  $('dv-camera-img').hidden = true;
  $('dv-camera-note').hidden = true;
  const url = caps.camera && r && r.camera && framable(r.camera) ? r.camera : null;
  $('dv-camera-card').hidden = !url;
  if (url && $('dv-camera').dataset.src !== url) {
    $('dv-camera').dataset.src = url;
    $('dv-camera').src = url;
  }
  if (!url) stopCamera();
}

function drawHms(r) {
  const list = r ? r.hms : [];
  $('dv-hms').replaceChildren(...(list.length ? list.map((item) => el('div', { className: `dv-hms level-${item.level}` },
    el('span', { className: 'dv-hms-level', textContent: item.level }),
    el('span', { className: 'dv-hms-text', textContent: item.text }),
    el('span', { className: 'quiet num', textContent: `${item.module} ${item.code}` })))
    : [el('p', { className: 'quiet', textContent: t('N/A') })]));
}

function drawVersions(r) {
  const rows = [];
  for (const v of (r ? r.versions : [])) {
    rows.push(el('dt', { textContent: v.name }), el('dd', { className: 'num', textContent: `${t('Version:')} ${v.sw || t('N/A')}${v.sn ? `  ${t('Serial:')} ${v.sn}` : ''}` }));
  }
  $('dv-versions').replaceChildren(...(rows.length ? rows : [el('dt', { textContent: t('N/A') })]));
}

// "Update firmware" (UpgradePanel.cpp `show_status`): offered when the
// printer says it found a newer firmware, with the versions; the state of
// an update under way; or why it cannot be had from here.
function drawUpdate(caps, r) {
  const up = r ? r.update : null;
  const button = $('dv-update-btn');
  const unavailable = caps.unavailable && caps.unavailable.update;
  button.hidden = !caps.update;
  let line = '';
  if (unavailable) line = unavailable;
  else if (!caps.update) line = '';
  else if (!up || up.state === 0 || up.state === null) line = t('N/A');
  else if (up.state === 1) {
    line = up.versions.length
      ? up.versions.map((v) => `${v.module}: ${v.now} → ${v.new}`).join('\n')
      : `ota: → ${up.version}`;
  } else if (up.state === 2) line = `${t('Updating')} ${up.progress ? `${up.progress}%` : ''}`.trim();
  else if (up.state === 3) line = up.status === 'UPGRADE_FAIL' ? t('Update failed') : t('Update successful');
  $('dv-update-line').textContent = line;
  const running = r && ['printing', 'paused', 'preparing'].includes(r.state);
  button.dataset.version = up && up.state === 1 ? up.version : '';
  button.disabled = deviceState.busy || !r || running || !(up && up.state === 1 && up.version);
  button.title = running ? t('The printer is printing: its firmware cannot be updated now.') : '';
}

$('dv-update-btn').addEventListener('click', () => attempt(() => deviceCommand({ action: 'update', version: $('dv-update-btn').dataset.version })));

// ---- commands ----

// Asked first, as the session words it; sent only once confirmed. The page
// is drawn again from the printer's answer right after.
async function deviceCommand(args) {
  const project = activeId();
  const asked = await call('device_command', { project, ...args });
  if (!asked.confirm) return;
  if (!await confirmBox(asked.confirm.title, asked.confirm.lines.join('\n\n'), asked.confirm.ok, t('Cancel'))) return;
  deviceState.busy = true;
  try {
    await call('device_command', { project, ...args, confirmed: true });
    say(t('Sent to the printer.'));
  } finally {
    deviceState.busy = false;
    watchDevice(false);
  }
}

$('dv-pause').addEventListener('click', () => attempt(() => deviceCommand({ action: deviceState.reading && deviceState.reading.state === 'paused' ? 'resume' : 'pause' })));
$('dv-stop').addEventListener('click', () => attempt(() => deviceCommand({ action: 'stop' })));
$('dv-light').addEventListener('change', () => attempt(async () => {
  const on = $('dv-light').checked;
  // Shown as it is until the printer says otherwise.
  $('dv-light').checked = !on;
  await deviceCommand({ action: 'light', on });
}));
