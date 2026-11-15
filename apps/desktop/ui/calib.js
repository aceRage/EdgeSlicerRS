'use strict';

// The Calibration menu's dialogs (calib_dlg.cpp): each asks for its test's
// values with the reference's fields, defaults and checks, then
// `calibrate` makes the test a project of its own on the plate, ready to
// slice (src/api/calib.rs). The dialog is made here each time it opens.

const Calib = (() => {
  // Opens a page of the reference's guides: the window opens it, a
  // browser shows its address.
  async function openGuide(url) {
    if (tauri) await tauri.core.invoke('hand_off', { page: url });
    else say(url);
  }

  // A number as wxString::FromDouble writes it.
  const shown = (number) => String(number);

  // ParseStringValues: whole numbers above zero, by commas or spaces;
  // reading stops at the first that is not a whole number.
  function parseList(text) {
    const values = [];
    for (const word of text.replace(/,/g, ' ').split(/\s+/).filter(Boolean)) {
      const match = /^[+-]?\d+/.exec(word);
      if (!match) break;
      const value = Number(match[0]);
      if (value > 0) values.push(value);
      if (match[0].length !== word.length) break;
    }
    return values;
  }

  // A labelled group of the dialog (the reference's LabeledStaticBox).
  function group(title, ...children) {
    return el('fieldset', { className: 'calib-group' }, el('legend', { textContent: t(title) }), ...children);
  }

  function radios(name, labels, picked) {
    return el('div', { className: 'calib-radios', role: 'radiogroup' }, ...labels.map((label, index) => {
      const input = el('input', { type: 'radio', name, value: String(index), checked: index === picked });
      return el('label', {}, input, t(label));
    }));
  }

  function field(label, unit = '') {
    const input = el('input', { type: 'text', inputMode: 'decimal', autocomplete: 'off', spellcheck: false });
    const row = el('label', { className: 'calib-field' }, el('span', { textContent: t(label) }), input);
    if (unit) row.append(el('span', { className: 'quiet', textContent: unit }));
    return { row, input };
  }

  // Runs the dialog until it is closed, or its values are taken; `take`
  // answers the error to show, or nothing when the test was made.
  function run(dialog, take) {
    document.body.append(dialog);
    return new Promise((resolve) => {
      dialog.addEventListener('close', () => { dialog.remove(); resolve(); }, { once: true });
      dialog.querySelector('form').addEventListener('submit', async (event) => {
        if (event.submitter && event.submitter.value === 'cancel') return;
        event.preventDefault();
        const error = await take();
        if (error) await inform(dialog.querySelector('h2').textContent, error);
        else dialog.close('ok');
      });
      dialog.showModal();
    });
  }

  // Makes the test's project, or says why not (the strip full: as "New Project").
  async function make(args) {
    let listing = await call('calibrate', args);
    if (listing.full) {
      if (!(await makeRoom(listing))) return null;
      listing = await call('calibrate', args);
      if (listing.full) return null;
    }
    if (state.tab === 'device') state.tab = 'prepare';
    state.project = null;
    await applyListing(listing);
    // The reference's notifications (no acceleration or speed given).
    if (listing.notes && listing.notes.length) say(listing.notes.join('\n'));
    return null;
  }

  // "PA Calibration" (calib_dlg.cpp `PA_Calibration_Dlg`).
  function pressureAdvance() {
    const extruder = radios('calib-extruder', ['DDE', 'Bowden'], 0);
    const method = radios('calib-method', ['PA Tower', 'PA Line', 'PA Pattern'], 0);
    const start = field('Start PA: ');
    const end = field('End PA: ');
    const step = field('PA step: ');
    const numbers = el('input', { type: 'checkbox' });
    const numbersRow = el('label', { className: 'calib-field' }, el('span', { textContent: t('Print numbers') }), numbers);
    const accels = field('Accelerations: ');
    accels.input.title = t('Comma-separated list of printing accelerations');
    const speeds = field('Speeds: ');
    speeds.input.title = t('Comma-separated list of printing speeds');
    for (const list of [accels, speeds]) {
      list.input.inputMode = 'numeric';
      list.input.addEventListener('input', () => { list.input.value = list.input.value.replace(/[^0-9,]/g, ''); });
    }
    const picked = (group) => Number(group.querySelector('input:checked').value);

    // PA_Calibration_Dlg::reset_params, on opening and each change of
    // extruder or method.
    function reset() {
      const dde = picked(extruder) === 0;
      const mode = picked(method);
      start.input.value = shown(0);
      if (mode === 1) {
        end.input.value = shown(0.1); step.input.value = shown(0.002);
        numbers.checked = true; numbers.disabled = false;
        accels.input.disabled = true; speeds.input.disabled = true;
      } else if (mode === 2) {
        end.input.value = shown(0.08); step.input.value = shown(0.005);
        numbers.checked = true; numbers.disabled = true;
        accels.input.disabled = false; speeds.input.disabled = false;
      } else {
        end.input.value = shown(0.1); step.input.value = shown(0.002);
        numbers.checked = false; numbers.disabled = true;
        accels.input.disabled = true; speeds.input.disabled = true;
      }
      if (!dde) {
        end.input.value = shown(1.0);
        step.input.value = shown(mode === 2 ? 0.05 : 0.02);
      }
    }
    extruder.addEventListener('change', reset);
    method.addEventListener('change', reset);
    reset();

    const guide = (label, url) => {
      const link = el('a', { href: url, textContent: t(label) });
      link.addEventListener('click', (event) => { event.preventDefault(); attempt(() => openGuide(url)); });
      return link;
    };
    const dialog = el('dialog', { className: 'calib-dialog' }, el('form', { method: 'dialog' },
      el('h2', { textContent: t('PA Calibration') }),
      group('Extruder type', extruder),
      group('Method', method),
      group('Settings', start.row, end.row, step.row, numbersRow, accels.row, speeds.row),
      el('div', { className: 'calib-links' },
        guide('Pressure Advance Guide', 'https://github.com/SoftFever/OrcaSlicer/wiki/pressure-advance-calib'),
        guide('Adaptive Pressure Advance Guide', 'https://github.com/SoftFever/OrcaSlicer/wiki/adaptive-pressure-advance-calib')),
      el('div', { className: 'row end' },
        el('button', { type: 'submit', value: 'ok', className: 'primary', textContent: t('OK') }),
        el('button', { type: 'submit', value: 'cancel', formNoValidate: true, textContent: t('Cancel') }))));

    return run(dialog, async () => {
      // PA_Calibration_Dlg::on_start: three numbers, then the lists.
      const numbersOf = [start, end, step].map((f) => (f.input.value.trim() === '' ? NaN : Number(f.input.value)));
      const [a, b, c] = numbersOf;
      if (numbersOf.some((n) => !Number.isFinite(n)) || a < 0 || c < 1e-4 || b < a + c) {
        return t('Please input valid values:\nStart PA: >= 0.0\nEnd PA: > Start PA\nPA step: >= 0.001');
      }
      const accelerations = parseList(accels.input.value);
      const speedList = parseList(speeds.input.value);
      // Orca #11493: accelerations must exceed speeds.
      if (accelerations.length && speedList.length
          && Math.min(...accelerations) <= Math.max(...speedList)) {
        return t('Acceleration values must be greater than speed values.\nPlease verify the inputs.');
      }
      const test = ['pa_tower', 'pa_line', 'pa_pattern'][picked(method)];
      try {
        await make({ test, start: a, end: b, step: c, print_numbers: numbers.checked, accelerations, speeds: speedList });
      } catch (error) {
        return error.message;
      }
      return null;
    });
  }

  // "Temperature calibration" (calib_dlg.cpp `Temp_Calibration_Dlg`).
  function temperature() {
    const DEGREES = '\u2103';
    const types = ['PLA', 'ABS/ASA', 'PETG', 'PCTG', 'TPU', 'PA-CF', 'PET-CF', 'Custom'];
    const filament = radios('calib-filament', types, 0);
    filament.classList.add('calib-two');
    const start = field('Start temp: ', DEGREES);
    const end = field('End temp: ', DEGREES);
    const step = field('Temp step: ', DEGREES);
    start.input.value = '230';
    end.input.value = '190';
    step.input.value = shown(5);
    step.input.disabled = true;
    // on_filament_type_changed.
    const ranges = [[230, 190], [270, 230], [250, 230], [280, 240], [240, 210], [320, 280], [320, 280], [230, 190]];
    filament.addEventListener('change', () => {
      const [from, to] = ranges[Number(filament.querySelector('input:checked').value)];
      start.input.value = String(from);
      end.input.value = String(to);
    });
    // The fields' check on leaving them (validate_text): a whole number
    // outside 170 to 350 is said and brought in, then rounded down to 5.
    for (const f of [start, end]) {
      f.input.addEventListener('blur', async () => {
        if (!/^\d+$/.test(f.input.value.trim())) return;
        let value = Number(f.input.value.trim());
        if (value > 350 || value < 170) {
          await inform(t('Temperature calibration'), `Supported range: 170${DEGREES} - 350${DEGREES}`);
          value = value > 350 ? 350 : 170;
        }
        f.input.value = String(Math.floor(value / 5) * 5);
      });
    }
    const link = el('a', { href: 'https://github.com/SoftFever/OrcaSlicer/wiki/temp-calib', textContent: t('Wiki Guide: Temperature Calibration') });
    link.addEventListener('click', (event) => { event.preventDefault(); attempt(() => openGuide(link.href)); });
    const dialog = el('dialog', { className: 'calib-dialog' }, el('form', { method: 'dialog' },
      el('h2', { textContent: t('Temperature calibration') }),
      group('Filament type', filament),
      group('Settings', start.row, end.row, step.row),
      el('div', { className: 'calib-links' }, link),
      el('div', { className: 'row end' },
        el('button', { type: 'submit', value: 'ok', className: 'primary', textContent: t('OK') }),
        el('button', { type: 'submit', value: 'cancel', formNoValidate: true, textContent: t('Cancel') }))));
    return run(dialog, async () => {
      // on_start: whole numbers (`ToULong`); the session checks the rest.
      const whole = (f) => (/^\d+$/.test(f.input.value.trim()) ? Number(f.input.value.trim()) : -1);
      try {
        await make({ test: 'temp_tower', start: whole(start), end: whole(end), step: 5 });
      } catch (error) {
        return error.message;
      }
      return null;
    });
  }

  // "Flow rate" (MainFrame.cpp, Plater.cpp `calib_flowrate`): no dialog;
  // the menu item makes the test's project at once.
  function flowRate(test) {
    return make({ test });
  }

  // "Tutorial" (MainFrame.cpp): the calibration guide. The reference sends
  // a person in China to a mirror by the country kept in its settings;
  // this app keeps no country.
  function tutorial() {
    return openGuide('https://github.com/SoftFever/OrcaSlicer/wiki/Calibration');
  }

  return { pressureAdvance, temperature, flowRate, tutorial, parseList };
})();
