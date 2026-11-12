'use strict';

// The object list of the sidebar: every plate with the objects on it, as
// the reference's list has them (GUI_ObjectList.cpp,
// ObjectDataViewModel.cpp): "Name", whether it is to be printed, and the
// filament ("Fila."). What is selected here is what is selected in the 3D
// scene, both ways. It is drawn from the scene the stage holds, and every
// change goes through scene-edit.js as one `scene_edit` call.
(function (GL) {
  // What a part is, in the reference's words (ModelVolumeType).
  const PART_KINDS = { part: 'Part', negative: 'Negative Part', modifier: 'Modifier', support_blocker: 'Support Blocker', support_enforcer: 'Support Enforcer', precise_seam: 'Precise Seam' };
  // Each kind's own mark before a part's name, as the reference marks
  // its volumes (`ADD_VOLUME_MENU_ITEMS`' icons): the page's own drawings.
  const KIND_ICONS = {
    part: 'M8,1.5l6,3v7l-6,3l-6-3v-7ZM2,4.5l6,3l6-3M8,7.5v7',
    negative: 'M2.5,2.5h11v11h-11ZM5,8h6',
    modifier: 'M2.5,2.5h11v11h-11ZM5,6h6M5,10h6M7,4.5v3M9,8.5v3',
    support_blocker: 'M8,1.5a6.5,6.5,0,1,1,0,13a6.5,6.5,0,1,1,0-13ZM3.4,3.4l9.2,9.2',
    support_enforcer: 'M8,1.5a6.5,6.5,0,1,1,0,13a6.5,6.5,0,1,1,0-13ZM8,4.5v7M4.5,8h7',
    precise_seam: 'M2.5,2.5h11v11h-11ZM8,2.5v2M8,6.5v3M8,11.5v2',
  };
  // GUI_ObjectList.cpp `check_last_selection`: one kind of row at a time.
  const CONFLICT_OBJECT = 'Selection conflicts\n\nIf the first selected item is an object, the second should also be an object.';
  const CONFLICT_PART = 'Selection conflicts\n\nIf the first selected item is a part, the second should be a part in the same object.';
  const icon = (path) => {
    const node = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    node.setAttribute('viewBox', '0 0 16 16');
    node.setAttribute('aria-hidden', 'true');
    node.classList.add('obj-kind');
    const line = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    line.setAttribute('d', path);
    node.append(line);
    return node;
  };
  const css = (colour) => `rgb(${colour.map((part) => Math.round(part * 255)).join(',')})`;

  const objects = {
    host: null,
    // The row a click last selected, for Shift to select from.
    anchor: null,

    init(host) {
      this.host = host;
      this.node = document.getElementById('objects');
      this.none = document.getElementById('objects-none');
      // Without the 3D stage there is no scene to list or to edit.
      if (!this.node || !GL.edit) {
        const section = document.getElementById('s-objects');
        if (section) section.hidden = true;
        return;
      }
      GL.objects = this;
      this.render();
    },

    // The rows there are: a plate's objects, then what is on no plate.
    groups(data) {
      const rows = (wanted) => {
        const out = [];
        data.objects.forEach((object, index) => {
          const all = data.placings.filter((placing) => placing.object === index);
          const here = all.filter(wanted);
          if (!here.length) return;
          out.push({ object, index, copies: here, child: false });
          // An object of several parts lists each under it (the reference's
          // volumes), before its instances.
          const parts = object.part_list || [];
          if (parts.length > 1) parts.forEach((part, at) => out.push({ object, index, copies: here, child: true, part, number: at + 1 }));
          // An object with several copies lists each under it.
          if (all.length > 1) here.forEach((copy) => out.push({ object, index, copies: [copy], child: true, number: all.indexOf(copy) + 1 }));
        });
        return out;
      };
      if (!data.plates.length) return [{ plate: null, rows: rows(() => true) }];
      const groups = data.plates.map((plate) => ({ plate, rows: rows((placing) => placing.plate === plate.id) }));
      const outside = rows((placing) => placing.plate === null);
      if (outside.length) groups.push({ plate: null, outside: true, rows: outside });
      return groups;
    },

    render() {
      if (!this.node) return;
      const { el } = this.host;
      const project = this.host.project();
      const data = GL.edit.data();
      const has = Boolean(data && data.objects.length);
      this.none.hidden = has;
      this.node.hidden = !has;
      if (!has) {
        this.node.replaceChildren();
        return;
      }
      const filaments = Math.max(project.filament_count || 1, (project.colours || []).length, (project.choices.filaments || []).length, 1);
      const swatch = (filament) => {
        const node = el('span', { className: 'swatch' });
        node.style.background = css(GL.Scene.filamentColour(data.colours, filament));
        return node;
      };
      const parts = [el('div', { className: 'obj-head' }, el('span', { textContent: 'Name' }), el('span'), el('span', { textContent: 'Fila.', title: 'Filament' }))];
      for (const group of this.groups(data)) {
        if (group.plate || group.outside) {
          const current = group.plate && group.plate.id === project.plate;
          const name = group.outside ? 'Outside' : `Plate ${group.plate.id}${group.plate.name ? ` (${group.plate.name})` : ''}`;
          const title = el('button', { type: 'button', className: 'obj-plate-name', textContent: name, disabled: Boolean(group.outside) });
          const head = el('div', { className: `obj-plate${current ? ' current' : ''}` }, title);
          if (group.plate) {
            title.addEventListener('click', () => this.host.plate(group.plate.id));
            title.addEventListener('contextmenu', (event) => { event.preventDefault(); GL.edit.plateMenu(event.clientX, event.clientY, group.plate.id); });
            if (GL.plateTools) title.addEventListener('keydown', (event) => GL.plateTools.keyMenu(event, title, group.plate.id));
            // A locked plate says so (`PartPlate::is_locked`).
            if (group.plate.locked) {
              title.title = `${name}: locked`;
              const lock = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
              lock.setAttribute('viewBox', '0 0 16 16');
              lock.setAttribute('class', 'lock-mark');
              lock.setAttribute('aria-hidden', 'true');
              const line = document.createElementNS('http://www.w3.org/2000/svg', 'path');
              line.setAttribute('d', 'M3.5,7.5h9v7h-9ZM5.5,7.5v-2.5a2.5,2.5,0,0,1,5,0v2.5');
              lock.append(line);
              head.append(lock);
            }
            if (data.plates.length > 1) {
              const remove = el('button', { type: 'button', className: 'icon tool', title: 'Remove current plate (if not last one)', textContent: '✕' });
              remove.setAttribute('aria-label', `Remove plate ${group.plate.id}`);
              remove.addEventListener('click', () => GL.edit.commit({ op: 'delete_plate', plate: group.plate.id }));
              head.append(remove);
            }
          }
          parts.push(head);
          if (!group.rows.length) parts.push(el('p', { className: 'obj-empty quiet', textContent: 'No objects on this plate.' }));
        }
        for (const row of group.rows) parts.push(this.row(row, { project, filaments, swatch }));
      }
      this.node.replaceChildren(...parts);
      this.mark();
    },

    row(row, { project, filaments, swatch }) {
      const { el, listbox } = this.host;
      const ids = row.copies.map((copy) => copy.id);
      if (row.part) return this.partRow(row, ids, swatch);
      const label = row.child ? `Instance ${row.number}` : (row.object.name || project.file || 'Object');
      const printable = row.copies.every((copy) => copy.printable);
      const eye = el('button', { type: 'button', className: 'icon tool obj-print', title: 'Printable', textContent: printable ? '●' : '○' });
      eye.setAttribute('aria-label', `Printable: ${label}`);
      eye.setAttribute('aria-pressed', String(printable));
      eye.addEventListener('click', () => GL.edit.commit({ op: 'printable', copies: ids, value: !printable }));
      const name = el('button', { type: 'button', className: 'obj-name', textContent: label, title: label });
      name.addEventListener('click', (event) => this.pick(node, event));
      name.addEventListener('contextmenu', (event) => {
        event.preventDefault();
        if (!ids.every((id) => GL.edit.stage.selection().has(id)) || GL.edit.stage.parts().size) GL.edit.stage.select(ids);
        GL.edit.menu(event.clientX, event.clientY);
      });
      const node = el('div', { className: `obj-row${row.child ? ' child' : ''}${printable ? '' : ' off'}` }, name, eye);
      node.setAttribute('role', 'treeitem');
      node.dataset.copies = ids.join(',');
      node.dataset.object = String(row.object.id);
      if (!row.child) {
        name.addEventListener('dblclick', () => this.rename(row.object.id));
        name.addEventListener('keydown', (event) => { if (event.key === 'F2') { event.preventDefault(); this.rename(row.object.id); } });
        // The filament the object prints in; its parts may differ, which
        // the list shows as none chosen.
        const select = el('select');
        select.setAttribute('aria-label', `Filament of ${label}`);
        if (!row.object.filament) select.append(new Option('…', ''));
        for (let filament = 1; filament <= Math.max(filaments, row.object.filament || 0); filament += 1) select.append(new Option(String(filament), String(filament)));
        select.value = row.object.filament ? String(row.object.filament) : '';
        select.addEventListener('change', () => {
          if (select.value) GL.edit.commit({ op: 'filament', object: row.object.id, filament: Number(select.value) });
        });
        const list = listbox(select, { bare: true, decorate: (item, option) => { if (option.value) item.prepend(swatch(Number(option.value))); } });
        const cell = el('span', { className: 'obj-fila' }, list);
        if (row.object.filament) cell.prepend(swatch(row.object.filament));
        node.append(cell);
      } else {
        node.append(el('span'));
      }
      return node;
    },

    // A part of an object (the reference's volume row): the mark of its
    // kind and its name (renamed by a double click or F2), and for a part
    // or a modifier its filament, "default" being its object's. A click
    // selects it as it is in the copies the row is under; its menu is the
    // part menu.
    partRow(row, ids, swatch) {
      const { el, listbox } = this.host;
      const { part } = row;
      const kind = PART_KINDS[part.kind] || 'Part';
      const label = part.name || `${kind} ${row.number}`;
      const name = el('button', { type: 'button', className: 'obj-name', title: `${label}\n${kind}` }, icon(KIND_ICONS[part.kind] || KIND_ICONS.part), el('span', { className: 'obj-label', textContent: label }));
      const node = el('div', { className: `obj-row child part${part.kind === 'part' ? '' : ' helper'}` }, name, el('span'));
      name.addEventListener('click', (event) => this.pick(node, event));
      name.addEventListener('dblclick', () => this.renamePart(row.object.id, part.id));
      name.addEventListener('keydown', (event) => { if (event.key === 'F2') { event.preventDefault(); this.renamePart(row.object.id, part.id); } });
      name.addEventListener('contextmenu', (event) => {
        event.preventDefault();
        const stage = GL.edit.stage;
        if (!stage.parts().has(part.id)) stage.selectParts(ids[0], [part.id]);
        GL.edit.partMenu(event.clientX, event.clientY);
      });
      node.setAttribute('role', 'treeitem');
      node.dataset.copies = ids.join(',');
      node.dataset.object = String(row.object.id);
      node.dataset.part = String(part.id);
      const cell = el('span', { className: 'obj-fila' });
      if (part.kind === 'part' || part.kind === 'modifier') {
        // GUI_ObjectList.cpp: "default" and each filament
        // (ExtraRenderers.cpp `BitmapChoiceRenderer::CreateEditorCtrl`).
        const filaments = GL.edit.filamentCount();
        const follows = row.object.own_filament || 1;
        const select = el('select');
        select.setAttribute('aria-label', `Filament of ${label}`);
        // "default" reads in the list; closed, the cell shows the number
        // it then prints in, as the reference's cell shows the object's.
        const fallback = new Option('default', '');
        fallback.label = String(follows);
        select.append(fallback);
        for (let filament = 1; filament <= Math.max(filaments, part.own || 0); filament += 1) select.append(new Option(String(filament), String(filament)));
        select.value = part.own ? String(part.own) : '';
        select.title = part.own ? `Filament ${part.own}` : `default: the object's, filament ${follows}`;
        select.addEventListener('change', () => {
          GL.edit.commit({ op: 'part_filament', object: row.object.id, parts: [part.id], filament: Number(select.value) || 0 });
        });
        const list = listbox(select, { bare: true, decorate: (item, option) => {
          if (!option.value) item.querySelector('.lb-text').textContent = 'default';
          item.prepend(swatch(option.value ? Number(option.value) : follows));
        } });
        cell.append(swatch(part.own || (part.kind === 'part' ? part.filament : follows)), list);
      }
      node.append(cell);
      return node;
    },

    // The name of a part becomes a field to type another into.
    renamePart(object, part) {
      const row = this.rows().find((held) => held.dataset.part === String(part));
      const name = row && row.querySelector('.obj-name');
      if (!name) return;
      const { el } = this.host;
      const text = name.querySelector('.obj-label').textContent;
      const input = el('input', { type: 'text', className: 'obj-rename', value: text, spellcheck: false });
      input.setAttribute('aria-label', 'Name');
      let done = false;
      const end = (take) => {
        if (done) return;
        done = true;
        const next = input.value.trim();
        if (take && next && next !== text) GL.edit.commit({ op: 'part_rename', object, part, name: next }).then((made) => { if (!made) this.render(); });
        else input.replaceWith(name);
      };
      input.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') { event.preventDefault(); end(true); }
        if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); end(false); }
      });
      input.addEventListener('blur', () => end(true));
      name.replaceWith(input);
      input.select();
    },

    rows() {
      return [...this.node.querySelectorAll('.obj-row')];
    },

    // A click on a row selects what it stands for; Ctrl adds it or takes
    // it away, Shift selects from the row last clicked to this one.
    pick(node, event) {
      const stage = GL.edit.stage;
      const ids = node.dataset.copies.split(',').map(Number);
      const adding = event.ctrlKey || event.metaKey || event.shiftKey;
      const parts = stage.parts();
      if (node.dataset.part) {
        // A part: alone, or with Ctrl or Shift added to the parts of the
        // same object selected, or taken away.
        const part = Number(node.dataset.part);
        const object = node.dataset.object;
        const sameObject = parts.size && this.rows().some((row) => row.dataset.object === object && row.dataset.part && parts.has(Number(row.dataset.part)));
        if (adding && parts.size && !sameObject) { this.host.say(CONFLICT_PART, false); return; }
        if (adding && !parts.size && stage.selection().size) { this.host.say(CONFLICT_OBJECT, false); return; }
        this.anchor = node.dataset.copies;
        if (adding && parts.size) {
          const chosen = new Set(parts);
          if (chosen.has(part)) chosen.delete(part); else chosen.add(part);
          if (chosen.size) stage.selectParts([...stage.selection()][0], chosen); else stage.select([]);
          return;
        }
        stage.selectParts(ids[0], [part]);
        return;
      }
      if (adding && parts.size) { this.host.say(CONFLICT_PART, false); return; }
      let chosen;
      if (event.shiftKey && this.anchor) {
        const rows = this.rows().filter((row) => !row.dataset.part);
        const [from, to] = [rows.findIndex((row) => row.dataset.copies === this.anchor), rows.indexOf(node)];
        const [low, high] = from < 0 ? [to, to] : [Math.min(from, to), Math.max(from, to)];
        chosen = new Set(rows.slice(low, high + 1).flatMap((row) => row.dataset.copies.split(',').map(Number)));
      } else if (event.ctrlKey || event.metaKey) {
        chosen = new Set(stage.selection());
        const all = ids.every((id) => chosen.has(id));
        for (const id of ids) { if (all) chosen.delete(id); else chosen.add(id); }
        this.anchor = node.dataset.copies;
      } else {
        chosen = new Set(ids);
        this.anchor = node.dataset.copies;
      }
      stage.select(chosen);
    },

    // Shows which rows are selected, without drawing them anew.
    mark() {
      if (!this.node || !GL.edit) return;
      const selection = GL.edit.stage.selection();
      const parts = GL.edit.stage.parts();
      for (const row of this.rows()) {
        const ids = row.dataset.copies.split(',').map(Number);
        // While parts are selected, their rows and no others.
        const on = parts.size ? Boolean(row.dataset.part) && parts.has(Number(row.dataset.part)) && ids.some((id) => selection.has(id)) : !row.dataset.part && ids.every((id) => selection.has(id));
        row.setAttribute('aria-selected', String(on));
      }
    },

    // The name of an object becomes a field to type another into: Enter
    // takes it, Escape leaves it as it was.
    rename(object) {
      const row = this.rows().find((held) => held.dataset.object === String(object) && !held.classList.contains('child'));
      const name = row && row.querySelector('.obj-name');
      if (!name) return;
      const { el } = this.host;
      const input = el('input', { type: 'text', className: 'obj-rename', value: name.textContent, spellcheck: false });
      input.setAttribute('aria-label', 'Name');
      let done = false;
      const end = (take) => {
        if (done) return;
        done = true;
        const next = input.value.trim();
        if (take && next && next !== name.textContent) GL.edit.commit({ op: 'rename', object, name: next }).then((made) => { if (!made) this.render(); });
        else input.replaceWith(name);
      };
      input.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') { event.preventDefault(); end(true); }
        if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); end(false); }
      });
      input.addEventListener('blur', () => end(true));
      name.replaceWith(input);
      input.select();
    },
  };

  GL.objectList = objects;
})(window.EdgeGL);
