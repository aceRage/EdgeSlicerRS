'use strict';

// The "Objects" side of the Process section: the settings of the object,
// the part or the height range selected (ParamsPanel.cpp `m_mode_region`,
// "Global" / "Objects"; Tab.cpp `TabPrintObject`, `TabPrintPart`,
// `TabPrintLayer`), and the heights of an object's ranges when "Layers" is
// selected (GUI_ObjectLayers.cpp). The pages are the process's, drawn by
// settings.js, with only the options that item may set, the reference's
// "Frequent" page first; each change is one `scene_edit`, so one step of
// the undo history.
//
// Loaded after settings.js and before app.js, whose `call`, `$`, `el`,
// `state`, `attempt` and the rest are used once the page runs.

// Every word this adds to the page. The reference's go through the
// translation layer as its message ids, as written in its source (`t`,
// i18n.js; this script runs once the language's table is in); the rest
// are this app's, listed in src/api/i18n/wording.txt and shown in
// English in every language.
const OBJECT_WORDS = {
  // ParamsPanel.cpp `create_layout`.
  global: t('Global'),
  objects: t('Objects'),
  // Tab.cpp `m_tt_value_unlock`, on an item's own value.
  revert: t('Click to reset current value and attach to the global value.'),
  // GUI_ObjectList.cpp, the editing column's tooltip.
  resetAll: t('Click the icon to reset all settings of the object'),
  // Tab.cpp, the global pages' reset of all (as index.html has it).
  resetPreset: t('Click to reset all settings to the last saved preset.'),
  // ObjectDataViewModel.cpp: "Layers", and a range's row, which the
  // reference builds of pieces: _(L("Range")) + " %.2f-%.2f " + "(" + _("mm") + ")".
  layers: t('Layers'),
  range: t('Range'),
  mm: t('mm'),
  // GUI_Factories.cpp: the object menu.
  heightRange: t('Height range Modifier'),
  editProcess: t('Edit Process Settings'),
  // GUI_ObjectLayers.cpp.
  addRange: t('Add height range'),
  removeRange: t('Remove height range'),
  invalid: t('Invalid numeric.'),
  // ObjectDataViewModel.cpp's kinds of row.
  object: t('Object'),
  part: t('Part'),
  modifier: t('Modifier'),
  // This app's own.
  revertLabel: 'reset to the value over it',
  modeLabel: 'Whose process settings are shown',
  pick: 'Select an object, a part or a height range in the object list to set settings of its own.',
  several: 'Several items are selected: select one to set its settings.',
  helper: 'A negative part, a support blocker, a support enforcer and a Precise Seam part have no settings of their own.',
  noRanges: 'The object has no height ranges.',
  ownMarked: 'Objects have settings of their own',
  bottom: 'Bottom of the range',
  top: 'Top of the range',
};

// The mark of an item with settings of its own (three sliders).
const OWN_ICON = 'M3,4.5h10M3,8h10M3,11.5h10M6,3v3M10.5,6.5v3M5,10v3';

// Whose settings the Process section shows, the reference's switch.
const objectsMode = { on: false };
// The pages of the item selected.
const itemPages = Object.assign(settingsBook('item', 'process'), {
  item: null,
  revert: OBJECT_WORDS.revert,
  revertLabel: OBJECT_WORDS.revertLabel,
});

// The pages the Process section shows now.
const processBook = () => (objectsMode.on ? itemPages : processPages);

const itemArgs = (item) => {
  if (!item) return {};
  const args = { object: item.object };
  if (item.part !== undefined) args.part = item.part;
  if (item.range !== undefined) args.range = item.range;
  return args;
};

const rangeName = (range) => `${OBJECT_WORDS.range} ${range.min_z.toFixed(2)}-${range.max_z.toFixed(2)} (${OBJECT_WORDS.mm})`;

// What is selected, as the reference's tabs take it
// (GUI_ObjectSettings.cpp `update_settings_list`): one object, one part
// or modifier, one height range or "Layers"; else why there is nothing.
function selectedItem() {
  const GL = window.EdgeGL;
  const edit = GL.edit;
  const data = edit && edit.data();
  if (!data) return null;
  const list = GL.objects;
  if (list && list.range) return list.range;
  if (edit.stage.parts().size) {
    const items = edit.partItems();
    if (items.length !== 1) return { why: OBJECT_WORDS.several };
    const part = items[0].part;
    if (part.kind !== 'part' && part.kind !== 'modifier') return { why: OBJECT_WORDS.helper };
    return { object: items[0].objectId, part: part.id };
  }
  const selection = edit.stage.selection();
  if (!selection.size) return null;
  const objects = new Set(data.placings.filter((placing) => selection.has(placing.id)).map((placing) => data.objects[placing.object].id));
  if (objects.size !== 1) return { why: OBJECT_WORDS.several };
  return { object: [...objects][0] };
}

// The object an item is of, as the scene has it.
function sceneObject(id) {
  const data = window.EdgeGL.edit && window.EdgeGL.edit.data();
  return data ? data.objects.find((object) => object.id === id) : null;
}

// What the head says the pages are of.
function itemName(item) {
  const found = sceneObject(item.object);
  if (!found) return '';
  // An object without a name of its own is listed by its file's (objects.js).
  const object = { ...found, name: found.name || (state.project && state.project.file) || OBJECT_WORDS.object };
  if (item.layers) return `${object.name} › ${OBJECT_WORDS.layers}`;
  if (item.range !== undefined) {
    const range = (object.ranges || [])[item.range];
    return range ? `${object.name} › ${rangeName(range)}` : object.name;
  }
  if (item.part !== undefined) {
    const parts = object.part_list || [];
    const at = parts.findIndex((held) => held.id === item.part);
    const part = parts[at];
    // As the list names a part without a name of its own (objects.js).
    const kind = part && part.kind === 'modifier' ? OBJECT_WORDS.modifier : OBJECT_WORDS.part;
    return `${object.name} › ${(part && part.name) || `${kind} ${at + 1}`}`;
  }
  return object.name;
}

// The pages of an item: the reference's "Frequent" page, then the
// process's, each with only what the item may set; a group, a line or a
// page with nothing left is left out, as `TabPrintModel::build` leaves it.
function itemBookPages(book) {
  const data = book.data;
  const answer = book.answer;
  if (!data || !answer) return [];
  const keys = new Set(answer.keys);
  const frequent = data.items && data.items.frequent;
  const all = [];
  if (frequent) all.push({ title: frequent.title, groups: [{ title: '', lines: frequent.keys.map((key) => ({ keys: [key] })) }], extruder: null });
  for (const page of data.pages) all.push({ title: page.title, groups: page.groups, extruder: null });
  return all.map((page) => ({
    ...page,
    groups: page.groups
      .map((group) => ({ ...group, lines: group.lines.map((line) => ({ ...line, keys: line.keys.filter((key) => keys.has(key)) })).filter((line) => line.keys.length) }))
      .filter((group) => group.lines.length),
  })).filter((page) => page.groups.length);
}

// The head of the section in either mode.
function showModeHead(item) {
  const on = objectsMode.on;
  for (const [id, mine] of [['process-global', !on], ['process-objects', on]]) $(id).setAttribute('aria-pressed', String(mine));
  // The preset is the project's ("Global"); an item has none.
  const choices = state.project && state.project.choices;
  const fromPresets = Boolean(choices) && choices.source === 'presets';
  $('process-row').hidden = on || !fromPresets;
  $('saved-process').hidden = on || fromPresets;
  // The preset's cog (presets.js) is the global settings', as the row is:
  // an item's own values are not the preset's edits.
  if ($('process-cog')) $('process-cog').hidden = on || !fromPresets;
  const line = $('process-item');
  line.hidden = !on;
  line.textContent = on && item && item.object !== undefined ? itemName(item) : '';
  line.title = line.textContent;
  $('process-reset').title = on ? OBJECT_WORDS.resetAll : OBJECT_WORDS.resetPreset;
  markObjectsSwitch();
}

// "Objects" is marked while anything has settings of its own, as the
// reference colours the switch's word (`notify_object_config_changed`).
function markObjectsSwitch() {
  const data = window.EdgeGL.edit && window.EdgeGL.edit.data();
  const any = Boolean(data) && data.objects.some((object) => object.settings || (object.part_list || []).some((part) => part.settings) || (object.ranges || []).length);
  $('process-objects').classList.toggle('set', any);
  $('process-objects').setAttribute('aria-description', any ? OBJECT_WORDS.ownMarked : '');
}

// What stands in place of the pages when there are none to show.
function showNoPages(text) {
  for (const id of ['process-tabs', 'process-search', 'process-found', 'process-refused']) $(id).hidden = true;
  $('process-reset').hidden = true;
  $('process-none').hidden = true;
  $('process-page').replaceChildren(el('p', { className: 'quiet item-hint', textContent: text }));
}

// The Process section in "Objects" mode.
async function showItemSettings() {
  const item = selectedItem();
  showModeHead(item);
  if (!item || item.why) { showNoPages(item ? item.why : OBJECT_WORDS.pick); return; }
  if (item.layers) { showRanges(item.object); return; }
  $('process-search').hidden = false;
  // Another kind of item has other options: its pages are built anew.
  const kind = item.part !== undefined ? 'part' : (item.range !== undefined ? 'range' : 'object');
  if (itemPages.shownKind !== kind) {
    itemPages.built.clear();
    itemPages.rows.clear();
    itemPages.shownKind = kind;
  }
  itemPages.item = item;
  await showSettings(itemPages);
}

// A change on an item's pages (settings.js `setSettings`): one
// `scene_edit`, whose answer the editor puts on the page.
async function itemSet(book, args) {
  const item = book.item;
  if (!item) return;
  const op = args.reset ? { op: 'item_reset', ...itemArgs(item) } : { op: 'item_set', ...itemArgs(item), key: args.key, value: args.value };
  await itemEdit(op);
}

async function itemEdit(op) {
  const project = state.project;
  const answer = await call('scene_edit', { project: project.id, ...op });
  window.EdgeGL.edit.take(answer);
  await showProcess();
}

// The heights of an object's ranges ("Layers" selected): a row for each,
// "Range" [bottom] - [top] mm, with "Remove height range" and "Add height
// range" (GUI_ObjectLayers.cpp `create_layer`).
function showRanges(objectId) {
  const object = sceneObject(objectId);
  const ranges = (object && object.ranges) || [];
  if (!ranges.length) { showNoPages(OBJECT_WORDS.noRanges); return; }
  for (const id of ['process-tabs', 'process-search', 'process-found', 'process-refused', 'process-none', 'process-reset']) $(id).hidden = true;
  const height = (value, label, commit) => {
    const input = el('input', { type: 'text', className: 'number', value: String(value), autocomplete: 'off', spellcheck: false });
    input.inputMode = 'decimal';
    input.setAttribute('aria-label', label);
    input.addEventListener('change', () => attempt(async () => {
      const text = input.value.trim();
      const number = /^[-+]?(\d+\.?\d*|\.\d+)$/.test(text) ? Number(text) : NaN;
      if (!Number.isFinite(number) || number < 0) { input.value = String(value); throw new Error(OBJECT_WORDS.invalid); }
      try {
        await commit(number);
      } catch (refused) {
        input.value = String(value);
        throw refused;
      }
    }));
    input.addEventListener('keydown', (event) => { if (event.key === 'Enter') input.blur(); });
    return el('span', { className: 'unit' }, input, ` ${OBJECT_WORDS.mm}`);
  };
  const tool = (title, path, act) => {
    const button = el('button', { type: 'button', className: 'icon tool', title, ariaLabel: title }, svg(path));
    button.addEventListener('click', () => attempt(act));
    return button;
  };
  const rows = ranges.map((range, at) => {
    const set = (key) => (number) => itemEdit({ op: 'range_set', object: objectId, range: at, [key]: number });
    return el('div', { className: 'range-row' },
      el('span', { className: 'range-label', textContent: OBJECT_WORDS.range }),
      height(range.min_z, `${OBJECT_WORDS.bottom} ${at + 1}`, set('min_z')),
      el('span', { className: 'range-dash', textContent: '-' }),
      height(range.max_z, `${OBJECT_WORDS.top} ${at + 1}`, set('max_z')),
      tool(OBJECT_WORDS.removeRange, 'M4,8h8', () => itemEdit({ op: 'range_delete', object: objectId, range: at })),
      tool(OBJECT_WORDS.addRange, 'M4,8h8M8,4v8', () => itemEdit({ op: 'range_add', object: objectId, range: at })));
  });
  $('process-page').replaceChildren(el('div', { className: 'process-page range-page' }, ...rows));
}

// "Height range Modifier" (GUI_Factories.cpp
// `append_menu_item_layers_editing`, GUI_ObjectList.cpp `layers_editing`):
// an object without ranges is given one from 0 to 2 mm; "Layers" is
// selected and the section shows "Objects".
async function heightRange(objectId, ids) {
  const object = sceneObject(objectId);
  if (object && !(object.ranges || []).length) {
    const project = state.project;
    window.EdgeGL.edit.take(await call('scene_edit', { project: project.id, op: 'range_add', object: objectId }));
  }
  objectsMode.on = true;
  if (window.EdgeGL.objects) window.EdgeGL.objects.pickRange({ object: objectId, layers: true }, ids);
  await showProcess();
}

// "Edit Process Settings": the section shows "Objects" for what is
// selected (GUI_ObjectList.cpp `switch_to_object_process`).
async function editProcess() {
  objectsMode.on = true;
  await showProcess();
}

// A click on an item's mark in the list: all its settings taken away.
function resetItem(item) {
  attempt(() => itemEdit({ op: 'item_reset', ...itemArgs(item) }));
}

// The list's selection changed: the pages follow, once for a burst.
let itemPending = false;
function itemSelected() {
  markObjectsSwitch();
  if (!objectsMode.on || itemPending) return;
  itemPending = true;
  setTimeout(() => {
    itemPending = false;
    if (objectsMode.on && state.project) attempt(showProcess);
  }, 0);
}

window.ObjectSettings = {
  WORDS: OBJECT_WORDS,
  OWN_ICON,
  rangeName,
  reset: resetItem,
  heightRange: (object, ids) => attempt(() => heightRange(object, ids)),
  editProcess: () => attempt(editProcess),
  selected: itemSelected,
};

document.addEventListener('scripts-loaded', () => {
  const group = $('process-mode-switch');
  group.setAttribute('aria-label', OBJECT_WORDS.modeLabel);
  $('process-global').textContent = OBJECT_WORDS.global;
  $('process-objects').textContent = OBJECT_WORDS.objects;
  const pick = (on) => attempt(async () => {
    if (objectsMode.on === on) return;
    objectsMode.on = on;
    $('process-search').value = '';
    await showProcess();
  });
  $('process-global').addEventListener('click', () => pick(false));
  $('process-objects').addEventListener('click', () => pick(true));
});
