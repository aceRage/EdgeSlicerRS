'use strict';

// The menu bar (src/api/menu.rs describes it): what each of its entries
// does, its keys, and in a browser (`--serve`) the bar itself.
//
// The session answers `menu` with the menus as they stand, in the
// language: each entry's id, its words, its key as it is shown, the key
// to bind, whether it can be used, whether it is ticked, and its
// tooltip (what it waits for when it is not built). In the app's window
// the same answer is made the system's menu bar (src/menubar.rs), which
// sends the id chosen back here as the event `menu`; in a browser the
// page draws it at its top (`#menubar`). Either way `run(id)` does it.
//
// The keys are the description's, bound here once. In the window a key
// the system's menu holds (one with Ctrl or Cmd, but Cut, Copy and Paste)
// is left to it, and comes back as its id; every other key, and in a
// browser every key, is read here. A key typed into a text field is the
// field's: only New, Open, Save, Save as, Quit and the next and previous
// project are taken from one, as they always were. Cut, Copy and Paste
// are the field's, or the text's when text is selected; in the window
// their entries are the system's, whose clipboard events are read here.
//
// Uses app.js's `call`, `pick`, `attempt`, `say`, `inform`, `confirmBox`,
// `state`, `activeId`, `newProject`, `openProject`, `selectProject`,
// `cycleProject`, `fit`, `stage3d`, `showMenu`/`hideMenu`, save.js's
// `saveProject`, `saveBeforeQuitting`, printers.js's
// `openPrinterSelection`, and the 3D editor (`window.EdgeGL.edit`).

/* global call, pick, attempt, say, inform, confirmBox, state, activeId, newProject, openProject,
   selectProject, cycleProject, fit, stage3d, saveProject, saveBeforeQuitting, openPrinterSelection,
   tauri, $, el, t, fmt, I18n, exportGcode, listbox */

const Menu = (() => {
  // The last answer of `menu`, and what it was asked with.
  let tree = null;
  let asked = '';
  let askedAt = 0;
  // What was copied (Edit > Copy, Ctrl+C): copies of objects, and the
  // project they are of; they are pasted into that project only.
  const clip = { project: null, copies: [] };

  const editor = () => (window.EdgeGL && window.EdgeGL.edit) || null;
  const stage = () => (typeof stage3d !== 'undefined' && stage3d.active ? stage3d : null);

  // What the session cannot know: the page's own state.
  function pageState() {
    const scene = stage();
    const prefs = scene ? scene.prefs : {};
    const selection = scene ? scene.selection() : new Set();
    return {
      tab: state.tab,
      selected: selection.size > 0,
      pasteable: clip.copies.length > 0 && clip.project === activeId(),
      window: Boolean(tauri),
      perspective: prefs.perspective !== false,
      auto_perspective: Boolean(prefs.autoPerspective),
      gcode_window: Boolean(prefs.gcodeWindow),
      navigator: Boolean(prefs.navigator),
      labels: Boolean(prefs.labels),
    };
  }

  // Asks for the menu again when what decides it may have changed: the
  // page's state, the project's edits, its slice; at least every few
  // seconds for what the page does not see change (a recent file gone).
  async function refresh(force = false) {
    const page = pageState();
    const project = state.project;
    const tile = (state.listing.projects || []).find((one) => one.id === activeId());
    const key = JSON.stringify([page, activeId(), state.listing.projects.map((one) => [one.id, one.name]),
      tile && [tile.dirty, tile.revision, tile.slicing], project && [project.sliced, project.edit, project.plate, project.empty]]);
    if (!force && key === asked && Date.now() - askedAt < 5000) return tree;
    asked = key;
    askedAt = Date.now();
    tree = await call('menu', page);
    if (!tauri) draw();
    return tree;
  }

  function find(id, items = tree ? tree.menus : []) {
    for (const item of items) {
      if (item.id === id) return item;
      if (item.items) {
        const found = find(id, item.items);
        if (found) return found;
      }
    }
    return null;
  }

  // ---- what each entry does ----

  const typingIn = (node) => node instanceof Element && (node.matches('input, textarea, select') || node.isContentEditable);

  // In a text field, Undo, Redo and Select all are the field's: the
  // window's menu holds their keys, so they come back here and are given
  // to the field.
  function inField(id) {
    const field = document.activeElement;
    if (!typingIn(field) || field.tagName === 'SELECT') return false;
    const command = { 'edit.undo': 'undo', 'edit.redo': 'redo', 'edit.select_all': 'selectAll' }[id];
    if (!command) return false;
    document.execCommand(command);
    return true;
  }

  async function openRecent(entry) {
    if (entry.missing) {
      // The reference says so and, the box answered, lets the entry go
      // (MainFrame.cpp `open_recent_project`; see UI_PARITY.md: its own
      // box never lets it go).
      await inform(t('Error'), t('The project is no longer available.'));
      await call('recent_remove', { path: entry.path });
      await refresh(true);
      return;
    }
    await openProject(entry.path);
  }

  // "Export all objects as one STL...": where, through the save dialog
  // (in a browser a typed path, asked about before a file is replaced).
  async function exportStl() {
    const { suggested } = await call('export_stl', { project: activeId(), suggest: true });
    const path = await pick('stl', suggested);
    if (!path) return;
    let answer = await call('export_stl', { project: activeId(), path, replace: Boolean(tauri) });
    if (answer.exists) {
      const sure = await confirmBox(t('Confirm Save As'), fmt(t('The file %s already exists\nDo you want to replace it?'), answer.path), t('Yes'), t('No'));
      if (!sure) return;
      answer = await call('export_stl', { project: activeId(), path, replace: true });
    }
    say(`Saved ${answer.path} (${answer.triangles} triangles).`);
    uncut(answer.uncut);
  }

  async function exportStls() {
    const dir = await pick('stl_folder');
    if (!dir) return;
    const answer = await call('export_stls', { project: activeId(), dir });
    say(`Saved ${answer.files.length} STL files in ${dir}.`);
    uncut(answer.uncut);
  }

  // Objects whose negative parts were not cut out: as the reference says
  // when it cannot cut them.
  function uncut(names) {
    if (names && names.length) inform(t('Export'), `${t('Unable to perform boolean operation on model meshes. Only positive parts will be exported.')}\n\n${names.join('\n')}`);
  }

  // The user presets' import and export (another branch): a function of
  // the page's that does it, when there is one; else the session's call,
  // given a file or a folder.
  async function presets(which) {
    const names = (tree && tree.preset_calls) || [];
    const own = which === 'import' ? window.presetImport : window.presetExport;
    if (typeof own === 'function') { await own(); return; }
    const name = names.find((one) => (which === 'import' ? /import/ : /export/).test(one));
    if (!name) return;
    const path = await pick(which === 'import' ? 'presets' : 'folder');
    if (!path) return;
    await call(name, which === 'import' ? { path } : { dir: path });
  }

  function copySelection() {
    const scene = stage();
    if (!scene || state.tab !== 'prepare' || !scene.selection().size || (editor() && editor().partMode())) return false;
    clip.copies = [...scene.selection()];
    clip.project = activeId();
    refresh();
    return true;
  }

  function paste() {
    const edit = editor();
    if (!edit || state.tab !== 'prepare' || !clip.copies.length || clip.project !== activeId()) return false;
    edit.clone(clip.copies, true);
    return true;
  }

  function selectAll() {
    const scene = stage();
    if (!scene || state.tab !== 'prepare' || !scene.scene.data) return false;
    // The objects on the plate shown.
    const id = scene.plateId();
    scene.select(scene.scene.data.placings.filter((placing) => placing.plate === id || scene.scene.plates.length === 1).map((placing) => placing.id));
    return true;
  }

  function deleteAll() {
    const scene = stage();
    const edit = editor();
    if (!scene || !edit || state.tab !== 'prepare' || !scene.scene.data) return false;
    const copies = scene.scene.data.placings.map((placing) => placing.id);
    if (!copies.length) return false;
    edit.commit({ op: 'delete', copies });
    return true;
  }

  function onPlater() { return state.tab === 'prepare' || state.tab === 'preview'; }

  const VIEWS = {
    'view.default': 'default', 'view.top': 'top', 'view.bottom': 'bottom', 'view.front': 'front',
    'view.rear': 'rear', 'view.left': 'left', 'view.right': 'right',
  };

  // Does what `id` stands for. False when there was nothing to do (a key
  // is then left to whatever else reads it).
  function run(id) {
    if (inField(id)) return true;
    // A dialog of the page's is open: the menu waits for it.
    if (document.querySelector('dialog[open]')) return false;
    const scene = stage();
    const edit = editor();
    const go = (work) => { attempt(work); return true; };
    if (id.startsWith('recent:')) {
      const entry = tree && tree.recent[Number(id.slice(7))];
      return entry ? go(() => openRecent(entry)) : false;
    }
    if (id.startsWith('project:')) return go(() => selectProject(Number(id.slice(8))));
    if (VIEWS[id]) {
      if (!scene || !onPlater()) return false;
      scene.view(VIEWS[id]);
      if (id === 'view.default') fit('bed');
      return true;
    }
    switch (id) {
      case 'file.new': return go(newProject);
      case 'file.open': return go(() => openProject());
      case 'file.save': return go(() => saveProject(activeId()));
      case 'file.save_as': return go(() => saveProject(activeId(), true));
      case 'file.quit':
        if (!tauri) return false;
        return go(async () => { if (await saveBeforeQuitting()) await tauri.core.invoke('close_window'); });
      case 'import.model':
        if (!edit || state.tab !== 'prepare') return false;
        return go(() => edit.add());
      case 'import.configs': return go(() => presets('import'));
      case 'export.preset_bundle': return go(() => presets('export'));
      case 'export.stl': return go(exportStl);
      case 'export.stls': return go(exportStls);
      case 'export.gcode': return go(exportGcode);
      case 'edit.undo': case 'edit.redo':
        if (!edit || state.tab !== 'prepare') return false;
        return go(() => edit.step(id === 'edit.undo' ? 'undo' : 'redo'));
      case 'edit.cut':
        if (!copySelection()) return false;
        return go(() => edit.remove());
      case 'edit.copy': return copySelection();
      case 'edit.paste': return paste();
      case 'edit.delete':
        if (!edit || !scene || state.tab !== 'prepare' || !scene.selection().size) return false;
        return go(() => edit.remove());
      case 'edit.delete_all': return deleteAll();
      case 'edit.clone':
        if (!edit || !scene || state.tab !== 'prepare' || !scene.selection().size || edit.partMode()) return false;
        return go(() => edit.clone([...scene.selection()]));
      case 'edit.duplicate_plate': {
        const project = state.project;
        if (!edit || !project || state.tab !== 'prepare') return false;
        return go(() => edit.commit({ op: 'plate_duplicate', plate: project.plate ?? 1 }, { select: true }));
      }
      case 'edit.select_all': return selectAll();
      case 'edit.deselect_all':
        if (!scene || state.tab !== 'prepare') return false;
        // A tool that is open is closed first (Escape, as the reference).
        if (edit && edit.escape()) return true;
        if (!scene.selection().size) return false;
        scene.select([]);
        return true;
      case 'view.fit':
        if (!onPlater()) return false;
        fit('part');
        return true;
      case 'view.perspective': case 'view.orthogonal':
        if (!scene) return false;
        scene.setPref('perspective', 'perspective', id === 'view.perspective');
        refresh(true);
        return true;
      case 'view.auto_perspective':
        if (!scene) return false;
        scene.setPref('autoPerspective', 'auto_perspective', !scene.prefs.autoPerspective);
        refresh(true);
        return true;
      case 'view.gcode_window':
        if (!scene || state.tab !== 'preview') return false;
        scene.toggleGcode();
        refresh(true);
        return true;
      case 'view.navigator':
        if (!scene) return false;
        scene.setPref('navigator', 'navigator', !scene.prefs.navigator);
        refresh(true);
        return true;
      case 'view.labels':
        if (!scene || !onPlater()) return false;
        scene.setPref('labels', 'labels', !scene.prefs.labels);
        refresh(true);
        return true;
      case 'window.next': case 'window.previous':
        return go(() => cycleProject(id === 'window.next' ? 1 : -1));
      case 'help.shortcuts': return go(showShortcuts);
      case 'help.wizard': return go(openPrinterSelection);
      case 'app.about': case 'help.about': return go(showAbout);
      case 'app.preferences': return go(showPreferences);
      default: return false;
    }
  }

  // ---- the keys ----

  const CLIPBOARD = ['edit.cut', 'edit.copy', 'edit.paste'];
  let keyedClip = null;
  const textChosen = () => !document.getSelection().isCollapsed;

  // Keys taken from a text field too: none of them is the field's.
  const ANYWHERE = new Set(['file.new', 'file.open', 'file.save', 'file.save_as', 'file.quit', 'window.next', 'window.previous']);

  function matches(bind, event) {
    if (Boolean(event.ctrlKey || event.metaKey) !== bind.command || event.altKey) return false;
    const key = bind.key;
    if (key === '?') return event.key === '?' || (event.shiftKey && event.code === 'Slash');
    if (event.shiftKey !== bind.shift) return false;
    if (/^[0-9]$/.test(key)) return event.code === `Digit${key}` || event.code === `Numpad${key}`;
    if (key === 'Delete') return event.key === 'Delete' || event.key === 'Backspace';
    if (key === ']') return event.key === ']' || event.key === '}';
    if (key === '[') return event.key === '[' || event.key === '{';
    if (key.length === 1) return event.key.toLowerCase() === key.toLowerCase();
    return event.key === key;
  }

  function keyed(event) {
    if (!tree) return null;
    const all = [];
    const walk = (items) => { for (const item of items) { if (item.bind) all.push(item); if (item.items) walk(item.items); } };
    walk(tree.menus);
    return all.find((item) => matches(item.bind, event)) || null;
  }

  document.addEventListener('keydown', (event) => {
    if (document.querySelector('dialog[open]')) return;
    // A menu that is open takes Escape first.
    if (event.key === 'Escape' && (!$('menu').hidden || openMenu)) return;
    const item = keyed(event);
    if (!item) return;
    // In the window the system's menu holds this key, and sends its id.
    if (tauri && item.accelerator) return;
    if (typingIn(event.target) && !ANYWHERE.has(item.id)) return;
    // Text selected: the clipboard's keys are the text's.
    if (CLIPBOARD.includes(item.id) && textChosen()) return;
    // Not built: its key does nothing, and nothing else has it. Whether
    // any other entry can be used is asked of the page as the key comes
    // (`run`), the menu's answer being a moment old.
    if (item.waits) {
      if (item.bind.command) event.preventDefault();
      return;
    }
    if (run(item.id)) {
      event.preventDefault();
      if (CLIPBOARD.includes(item.id)) keyedClip = { id: item.id, at: Date.now() };
    }
  }, true);

  // The menu's ids from the window's menu bar.
  if (tauri) tauri.event.listen('menu', (event) => run(String(event.payload)));

  // Cut, Copy and Paste of objects: their keys are read above where no
  // field has them and no text is selected; the window's Edit menu has
  // the system's own entries, which send the clipboard's events, read
  // here (once: not again for a key just read above).
  for (const kind of ['copy', 'cut', 'paste']) {
    document.addEventListener(kind, (event) => {
      if (typingIn(event.target) || (kind !== 'paste' && textChosen()) || document.querySelector('dialog[open]')) return;
      if (keyedClip && keyedClip.id === `edit.${kind}` && Date.now() - keyedClip.at < 500) { event.preventDefault(); return; }
      if (run(`edit.${kind}`)) event.preventDefault();
    });
  }

  // ---- the bar in a browser ----

  let openMenu = null;

  // While a menu is open it is left as it is, and drawn anew once it is
  // closed: what is under the pointer does not move.
  let stale = false;
  function draw() {
    const bar = $('menubar');
    if (!bar || !tree) return;
    if (openMenu) { stale = true; return; }
    stale = false;
    document.body.classList.add('has-menubar');
    bar.hidden = false;
    bar.replaceChildren(...tree.menus.map((menu) => {
      const button = el('button', { type: 'button', className: 'menubar-top', textContent: menu.label });
      button.dataset.id = menu.id;
      button.setAttribute('aria-haspopup', 'menu');
      button.setAttribute('aria-expanded', 'false');
      button.addEventListener('click', () => (openMenu === button ? closeMenus() : openTop(button, false)));
      button.addEventListener('pointerenter', () => { if (openMenu && openMenu !== button) openTop(button, false); });
      button.addEventListener('keydown', (event) => {
        if (event.key === 'ArrowDown' || event.key === 'Enter' || event.key === ' ') { event.preventDefault(); openTop(button, true); }
        if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') { event.preventDefault(); neighbour(button, event.key === 'ArrowRight' ? 1 : -1, false); }
      });
      return button;
    }));
  }

  function neighbour(button, step, focusInside) {
    const tops = [...$('menubar').querySelectorAll('.menubar-top')];
    const next = tops[(tops.indexOf(button) + step + tops.length) % tops.length];
    if (openMenu) openTop(next, focusInside); else next.focus();
  }

  function panels() { return [...document.querySelectorAll('.menubar-panel')]; }

  function closeMenus() {
    for (const panel of panels()) panel.remove();
    if (openMenu) openMenu.setAttribute('aria-expanded', 'false');
    openMenu = null;
    if (stale) draw();
  }

  // A menu's entries, in a panel under (or beside) what opened it.
  function panelOf(items, depth) {
    const panel = el('div', { className: 'menu menubar-panel' });
    panel.setAttribute('role', 'menu');
    panel.dataset.depth = depth;
    for (const item of items) {
      if (item.line) { panel.append(el('hr')); continue; }
      const off = item.enabled === false;
      const row = el('button', { type: 'button', className: 'menubar-item' });
      row.dataset.id = item.id;
      // A recent project's path is cut at its start, so its file name
      // stays in sight; it is whole in its tooltip.
      let label = item.label;
      if (item.path && label.length > 72) label = `${label.split(' ', 1)[0]} …${item.path.slice(-64)}`;
      row.append(el('span', { className: 'menubar-label', textContent: label }));
      if (item.key) row.append(el('span', { className: 'menubar-key', textContent: item.key }));
      if (item.items) {
        row.classList.add('has-sub');
        row.setAttribute('aria-haspopup', 'menu');
        row.append(el('span', { className: 'sub-mark' }));
      }
      if (item.checked !== null && item.checked !== undefined) {
        row.setAttribute('role', item.radio ? 'menuitemradio' : 'menuitemcheckbox');
        row.setAttribute('aria-checked', String(item.checked));
      } else {
        row.setAttribute('role', 'menuitem');
      }
      // Off, but its tooltip still comes up: what it waits for.
      if (off) row.setAttribute('aria-disabled', 'true');
      if (item.waits) row.classList.add('waits');
      if (item.tip) row.title = item.tip;
      if (item.path) row.title = item.missing ? `${item.path}\n${t('File is missing')}` : item.path;
      if (item.missing) row.classList.add('missing');
      const openSub = (focus) => {
        if (off) return;
        for (const deeper of panels().filter((one) => Number(one.dataset.depth) > depth)) deeper.remove();
        const sub = panelOf(item.items, depth + 1);
        document.body.append(sub);
        const at = row.getBoundingClientRect();
        const box = sub.getBoundingClientRect();
        const right = at.right + box.width + 4 <= window.innerWidth;
        sub.style.left = `${right ? at.right + 2 : Math.max(4, at.left - box.width - 2)}px`;
        sub.style.top = `${Math.max(4, Math.min(at.top - 4, window.innerHeight - box.height - 4))}px`;
        sub.from = row;
        if (focus) focusFirst(sub);
      };
      row.addEventListener('pointerenter', () => {
        for (const deeper of panels().filter((one) => Number(one.dataset.depth) > depth)) {
          if (deeper.from !== row) deeper.remove();
        }
        if (item.items && !panels().some((one) => one.from === row)) openSub(false);
      });
      row.addEventListener('click', () => {
        if (off) return;
        if (item.items) { openSub(true); return; }
        closeMenus();
        run(item.id);
      });
      row.addEventListener('keydown', (event) => {
        const rows = [...panel.querySelectorAll('.menubar-item')];
        const at = rows.indexOf(row);
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
          event.preventDefault();
          rows[(at + (event.key === 'ArrowDown' ? 1 : rows.length - 1)) % rows.length].focus();
        } else if (event.key === 'ArrowRight') {
          event.preventDefault();
          if (item.items) openSub(true); else neighbour(openMenu, 1, true);
        } else if (event.key === 'ArrowLeft') {
          event.preventDefault();
          if (depth > 0) { panel.remove(); if (panel.from) panel.from.focus(); } else neighbour(openMenu, -1, true);
        } else if (event.key === 'Escape') {
          event.preventDefault();
          event.stopPropagation();
          if (depth > 0) { panel.remove(); if (panel.from) panel.from.focus(); } else { const top = openMenu; closeMenus(); top.focus(); }
        } else if (event.key === 'Home' || event.key === 'End') {
          event.preventDefault();
          rows[event.key === 'Home' ? 0 : rows.length - 1].focus();
        }
      });
      panel.append(row);
    }
    return panel;
  }

  function focusFirst(panel) {
    const first = panel.querySelector('.menubar-item:not([aria-disabled="true"])') || panel.querySelector('.menubar-item');
    if (first) first.focus();
  }

  function openTop(button, focusInside) {
    const id = button.dataset.id;
    closeMenus();
    // The bar may have been drawn anew as the last menu closed.
    button = $('menubar').querySelector(`[data-id="${CSS.escape(id)}"]`) || button;
    const menu = tree && tree.menus.find((one) => one.id === id);
    if (!menu) return;
    openMenu = button;
    button.setAttribute('aria-expanded', 'true');
    const panel = panelOf(menu.items, 0);
    document.body.append(panel);
    const at = button.getBoundingClientRect();
    const box = panel.getBoundingClientRect();
    panel.style.left = `${Math.max(4, Math.min(at.left, window.innerWidth - box.width - 4))}px`;
    panel.style.top = `${at.bottom + 2}px`;
    panel.style.maxHeight = `${window.innerHeight - at.bottom - 8}px`;
    if (focusInside) focusFirst(panel); else button.focus();
  }

  document.addEventListener('pointerdown', (event) => {
    if (!openMenu) return;
    if (event.target.closest('.menubar-panel, #menubar')) return;
    closeMenus();
  });
  window.addEventListener('blur', () => closeMenus());
  window.addEventListener('resize', () => closeMenus());

  // ---- the windows the menu opens ----

  // "Keyboard Shortcuts" (KBShortcutsDialog.cpp): the reference's lists,
  // the keys as this system shows them; what is not built here is dimmed.
  async function showShortcuts() {
    const lists = await call('shortcuts');
    const dialog = $('shortcuts');
    $('shortcuts-title').textContent = lists.title;
    $('shortcuts-body').replaceChildren(...lists.sections.map((section) => el('section', {},
      el('h3', { className: 'caps', textContent: section.title }),
      el('dl', {}, ...section.rows.flatMap((row) => {
        const key = el('dt', { textContent: row.key });
        const does = el('dd', { textContent: row.does });
        if (!row.built) {
          key.className = 'off';
          does.className = 'off';
          does.title = 'not yet';
        }
        return [key, does];
      })))));
    dialog.showModal();
  }

  // "About EdgeSlicerRS".
  function showAbout() {
    const item = find('app.about') || find('help.about');
    $('about-title').textContent = item ? item.label : 'EdgeSlicerRS';
    $('about').showModal();
  }

  // "Preferences": of the reference's, what this app has (Preferences.cpp,
  // General: "Language"). Another language reloads the page in it.
  let prefsList = false;
  function showPreferences() {
    const select = $('prefs-language');
    select.replaceChildren(...I18n.languages.map((language) => new Option(language.name, language.code)));
    select.value = I18n.language;
    select.dataset.value = I18n.language;
    // The page's own list, not the system's (STYLE.md).
    if (!prefsList) { listbox(select, { bare: true }); prefsList = true; }
    const dialog = $('prefs');
    dialog.returnValue = 'cancel';
    dialog.addEventListener('close', () => {
      if (dialog.returnValue === 'ok' && select.value !== I18n.language) attempt(() => I18n.choose(select.value));
    }, { once: true });
    dialog.showModal();
  }

  return { refresh, run, pageState, get tree() { return tree; } };
})();

// The first menu, and again as the session changes (app.js `tick` calls
// `Menu.refresh` with each listing).
attempt(() => Menu.refresh(true));
