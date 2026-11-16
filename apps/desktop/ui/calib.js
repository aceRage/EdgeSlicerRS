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
        // '' keeps the dialog open with nothing said (a question answered No).
        if (error) await inform(dialog.querySelector('h2').textContent, error);
        else if (error !== '') dialog.close('ok');
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

  // "Retraction test" (calib_dlg.cpp `Retraction_Test_Dlg`).
  function retraction() {
    const start = field('Start retraction length: ', 'mm');
    const end = field('End retraction length: ', 'mm');
    const step = field(`${t('Step')}: `, 'mm');
    start.input.value = '0';
    end.input.value = '2';
    step.input.value = shown(0.1);
    const link = el('a', { href: 'https://github.com/SoftFever/OrcaSlicer/wiki/retraction-calib', textContent: t('Wiki Guide: Retraction Calibration') });
    link.addEventListener('click', (event) => { event.preventDefault(); attempt(() => openGuide(link.href)); });
    const dialog = el('dialog', { className: 'calib-dialog' }, el('form', { method: 'dialog' },
      el('h2', { textContent: t('Retraction test') }),
      group('Settings', start.row, end.row, step.row),
      el('div', { className: 'calib-links' }, link),
      el('div', { className: 'row end' },
        el('button', { type: 'submit', value: 'ok', className: 'primary', textContent: t('OK') }),
        el('button', { type: 'submit', value: 'cancel', formNoValidate: true, textContent: t('Cancel') }))));
    return run(dialog, async () => {
      // on_start: three numbers (`ToDouble`); the session checks them.
      const read = (f) => (f.input.value.trim() === '' ? null : Number(f.input.value));
      try {
        await make({ test: 'retraction', start: read(start), end: read(end), step: read(step) });
      } catch (error) {
        return error.message;
      }
      return null;
    });
  }

  // A dialog of three numbers with units and a guide (calib_dlg.cpp
  // `MaxVolumetricSpeed_Test_Dlg`, `VFA_Test_Dlg`): its fields, then the
  // test made from what they hold (`ToDouble`; the session checks them).
  function threeNumbers({ title, labels, unit, values, guide, test, extra = [] }) {
    const fields = labels.map((label) => field(label, unit));
    fields.forEach((f, index) => { f.input.value = values[index]; });
    const link = el('a', { href: guide[1], textContent: t(guide[0]) });
    link.addEventListener('click', (event) => { event.preventDefault(); attempt(() => openGuide(link.href)); });
    const dialog = el('dialog', { className: 'calib-dialog' }, el('form', { method: 'dialog' },
      el('h2', { textContent: t(title) }),
      group('Settings', ...fields.map((f) => f.row), ...extra.map((e) => e.row)),
      el('div', { className: 'calib-links' }, link),
      el('div', { className: 'row end' },
        el('button', { type: 'submit', value: 'ok', className: 'primary', textContent: t('OK') }),
        el('button', { type: 'submit', value: 'cancel', formNoValidate: true, textContent: t('Cancel') }))));
    return { dialog, fields };
  }

  // A number as `wxString::ToDouble` reads it: the whole text, or nothing.
  const readNumber = (f) => (f.input.value.trim() === '' ? null : Number(f.input.value));

  // "Max volumetric speed test" (calib_dlg.cpp `MaxVolumetricSpeed_Test_Dlg`).
  function maxFlowrate() {
    const { dialog, fields } = threeNumbers({
      title: 'Max volumetric speed test',
      labels: ['Start volumetric speed: ', 'End volumetric speed: ', `${t('Step')}: `],
      unit: 'mm\u00b3/s',
      values: ['5', '20', shown(0.5)],
      guide: ['Wiki Guide: Volumetric Speed Calibration', 'https://github.com/SoftFever/OrcaSlicer/wiki/volumetric-speed-calib'],
    });
    return run(dialog, async () => {
      const [start, end, step] = fields.map(readNumber);
      try {
        await make({ test: 'max_flowrate', start, end, step });
      } catch (error) {
        return error.message;
      }
      return null;
    });
  }

  // A tick with its words, and the tooltip on both (calib_dlg.cpp
  // `add_scale_checkbox` and the rest).
  function tick(label, tooltip, checked = true) {
    const input = el('input', { type: 'checkbox', checked });
    const row = el('label', { className: 'calib-field calib-tick', title: t(tooltip) }, input, el('span', { textContent: t(label) }));
    return { row, input };
  }

  // "VFA test" (calib_dlg.cpp `VFA_Test_Dlg`).
  function vfa() {
    const resize = tick('Auto-scale for nozzle', 'This model is designed around a 0.4 mm nozzle with a 0.2 mm layer height.\nWhen the scaling option is enabled (recommended), it dynamically resizes to match your current nozzle diameter and an appropriate layer height, making the test both accurate and easy to read.\nTurn scaling off only if you wish to print the reference model exactly as-is.');
    const adjust = tick('Auto-adjust to max volumetric speed', "If the end speed would exceed the filament's maximum volumetric speed, automatically lower the layer height (keeping standard values and staying within the machine's limits) to reach it. If even the minimum layer height is not enough, lower the end speed instead.");
    const { dialog, fields } = threeNumbers({
      title: 'VFA test',
      labels: ['Start speed: ', 'End speed: ', `${t('Step')}: `],
      unit: 'mm/s',
      values: ['40', '200', shown(10)],
      guide: ['Wiki Guide: VFA', 'https://github.com/SoftFever/OrcaSlicer/wiki/vfa-calib'],
      extra: [resize, adjust],
    });
    const title = t('VFA test');
    const yesNo = (text) => confirmBox(title, text, t('Yes'), t('No'));
    return run(dialog, async () => {
      const [start, end, step] = fields.map(readNumber);
      const args = { test: 'vfa', start, end, step, resize: resize.input.checked };
      try {
        // on_start: the end speed against what the filament melts.
        const check = await call('calib_vfa_check', args);
        const clamp = (question) => yesNo(fmt(t("The end speed (%.0f mm/s) exceeds the filament's maximum volumetric speed (%.1f mm³/s), which limits the outer wall to about %.0f mm/s at this line width and layer height.\nSpeeds above this will be clamped, so the upper blocks of the tower will not print at the requested speed.\n\n%s"),
          check.end, check.max_volumetric_speed, check.limit, question));
        if (check.over) {
          if (adjust.input.checked && resize.input.checked) {
            const plan = check.plan || { kind: 'fits' };
            if (plan.kind === 'layer_height') {
              args.layer_height = plan.layer_height;
              await inform(title, fmt(t("The end speed (%.0f mm/s) exceeds the filament's maximum volumetric speed (%.1f mm³/s) at the default layer height (%.2f mm).\n\nThe layer height has been reduced to %.2f mm (a value used by this printer's profiles) so the tower can reach the requested speed."),
                check.end, check.max_volumetric_speed, check.default_layer_height, plan.layer_height));
            } else if (plan.kind === 'end_speed') {
              const go = await yesNo(fmt(t("Even at the smallest layer height used by this printer's profiles (%.2f mm) the end speed (%.0f mm/s) exceeds the filament's maximum volumetric speed (%.1f mm³/s).\n\nThe layer height will be set to %.2f mm and the end speed lowered to %.0f mm/s.\n\nContinue?"),
                plan.layer_height, check.end, check.max_volumetric_speed, plan.layer_height, plan.end));
              if (!go) return '';
              args.end = plan.end;
              args.layer_height = plan.layer_height;
            } else if (plan.kind === 'no_candidates') {
              if (!(await clamp(t('Continue anyway?')))) return '';
            }
          } else if (!(await clamp(resize.input.checked
            ? t('Enable "Auto-adjust" to fix this automatically, or continue anyway?')
            : t('Enable "Auto-scale for nozzle" and "Auto-adjust" to fix this automatically, or continue anyway?')))) {
            return '';
          }
        }
        await make(args);
      } catch (error) {
        return error.message;
      }
      return null;
    });
  }

  // A row of two numbers after one label (the input shaping dialogs'
  // "X Start / End: " and the rest), each with its unit.
  function pair(label, unit, values) {
    const inputs = values.map((value) => el('input', { type: 'text', inputMode: 'decimal', autocomplete: 'off', spellcheck: false, value }));
    const row = el('div', { className: 'calib-field' }, el('span', { textContent: label }),
      ...inputs.flatMap((input) => [input, el('span', { className: 'quiet', textContent: unit })]));
    return { row, inputs };
  }

  // The firmware's note under the shaper types (calib_dlg.cpp; not
  // translated there).
  function firmwareNote(flavor, fallback) {
    switch (flavor) {
      case 'marlin2': case 'marlin': return 'Marlin version => 2.1.2\nFixed-Time motion not yet implemented.';
      case 'klipper': return 'Klipper version => 0.9.0';
      case 'reprapfirmware': return 'RepRap firmware version => 3.4.0\nCheck your firmware documentation for supported shaper types.';
      default: return fallback;
    }
  }

  // "Input shaping Frequency test" and "Input shaping Damp test"
  // (calib_dlg.cpp `Input_Shaping_Freq_Test_Dlg`, `Input_Shaping_Damp_Test_Dlg`).
  async function inputShaping(damping) {
    const printer = await call('calib_printer');
    const reprap = printer.flavor === 'reprapfirmware';
    const model = radios('calib-model', ['Ringing Tower', 'Fast Tower'], 0);
    const types = el('div', { className: 'calib-radios calib-two', role: 'radiogroup' }, ...printer.shaper_types.map((type, index) =>
      el('label', {}, el('input', { type: 'radio', name: 'calib-shaper', value: type, checked: index === 0 }), type)));
    const note = el('p', { className: 'quiet calib-note', textContent: firmwareNote(printer.flavor, damping ? 'Check firmware compatibility.' : 'Please ensure the selected type is compatible with your firmware version.') });
    let settings;
    let read;
    if (!damping) {
      const x = pair(reprap ? t('Frequency (Start / End): ') : `X ${t('Start / End')}: `, 'Hz', ['15', '110']);
      const y = pair(`Y ${t('Start / End')}: `, 'Hz', ['15', '110']);
      if (reprap) {
        y.row.hidden = true;
        for (const input of x.inputs) input.title = t('RepRap firmware uses the same frequency range for both axes.');
      }
      const damp = field('Damp: ');
      damp.input.value = (0.15).toFixed(3);
      settings = group('Frequency settings', x.row, y.row, damp.row,
        el('p', { className: 'quiet calib-note', textContent: t("Recommended: Set Damp to 0.\nThis will use the printer's default or saved value.") }));
      read = () => ({
        freq_start_x: readNumber({ input: x.inputs[0] }), freq_end_x: readNumber({ input: x.inputs[1] }),
        freq_start_y: readNumber({ input: y.inputs[0] }), freq_end_y: readNumber({ input: y.inputs[1] }),
        damping: readNumber(damp),
      });
    } else {
      const freq = pair(reprap ? t('Frequency: ') : `${t('Frequency')} X / Y: `, 'Hz', ['30', '30']);
      if (reprap) {
        freq.inputs[1].hidden = true;
        freq.row.lastChild.hidden = true;
        freq.inputs[0].title = t('RepRap firmware uses the same frequency for both axes.');
      }
      const damp = pair(`${t('Damp')} ${t('Start / End')}: `, '', [(0).toFixed(3), (0.4).toFixed(3)]);
      settings = group('Frequency settings', freq.row, damp.row,
        el('p', { className: 'quiet calib-note', textContent: t('Note: Use previously calculated frequencies.') }));
      read = () => ({
        freq_x: readNumber({ input: freq.inputs[0] }), freq_y: readNumber({ input: freq.inputs[1] }),
        damping_start: readNumber({ input: damp.inputs[0] }), damping_end: readNumber({ input: damp.inputs[1] }),
      });
    }
    const url = 'https://github.com/SoftFever/OrcaSlicer/wiki/input-shaping-calib';
    const link = el('a', { href: url, textContent: t('Wiki Guide: Input Shaping Calibration') });
    link.addEventListener('click', (event) => { event.preventDefault(); attempt(() => openGuide(url)); });
    const title = damping ? 'Input shaping Damp test' : 'Input shaping Frequency test';
    const dialog = el('dialog', { className: 'calib-dialog' }, el('form', { method: 'dialog' },
      el('h2', { textContent: t(title) }),
      group('Test model', model),
      group('Input shaper type', types, note),
      settings,
      el('div', { className: 'calib-links' }, link),
      el('div', { className: 'row end' },
        el('button', { type: 'submit', value: 'ok', className: 'primary', textContent: t('OK') }),
        el('button', { type: 'submit', value: 'cancel', formNoValidate: true, textContent: t('Cancel') }))));
    return run(dialog, async () => {
      const shaper = types.querySelector('input:checked');
      try {
        await make({
          test: damping ? 'shaping_damping' : 'shaping_frequency', reprap,
          model: Number(model.querySelector('input:checked').value),
          shaper: shaper ? shaper.value : '', ...read(),
        });
      } catch (error) {
        return error.message;
      }
      return null;
    });
  }

  // "Cornering test" (calib_dlg.cpp `Cornering_Test_Dlg`): junction
  // deviation (mm) on Marlin 2 with one set, else the jerk (mm/s).
  async function cornering() {
    const printer = await call('calib_printer');
    const deviation = printer.junction_deviation;
    const model = radios('calib-model', ['Ringing Tower', 'Fast Tower', 'SCV-V2'], 0);
    const unit = deviation ? 'mm' : 'mm/s';
    const start = field('Start: ', unit);
    const end = field('End: ', unit);
    start.input.value = (deviation ? 0 : 1).toFixed(3);
    end.input.value = (deviation ? 0.25 : 15).toFixed(3);
    let note = t('Note: Lower values = sharper corners but slower speeds.\n');
    if (printer.flavor === 'marlin2') {
      note += deviation
        ? t("Marlin 2 Junction Deviation detected:\nTo test Classic Jerk, set 'Maximum Junction Deviation' in Motion ability to 0.")
        : t("Marlin 2 Classic Jerk detected:\nTo test Junction Deviation, set 'Maximum Junction Deviation' in Motion ability to a value > 0.");
    } else if (printer.flavor === 'reprapfirmware') {
      note += t('RepRap detected: Jerk in mm/s.\nOrcaSlicer will convert the values to mm/min when necessary.');
    }
    const url = 'https://github.com/SoftFever/OrcaSlicer/wiki/cornering-calib';
    const link = el('a', { href: url, textContent: t('Wiki Guide: Cornering Calibration') });
    link.addEventListener('click', (event) => { event.preventDefault(); attempt(() => openGuide(url)); });
    const dialog = el('dialog', { className: 'calib-dialog' }, el('form', { method: 'dialog' },
      el('h2', { textContent: t('Cornering test') }),
      group('Test model', model),
      group('Cornering settings', el('div', { className: 'calib-pair' }, start.row, end.row),
        el('p', { className: 'quiet calib-note', textContent: note })),
      el('div', { className: 'calib-links' }, link),
      el('div', { className: 'row end' },
        el('button', { type: 'submit', value: 'ok', className: 'primary', textContent: t('OK') }),
        el('button', { type: 'submit', value: 'cancel', formNoValidate: true, textContent: t('Cancel') }))));
    return run(dialog, async () => {
      const args = {
        test: 'cornering', junction_deviation: deviation,
        model: Number(model.querySelector('input:checked').value),
        start: readNumber(start), end: readNumber(end),
      };
      // on_start: past the warning's threshold it is said, then made.
      const warning = deviation ? 0.3 : 20;
      const most = deviation ? 1 : 100;
      if (Number.isFinite(args.end) && Number.isFinite(args.start) && args.start >= 0 && args.start < args.end
          && args.end <= most && args.end > warning) {
        await inform(t('Cornering test'), fmt(t('NOTE: High values may cause Layer shift (>%s)'), warning.toFixed(3)));
      }
      try {
        await make(args);
      } catch (error) {
        return error.message;
      }
      return null;
    });
  }

  // "Stringing (temp × retraction)" (calib_dlg.cpp
  // `TempRetract_Calibration_Dlg`): the defaults from the first filament,
  // the line under the fields saying what is wrong or what will be
  // printed, the session checking the values again as the test is made.
  async function stringing() {
    const DEGREES = '\u2103';
    const d = await call('calib_stringing');
    const start = field('Start temp: ', DEGREES);
    const end = field('End temp: ', DEGREES);
    const step = field('Temp step: ', DEGREES);
    const band = field('Band height: ', 'mm');
    const rStart = field('Retraction min: ', 'mm');
    const rEnd = field('Retraction max: ', 'mm');
    const rStep = field('Retraction step: ', 'mm');
    const radius = field('Fillet radius: ', 'mm');
    start.input.value = String(Math.trunc(d.start));
    end.input.value = String(Math.trunc(d.end));
    step.input.value = shown(d.step);
    band.input.value = shown(d.band_height);
    rStart.input.value = shown(d.retract_start);
    rEnd.input.value = shown(d.retract_end);
    rStep.input.value = shown(d.retract_step);
    radius.input.value = shown(d.fillet_radius);
    radius.input.title = fmt(t('0.5 to %.1f mm: the largest radius keeps the fillet on the base plate.'), d.max_fillet_radius);
    const labels = tick('Raised labels', 'Emboss retraction values on each base and temperatures on a label tower. Off by default: the values are in the G-code comments.', d.labels);
    const wait = tick('Wait for temperature at each band (M109)', 'Use M109 (wait) at each temperature band. Off writes M104 and keeps printing while the hotend settles.', d.wait);
    const noHop = tick('Disable z-hop for this test', 'Off (default): keep your z-hop and lift settings. On: z-hop 0 for this test only.', d.no_z_hop);
    const fillet = tick('Fillet tower base', 'Add a concave fillet where each tower meets its base plate, so tall towers do not snap off at the joint.', d.fillet);
    const summary = el('p', { className: 'quiet calib-note' });
    const rangeNote = el('p', { className: 'calib-note calib-warn', textContent: t("The filament's temperature range fits only one band. Set a start and end temperature range to test."), hidden: !d.needs_range });
    // read_params: `ToDouble` of each field, the radius only with the fillet on.
    const values = () => ({
      test: 'stringing',
      start: readNumber(start), end: readNumber(end), step: readNumber(step),
      band_height: readNumber(band), retract_start: readNumber(rStart),
      retract_end: readNumber(rEnd), retract_step: readNumber(rStep),
      fillet: fillet.input.checked, fillet_radius: readNumber(radius),
      labels: labels.input.checked, wait: wait.input.checked, no_z_hop: noHop.input.checked,
    });
    async function update() {
      radius.input.disabled = !fillet.input.checked;
      try {
        summary.textContent = (await call('calib_stringing', values())).summary;
      } catch (error) {
        summary.textContent = error.message;
      }
    }
    for (const f of [start, end, step, band, rStart, rEnd, rStep, radius]) f.input.addEventListener('blur', update);
    for (const f of [labels, wait, noHop, fillet]) f.input.addEventListener('change', update);
    const dialog = el('dialog', { className: 'calib-dialog' }, el('form', { method: 'dialog' },
      el('h2', { textContent: t('Stringing (temp × retraction)') }),
      group('Settings', start.row, end.row, step.row, band.row, rStart.row, rEnd.row, rStep.row,
        labels.row, wait.row, noHop.row, fillet.row, radius.row, summary, rangeNote),
      el('p', { className: 'quiet calib-note', textContent: t('Single toolhead. Layer 1 keeps the first-layer temperature; band 0 starts at layer 2. Raft is disabled.') }),
      el('div', { className: 'row end' },
        el('button', { type: 'submit', value: 'ok', className: 'primary', textContent: t('OK') }),
        el('button', { type: 'submit', value: 'cancel', formNoValidate: true, textContent: t('Cancel') }))));
    await update();
    return run(dialog, async () => {
      try {
        await make(values());
      } catch (error) {
        return error.message;
      }
      return null;
    });
  }

  // "Flow rate" (MainFrame.cpp, Plater.cpp `calib_flowrate`): no dialog;
  // the menu item makes the test's project at once.
  // "Ironing flow test" (calib_dlg.cpp `IroningFlow_Test_Dlg`): the least
  // and most flow and the band's length; the session checks them against
  // the printer (`ironing_check`).
  function ironingFlow() {
    const start = field('Minimum ironing flow: ', '%');
    const end = field('Maximum ironing flow: ', '%');
    const length = field('Length: ', 'mm');
    start.input.value = '0';
    end.input.value = '70';
    length.input.value = '140';
    const note = el('p', { className: 'quiet calib-note', textContent: t('The front band irons from the minimum flow on the left to the maximum on the right. A second band behind it ramps the other way so you can tell ironing flow from bed tilt: if the best-looking region is at the same flow % on both bands, trust that value. If they disagree, level the bed and reprint.\n\nThe plate sits on extra solid layers so small bed height errors are less likely to show on the ironed surface. Run this after flow-ratio calibration. Check the result in reflected light: the optimum hides the surface lines without roughness from overextrusion. Lower values look matte; higher ones look glossier.') });
    const dialog = el('dialog', { className: 'calib-dialog' }, el('form', { method: 'dialog' },
      el('h2', { textContent: t('Ironing flow test') }),
      group('Settings', start.row, end.row, length.row, note),
      el('div', { className: 'row end' },
        el('button', { type: 'submit', value: 'ok', className: 'primary', textContent: t('OK') }),
        el('button', { type: 'submit', value: 'cancel', formNoValidate: true, textContent: t('Cancel') }))));
    return run(dialog, async () => {
      try {
        await make({ test: 'ironing_flow', start: readNumber(start), end: readNumber(end), length: readNumber(length) });
      } catch (error) {
        return error.message;
      }
      return null;
    });
  }

  function flowRate(test) {
    return make({ test });
  }

  // "Tutorial" (MainFrame.cpp): the calibration guide. The reference sends
  // a person in China to a mirror by the country kept in its settings;
  // this app keeps no country.
  function tutorial() {
    return openGuide('https://github.com/SoftFever/OrcaSlicer/wiki/Calibration');
  }

  return { pressureAdvance, temperature, retraction, maxFlowrate, vfa, inputShaping, cornering, stringing, ironingFlow, flowRate, tutorial, parseList };
})();
