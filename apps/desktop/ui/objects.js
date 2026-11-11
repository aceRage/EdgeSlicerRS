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
        if (!ids.every((id) => GL.edit.stage.selection().has(id))) GL.edit.stage.select(ids);
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

    // A part of an object, read only: its name, what it is, its
    // filament. A click selects the object; a part of its own cannot be
    // selected, moved or set yet.
    partRow(row, ids, swatch) {
      const { el } = this.host;
      const { part } = row;
      const kind = PART_KINDS[part.kind] || 'Part';
      const label = part.name || `${kind} ${row.number}`;
      const name = el('button', { type: 'button', className: 'obj-name', textContent: label, title: `${label}\n${kind}` });
      const node = el('div', { className: `obj-row child part${part.kind === 'part' ? '' : ' helper'}` }, name, el('span'));
      name.addEventListener('click', (event) => this.pick(node, event));
      node.setAttribute('role', 'treeitem');
      node.dataset.copies = ids.join(',');
      node.dataset.object = String(row.object.id);
      const cell = el('span', { className: 'obj-fila' });
      if (part.kind === 'part') cell.append(swatch(part.filament), el('span', { className: 'obj-fila-number', textContent: String(part.filament) }));
      node.append(cell);
      return node;
    },

    rows() {
      return [...this.node.querySelectorAll('.obj-row')];
    },

    // A click on a row selects what it stands for; Ctrl adds it or takes
    // it away, Shift selects from the row last clicked to this one.
    pick(node, event) {
      const stage = GL.edit.stage;
      const ids = node.dataset.copies.split(',').map(Number);
      let chosen;
      if (event.shiftKey && this.anchor) {
        const rows = this.rows();
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
      for (const row of this.rows()) {
        const ids = row.dataset.copies.split(',').map(Number);
        row.setAttribute('aria-selected', String(ids.every((id) => selection.has(id))));
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
