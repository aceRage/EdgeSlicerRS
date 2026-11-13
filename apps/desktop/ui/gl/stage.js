'use strict';

// The stage in 3D: Prepare's scene and Preview's print on one canvas with
// one camera, as the reference's two views share theirs (Plater.cpp
// `set_current_panel`). app.js says what is to be shown (`update`); this
// asks the session for what it takes, keeps it on the graphics card, and
// answers the mouse and the keys as the reference's canvas does
// (GLCanvas3D.cpp `on_mouse`, `on_mouse_wheel`, `on_char`, `on_key`).
(function (GL) {
  const M = GL.math;
  const { el } = GL.controls;
  const T = GL.Toolpath;

  // GLCanvas3D.cpp: TRACKBALLSIZE, the margins a view is fitted with.
  const ORBIT = (Math.PI * 0.8) / 180;
  const MARGIN = { box: 1.25, bed: 2.0, plate: 1.25 };
  // A press that moves less than this is a click.
  const CLICK = 4;
  // How many prints' moves are kept on the graphics card at once.
  const PRINTS_KEPT = 3;

  // The settings of Preferences the camera takes (Preferences.cpp,
  // GLCanvas3D.cpp `on_mouse`), kept by the session (prefs.js).
  const PREFERENCES = { zoomToMouse: 'zoom_to_mouse', reverseZoom: 'reverse_mouse_wheel_zoom' };
  const pref = (key) => typeof Prefs !== 'undefined' && Prefs.flag(key);
  const orbit = () => {
    const mult = typeof Prefs !== 'undefined' ? Number(Prefs.get('camera_orbit_mult')) : 1;
    return ORBIT * (Number.isFinite(mult) && mult > 0 ? mult : 1);
  };
  // "Camera style": Touchpad turns the view with Alt and moves it with
  // Shift, the pointer moved without a button (`is_camera_rotate`,
  // `is_camera_pan`).
  const touchpad = () => typeof Prefs !== 'undefined' && Prefs.get('camera_navigation_style') === '1';

  const hex = (colour) => [1, 3, 5].map((at) => parseInt(colour.slice(at, at + 2), 16));
  const css = (colour) => `rgb(${colour.join(',')})`;

  const stage = {
    active: false,
    // What was asked of the session for the 3D views, and how long things took.
    stats: { calls: 0, bytes: 0, json: 0, events: [] },

    // Sets the 3D stage up on `host.canvas`. False where the browser has
    // no WebGL2: the page then keeps its view from above.
    init(host) {
      this.host = host;
      try {
        this.renderer = new GL.Renderer(host.canvas);
      } catch (error) {
        host.note.textContent = error.missing
          ? 'The 3D view needs WebGL2, which this browser does not offer. The plate is shown from above instead.'
          : `The 3D view could not be started (${error.message}). The plate is shown from above instead.`;
        host.note.hidden = false;
        host.canvas.hidden = true;
        return false;
      }
      this.canvas = host.canvas;
      this.scene = new GL.Scene(this.renderer);
      this.camera = new GL.Camera();
      this.marker = this.renderer.meshFromPoints(T.arrow());
      this.model = null;
      this.shown = null;          // the project the camera is on
      this.sceneKey = '';
      this.sceneTurn = 0;
      this.bedModelOf = '';
      this.bedPictureOf = '';
      this.prints = new Map();    // "project:plate:run" -> { path, ... }
      this.print = null;
      this.prefs = {
        perspective: host.remembered.get('gl.perspective') !== '0',
        autoPerspective: host.remembered.get('gl.auto_perspective') === '1',
        navigator: host.remembered.get('gl.navigator') !== '0',
        labels: host.remembered.get('gl.labels') === '1',
        // The G-code window, shown unless it was hidden (the reference
        // starts with it on, `show_gcode_window`).
        gcodeWindow: host.remembered.get('gl.gcode_window') !== '0',
      };
      this.camera.perspective = this.prefs.perspective;
      this.build(host.stage);
      this.listen();
      host.stage.classList.add('gl');
      this.active = true;
      return true;
    },

    // ---- what is laid over the view ----

    build(parent) {
      this.navigator = new GL.controls.Navigator(parent, (view) => this.view(view));
      const fit = el('button', { type: 'button', className: 'icon tool gl-fit', title: `${t('Fit in all view')} [Z]` });
      fit.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5,6V2.5H6M10,2.5h3.5V6M13.5,10v3.5H10M6,13.5H2.5V10"/><rect x="5.5" y="5.5" width="5" height="5" rx="0.5"/></svg>';
      fit.setAttribute('aria-label', t('Fit in all view'));
      fit.addEventListener('click', () => this.fit('part'));
      const menu = el('button', { type: 'button', className: 'quiet gl-view', textContent: t('View') });
      menu.insertAdjacentHTML('beforeend', '<svg class="chevron" viewBox="0 0 10 6" aria-hidden="true"><path d="M1 5l4-4 4 4"/></svg>');
      menu.setAttribute('aria-haspopup', 'menu');
      menu.addEventListener('click', () => {
        const box = menu.getBoundingClientRect();
        this.host.menu(box.left, box.top - 8, this.viewMenu(), true);
      });
      this.tools = el('div', { className: 'gl-tools toolbar' }, fit, el('span', { className: 'toolbar-gap' }), menu);
      parent.append(this.tools);
      this.labels = el('div', { className: 'gl-labels' });
      parent.append(this.labels);
      this.readout = el('div', { className: 'gl-readout overlay', hidden: true });
      parent.append(this.readout);
      this.loading = el('div', { className: 'gl-loading overlay', hidden: true });
      parent.append(this.loading);
      this.layerSlider = new GL.controls.LayerSlider(parent, {
        onChange: (low, high, which) => this.layersChanged(low, high, which),
        onMenu: (x, y) => this.host.layerMenu(x, y),
      });
      this.moveSlider = new GL.controls.MoveSlider(parent, { onChange: (value) => this.moveChanged(value) });
      this.gcodeWindow = new GL.GcodeWindow(parent, {
        call: (name, args) => this.call(name, args),
        bytes: (name, args) => this.bytes(name, args),
        step: (by) => this.moveSlider.step(by),
        say: (text, bad) => this.host.say(text, bad),
      });
      this.gcodeButton = document.getElementById('legend-gcode');
      if (this.gcodeButton) this.gcodeButton.addEventListener('click', () => this.toggleGcode());
      this.navigator.node.hidden = !this.prefs.navigator;
    },

    setPref(name, key, value) {
      // Those of Preferences are the session's (prefs.js).
      if (PREFERENCES[name]) {
        attempt(() => Prefs.set(PREFERENCES[name], value ? 'true' : 'false'));
        return;
      }
      this.prefs[name] = value;
      this.host.remembered.set(`gl.${key}`, value ? '1' : '0');
      if (name === 'perspective') this.camera.perspective = value;
      this.navigator.node.hidden = !this.prefs.navigator;
      this.frame();
    },

    // The reference's View menu, and of its Preferences what the camera
    // takes (MainFrame.cpp, Preferences.cpp).
    viewMenu() {
      const p = this.prefs;
      return [
        [t('Default View'), () => this.view('default')],
        [t('Top'), () => this.view('top')],
        [t('Bottom'), () => this.view('bottom')],
        [t('Front'), () => this.view('front')],
        [t('Rear'), () => this.view('rear')],
        [t('Left'), () => this.view('left')],
        [t('Right'), () => this.view('right')],
        null,
        [t('Use Perspective View'), () => this.setPref('perspective', 'perspective', true), { checked: p.perspective }],
        [t('Use Orthogonal View'), () => this.setPref('perspective', 'perspective', false), { checked: !p.perspective }],
        [t('Auto Perspective'), () => this.setPref('autoPerspective', 'auto_perspective', !p.autoPerspective), { checked: p.autoPerspective }],
        null,
        [t('Zoom to mouse position'), () => this.setPref('zoomToMouse', 'zoom_to_mouse', !pref('zoom_to_mouse')), { checked: pref('zoom_to_mouse') }],
        [t('Reverse mouse zoom'), () => this.setPref('reverseZoom', 'reverse_zoom', !pref('reverse_mouse_wheel_zoom')), { checked: pref('reverse_mouse_wheel_zoom') }],
        null,
        [t('Show 3D Navigator'), () => this.setPref('navigator', 'navigator', !p.navigator), { checked: p.navigator }],
        [t('Show Labels'), () => this.setPref('labels', 'labels', !p.labels), { checked: p.labels }],
      ];
    },

    // ---- the session ----

    async call(name, args) {
      const answer = await this.host.call(name, args);
      this.stats.calls += 1;
      this.stats.json += JSON.stringify(answer).length;
      return answer;
    },

    async bytes(name, args) {
      const answer = await this.host.bytes(name, args);
      this.stats.calls += 1;
      this.stats.bytes += answer.byteLength;
      return answer;
    },

    note(what, since, more = {}) {
      this.stats.events.push({ what, ms: Math.round(performance.now() - since), ...more });
    },

    // ---- what is shown ----

    // Shows what app.js says is on the stage: `model` is the project, the
    // tab, what the project would be sliced with, the summary of its
    // slice in Preview, and the page's own notes on it (`kept`).
    update(model) {
      if (!this.active) return;
      const before = this.model;
      this.model = model;
      const project = model.project;
      if (!project || model.tab === 'device') return;
      const mine = this.mine();
      if (GL.edit) GL.edit.shown(model);
      // Each project keeps its own camera; the two tabs share it.
      if (this.shown !== project.id) {
        if (before && before.project && before.kept.gl && this.shown === before.project.id) before.kept.gl.camera = this.camera.save();
        this.shown = project.id;
        if (mine.camera) this.camera.load(mine.camera);
      }
      this.wantScene();
      const preview = model.tab === 'preview';
      const printed = this.print;
      if (preview && model.summary) this.wantPrint();
      else this.print = null;
      // A print that was held already has its legend to show at once.
      if (this.print && this.print !== printed && this.print.path) this.host.legend();
      if (preview) this.followPlate();
      this.syncControls();
      this.frame();
    },

    // In Preview the plate shown is the only one there is to see, and it
    // stands where it stands among the others: the camera goes with it,
    // as far as it is from the plate the camera was last turned to. (The
    // reference leaves the camera where it was, which can be on a plate
    // that is not drawn.)
    followPlate() {
      const mine = this.mine();
      const plate = this.plateOrigin();
      if (!plate) return;
      if (mine.plateAt && (mine.plateAt[0] !== plate[0] || mine.plateAt[1] !== plate[1])) {
        this.camera.target = M.add(this.camera.target, [plate[0] - mine.plateAt[0], plate[1] - mine.plateAt[1], 0]);
      }
      mine.plateAt = plate.slice();
    },

    // The page's own notes on the project shown, for the 3D views.
    mine() {
      const kept = this.model.kept;
      if (!kept.gl) kept.gl = { camera: null, fitted: '', selection: new Set(), parts: new Set(), low: {}, moveAt: {}, plateAt: null };
      return kept.gl;
    },

    plateId() {
      const project = this.model.project;
      return project.plate === undefined ? null : project.plate;
    },

    plateOrigin() {
      const plate = this.scene.data && this.scene.data.project === this.model.project.id ? this.scene.plate(this.plateId()) : null;
      return plate ? plate.origin : null;
    },

    // What tells one scene from another: the project, its file, its
    // printer, and how often its model was edited.
    keyOf(project, settled) {
      const printer = settled && settled.printer ? `${settled.printer.name}|${JSON.stringify(settled.printer.bed)}` : '';
      const revision = project.edit ? project.edit.revision : 0;
      return `${project.id}|${project.path || ''}|${project.empty}|${revision}|${printer}`;
    },

    // Asks for the project's scene when it is not the one held: another
    // project, another file, another printer, an edit made elsewhere.
    wantScene() {
      const { project, settled } = this.model;
      const key = this.keyOf(project, settled);
      if (key === this.sceneKey) {
        if (this.scene.data) this.scene.data.plate = this.plateId();
        return;
      }
      this.sceneKey = key;
      const turn = ++this.sceneTurn;
      this.call('scene', { project: project.id }).then((data) => {
        if (turn === this.sceneTurn) this.takeScene(data, turn);
      }).catch((error) => { if (turn === this.sceneTurn) this.host.say(error.message, true); });
    },

    // Shows a scene as the session gave it, and asks for the meshes and
    // the bed it has not got.
    async takeScene(data, turn) {
      const since = performance.now();
      const sent = this.stats.bytes;
      const mine = this.mine();
      this.scene.set(data);
      this.scene.keep(this.openKeys(data));
      this.scene.prune();
      // A bed that is new to the project, or of another size, is
      // looked at afresh, as the reference's window starts.
      const bed = JSON.stringify(data.bed.box);
      if (mine.fitted !== bed) {
        mine.fitted = bed;
        this.view('default');
      }
      // What is selected is still selected while it is still there.
      const there = new Set(data.placings.map((placing) => placing.id));
      mine.selection = new Set([...mine.selection].filter((id) => there.has(id)));
      // Parts selected stay selected while their copy and they are there.
      const copy = mine.selection.size === 1 ? data.placings.find((placing) => mine.selection.has(placing.id)) : null;
      const kept = copy ? new Set((data.objects[copy.object].part_list || []).map((part) => part.id)) : new Set();
      mine.parts = new Set([...(mine.parts || [])].filter((id) => kept.has(id)));
      if (GL.edit) GL.edit.sceneChanged();
      this.frame();
      await Promise.all([this.wantBed(data, turn), this.wantMeshes(data, turn, since, sent)]);
    },

    // The scene an edit answered with (scene-edit.js): shown at once,
    // with nothing asked for but the meshes that are new.
    adopt(project, data) {
      if (!this.model || !this.model.project || this.model.project.id !== project.id) return;
      this.model.project = project;
      this.sceneKey = this.keyOf(project, this.model.settled);
      const turn = ++this.sceneTurn;
      this.takeScene(data, turn).catch((error) => { if (turn === this.sceneTurn) this.host.say(error.message, true); });
    },

    // The keys of this scene and of those of the other projects still
    // open: their meshes are kept, so that coming back to one sends nothing.
    openKeys(data) {
      const open = new Set(this.host.open());
      const keys = new Set([data.key]);
      for (const key of this.scene.meshes.keys()) {
        const scene = key.slice(0, key.lastIndexOf(':'));
        const project = Number(scene.split(':')[0]);
        if (open.has(project) && project !== data.project) keys.add(scene);
      }
      return [...keys];
    },

    // The meshes the scene has not got yet, one object after another,
    // those of the plate shown first.
    async wantMeshes(data, turn, since, sent) {
      // The object being painted is drawn by its tool, and its mesh as the
      // paint now colours it is asked for when the tool closes.
      const painting = this.scene.painting;
      const order = [...data.objects.keys()].filter((object) => !this.scene.meshes.has(`${data.key}:${data.objects[object].mesh}`) && !(painting && painting.object === object));
      const onPlate = new Set(data.placings.filter((placing) => placing.plate === data.plate).map((placing) => placing.object));
      order.sort((a, b) => Number(onPlate.has(b)) - Number(onPlate.has(a)));
      let first = true;
      for (const object of order) {
        this.say(`Loading ${data.objects[object].name || 'the model'}…`);
        // Two objects of one shape (a clone) have one mesh.
        const key = `${data.key}:${data.objects[object].mesh}`;
        if (this.scene.meshes.has(key)) continue;
        const bytes = await this.bytes('scene_mesh', { project: data.project, mesh: data.objects[object].mesh });
        if (turn !== this.sceneTurn) return;
        this.scene.addMesh(key, this.renderer.meshFromBytes(bytes));
        this.draw();
        if (first) this.note('scene: first object drawn', since, { bytes: this.stats.bytes - sent });
        first = false;
        // Let the page breathe between two objects.
        await new Promise((resolve) => setTimeout(resolve, 0));
        if (turn !== this.sceneTurn) return;
      }
      this.say('');
      if (order.length) {
        const triangles = data.objects.reduce((sum, object) => sum + object.triangles, 0);
        this.note('scene: all objects drawn', since, { bytes: this.stats.bytes - sent, triangles, objects: order.length });
      }
    },

    // The meshes of the scene shown that are not here yet: those not asked
    // for while a painting tool was open.
    wantMissing() {
      const data = this.scene.data;
      if (!data) return;
      const turn = this.sceneTurn;
      this.wantMeshes(data, turn, performance.now(), this.stats.bytes).catch((error) => { if (turn === this.sceneTurn) this.host.say(error.message, true); });
    },

    // The vendor's model and picture of the bed, when the printer's
    // profile has them and they are not the ones held.
    async wantBed(data, turn) {
      const { model, texture } = data.bed;
      const printer = this.sceneKey.split('|').slice(3).join('|');
      if (`${printer}|${model}` !== this.bedModelOf) {
        this.bedModelOf = `${printer}|${model}`;
        this.scene.setBedModel(null);
        if (model) {
          try {
            const bytes = await this.bytes('bed_model', { project: data.project });
            if (turn === this.sceneTurn) this.scene.setBedModel(this.renderer.meshFromBytes(bytes));
          } catch { this.bedModelOf = ''; }
          this.frame();
        }
      }
      if (`${printer}|${texture}` !== this.bedPictureOf) {
        this.bedPictureOf = `${printer}|${texture}`;
        this.scene.setBedPicture(null);
        if (texture) {
          try {
            const bytes = await this.bytes('bed_texture', { project: data.project });
            const image = await this.imageOf(bytes, /\.svg$/i.test(texture) ? 'image/svg+xml' : 'image/png');
            if (turn === this.sceneTurn) this.scene.setBedPicture(this.renderer.picture(image));
          } catch { this.bedPictureOf = ''; }
          this.frame();
        }
      }
    },

    // A picture's bytes as a canvas of at most 2048 pixels a side, which
    // is as fine as the reference draws a bed's picture.
    imageOf(bytes, kind) {
      const data = new Uint8Array(bytes);
      let text = '';
      for (let at = 0; at < data.length; at += 0x8000) text += String.fromCharCode(...data.subarray(at, at + 0x8000));
      return new Promise((resolve, reject) => {
        const image = new Image();
        image.onload = () => {
          const [wide, high] = [image.naturalWidth || 1024, image.naturalHeight || 1024];
          const scale = 2048 / Math.max(wide, high);
          const canvas = document.createElement('canvas');
          canvas.width = Math.max(1, Math.round(wide * scale));
          canvas.height = Math.max(1, Math.round(high * scale));
          canvas.getContext('2d').drawImage(image, 0, 0, canvas.width, canvas.height);
          resolve(canvas);
        };
        image.onerror = () => reject(new Error('the picture of the bed cannot be read'));
        image.src = `data:${kind};base64,${btoa(text)}`;
      });
    },

    // The moves of the plate's slice: those held when it was looked at
    // before, else asked for, a stretch at a time.
    wantPrint() {
      const project = this.model.project;
      const plate = this.plateId();
      const key = `${project.id}:${plate}:${project.status.run}`;
      let print = this.prints.get(key);
      if (!print) {
        print = { key, path: null };
        // An earlier slice of the plate is let go, and the oldest of more
        // than a few.
        const plateOf = (known) => known.slice(0, known.lastIndexOf(':'));
        for (const [old, other] of [...this.prints]) {
          if (plateOf(old) !== plateOf(key) && this.prints.size < PRINTS_KEPT) continue;
          if (other.path) other.path.dispose();
          this.prints.delete(old);
        }
        this.prints.set(key, print);
        this.loadPrint(print, project.id, plate);
      }
      this.print = print;
    },

    async loadPrint(print, project, plate) {
      const since = performance.now();
      const sent = this.stats.bytes;
      const args = { project, plate: plate === null ? undefined : plate };
      try {
        const index = await this.call('toolpath', args);
        if (!this.prints.has(print.key)) return;
        const path = new T(this.renderer, index);
        print.path = path;
        this.host.legend();
        this.syncControls();
        let first = true;
        while (!path.complete) {
          const bytes = await this.bytes('toolpath_moves', { ...args, from: path.loaded, count: index.at_once });
          if (!this.prints.has(print.key)) return;
          if (!path.append(bytes)) throw new Error('the moves stopped short');
          if (this.print === print) {
            this.say(path.complete ? '' : `Loading the print… ${Math.round((100 * path.loaded) / path.total)}%`);
            this.syncControls();
            this.draw();
          }
          if (first) this.note('print: first moves drawn', since, { bytes: this.stats.bytes - sent, moves: path.loaded });
          first = false;
          await new Promise((resolve) => setTimeout(resolve, 0));
        }
        this.note('print: all moves drawn', since, { bytes: this.stats.bytes - sent, moves: path.total });
        if (this.print === print) {
          this.host.legend();
          this.frame();
        }
      } catch (error) {
        // A slice that was let go while its moves were on their way is
        // nothing to say; it is asked for again when it is there again.
        if (print.path) print.path.dispose();
        this.prints.delete(print.key);
        if (this.print !== print) return;
        this.print = null;
        this.say('');
        this.host.say(error.message, true);
      }
    },

    say(text) {
      this.loading.textContent = text;
      this.loading.hidden = !text;
    },

    // ---- Preview's sliders and legend ----

    path() {
      return this.print && this.print.path ? this.print.path : null;
    },

    // The layers shown, as [lowest, highest], and the move the print has
    // got to (null: all of them).
    range() {
      const path = this.path();
      if (!path) return null;
      const plate = String(this.plateId() ?? 0);
      const mine = this.mine();
      const last = path.layers.first.length - 1;
      const high = Math.max(0, Math.min(last, this.model.kept.layerAt[this.plateId() ?? 0] ?? last));
      const low = this.layerSlider.one ? high : Math.max(0, Math.min(high, mine.low[plate] ?? 0));
      const [from, end] = [path.layerStart(low), path.layerEnd(high)];
      const at = mine.moveAt[plate];
      return { low, high, last, from, end, to: at === null || at === undefined ? end : Math.max(from + 1, Math.min(end, at)) };
    },

    syncControls() {
      const preview = this.model && this.model.tab === 'preview';
      const path = preview ? this.path() : null;
      const range = this.range();
      if (this.gcodeButton) {
        this.gcodeButton.hidden = !path || !path.index.gcode_lines;
        this.gcodeButton.setAttribute('aria-pressed', String(this.prefs.gcodeWindow));
      }
      if (!path || !range) {
        this.layerSlider.set(0, 0, 0, () => ['', '']);
        this.moveSlider.set(0, 0, 0);
        this.readout.hidden = true;
        this.gcodeWindow.show(null, null, -1, false);
        return;
      }
      this.layerSlider.set(range.last + 1, range.low, range.high, (layer) => [String(layer + 1), path.layers.z[layer].toFixed(2)]);
      this.showTicks(path);
      // The moves slider runs over the moves of the layers shown that
      // have come in so far.
      this.moveSlider.set(range.from + 1, Math.min(range.end, Math.max(path.loaded, range.from + 1)), range.to);
      this.showReadout(path, range);
      this.showGcode(range);
    },

    // What is set at the plate's layers (`layer_codes`), as app.js last
    // had it for this print.
    setCodes(codes) {
      this.codes = codes;
      this.ticksShown = null;
      this.syncControls();
    },

    // The layer an item set at a height is at: the one whose top is
    // there, else the first above it (IMSlider.cpp `get_tick_from_value`).
    layerAtZ(path, z) {
      const tops = path.layers.z;
      let at = tops.findIndex((top) => Math.abs(top - z) < 0.002);
      if (at < 0) at = tops.findIndex((top) => top > z);
      return at < 0 ? tops.length - 1 : at;
    },

    showTicks(path) {
      const codes = this.codes;
      const plate = this.plateId();
      const mine = codes && codes.project === this.model.project.id && (codes.plate ?? null) === plate ? codes : null;
      const key = mine ? `${this.print.key}|${JSON.stringify(mine.items)}` : '';
      if (key === this.ticksShown) return;
      this.ticksShown = key;
      const ticks = (mine ? mine.items : []).map((item) => ({ ...this.host.describeCode(item, mine), layer: this.layerAtZ(path, item.print_z), item }));
      this.layerSlider.setTicks(ticks, (tick) => this.host.deleteCode(tick.item));
    },

    // The layer the thumb taken up is on, and its top.
    selectedLayer() {
      const path = this.path();
      if (!path) return null;
      const slider = this.layerSlider;
      const layer = slider.one || slider.selected === 'high' ? slider.high : slider.low;
      return { layer, z: path.layers.z[layer], count: path.layers.z.length, tick: slider.tickHere() };
    },

    // The G-code window, at the line of the move the print has got to
    // (the last of the layers shown when the slider is at its end), under
    // the legend and right-aligned with it.
    showGcode(range) {
      const legend = document.getElementById('legend');
      const stage = this.host.stage;
      const inset = 12;
      // Down to the moves slider, or to "About this print" where the
      // window would reach over it.
      const notes = document.getElementById('notes');
      const bottomFor = (right) => (notes && !notes.hidden && notes.offsetLeft + notes.offsetWidth > stage.clientWidth - right - 480
        ? Math.max(64, stage.clientHeight - notes.offsetTop + 6)
        : 64);
      let place = { top: inset, right: 104 };
      if (legend && !legend.hidden) {
        const right = stage.clientWidth - legend.offsetLeft - legend.offsetWidth;
        place = { top: legend.offsetTop + legend.offsetHeight + 6, right };
        // Where a long legend leaves too little under it for eight
        // lines, the window stands beside it instead, at its left.
        if (stage.clientHeight - bottomFor(right) - place.top < 8 * 17) place = { top: legend.offsetTop, right: right + legend.offsetWidth + 8 };
      }
      place.bottom = bottomFor(place.right);
      const plate = this.plateId();
      const args = { project: this.model.project.id, plate: plate === null ? undefined : plate };
      this.gcodeWindow.show(this.print, args, range.to - 1, this.prefs.gcodeWindow && range.to - 1 < this.path().loaded, place);
    },

    toggleGcode() {
      this.prefs.gcodeWindow = !this.prefs.gcodeWindow;
      this.host.remembered.set('gl.gcode_window', this.prefs.gcodeWindow ? '1' : '0');
      this.syncControls();
    },

    // Where the nozzle is and what it is doing there, while the print is
    // stopped part of the way (GCodeViewer.cpp, the position window).
    showReadout(path, range) {
      const stopped = range.to < range.end && range.to - 1 < path.loaded;
      this.readout.hidden = !stopped;
      if (!stopped) return;
      const move = path.move(range.to - 1);
      const kinds = [t('Noop'), t('Travel'), t('Extrude'), t('Wipe'), t('Retract'), t('Unretract'), t('Seam'), t('Tool Change')];
      const view = T.VIEWS.find((known) => known.name === this.model.kept.viewType) || T.VIEWS[0];
      const digits = (value) => (Math.abs(value) > 9999 ? 1 : Math.abs(value) > 999 ? 2 : 3);
      const what = move.kind === T.KIND.line ? t(T.FEATURES[move.feature][0]) : kinds[move.kind];
      // GCodeViewer.cpp l.440–467: the label, then the value.
      const detail = {
        0: what, 1: `${t('Tool: ')}${move.filament}`, 2: what, 3: `${t('Height: ')}${move.height.toFixed(2)}`, 4: `${t('Width: ')}${move.width.toFixed(2)}`,
        5: `${t('Flow: ')}${move.flow.toFixed(2)}`, 6: `${t('Fan: ')}${move.fan.toFixed(0)}`, 7: `${t('Temperature: ')}${move.temperature.toFixed(0)}`,
      }[view.id];
      this.readout.replaceChildren(
        ...['X', 'Y', 'Z'].flatMap((axis, at) => [el('b', { className: `gl-${axis.toLowerCase()}`, textContent: `${axis} ` }), `${move.at[at].toFixed(digits(move.at[at]))}  `]),
        el('br'),
        `${t('Speed: ')}${move.feedrate.toFixed(0)}  ${detail}`,
      );
    },

    layersChanged(low, high, which) {
      const mine = this.mine();
      const plate = this.plateId() ?? 0;
      mine.low[String(plate)] = low;
      // Moving a layer thumb puts the moves slider back at its end.
      mine.moveAt[String(plate)] = null;
      this.model.kept.layerAt[plate] = high;
      this.host.noted();
      this.syncControls();
      this.frame();
      void which;
    },

    moveChanged(value) {
      const range = this.range();
      if (!range) return;
      this.mine().moveAt[String(this.plateId() ?? 0)] = value >= range.end ? null : value;
      this.syncControls();
      this.frame();
    },

    // app.js's own ways of going to a layer (the arrow keys, "Jump to
    // Layer") move the thumb taken up.
    setLayer(index) {
      if (!this.path()) return;
      this.layerSlider.move(index);
    },

    stepLayer(by) {
      if (this.path()) this.layerSlider.step(by);
    },

    // What the legend offers and lists, for app.js to put in its table:
    // the view types there is the data for, and the rows of the one
    // chosen. A row that can be hidden or shown calls `changed`.
    legend(kept, summary, filamentColour) {
      const path = this.path();
      const index = path ? path.index : null;
      const views = T.VIEWS.filter((view) => !view.needs || (index && index.source === 'gcode'));
      const view = views.find((known) => known.name === kept.viewType) || views[0];
      if (!kept.options) kept.options = new Set(T.OPTIONS.filter((option) => option.shown).map((option) => option.name));
      const head = (...names) => el('tr', {}, ...names.map((name) => el('th', { textContent: name })));
      const rows = (changed) => {
        const toggle = (set, key, name, colour, on) => {
          const off = set.has(key) !== on;
          const eye = el('button', { type: 'button', className: 'eye', textContent: off ? '○' : '●', title: off ? t('Show') : t('Hide') });
          eye.setAttribute('aria-label', `${off ? t('Show') : t('Hide')} ${name}`);
          eye.setAttribute('aria-pressed', String(!off));
          const swatch = el('span', { className: 'swatch' });
          swatch.style.background = colour;
          const row = el('tr', { className: `row${off ? ' off' : ''}` }, el('td', {}, swatch, name), el('td', {}, eye));
          row.addEventListener('click', () => {
            if (set.has(key)) set.delete(key); else set.add(key);
            changed();
          });
          return row;
        };
        // The marks and the moves between lines, those the print has.
        const options = (only = null) => {
          const present = T.OPTIONS.filter((option) => index && index.counts[option.count] > 0 && (!only || only.includes(option.name)));
          if (!present.length) return [];
          return [head(t('Options'), t('Display')), ...present.map((option) => toggle(kept.options, option.name, t(option.name), css(option.colour), true))];
        };
        if (view.id === 0) {
          // Before the moves are in, the features the slice is known to have.
          const features = index ? index.features.map((feature) => T.FEATURES[feature]) : summary.roles.map((name) => T.FEATURES.find(([known]) => known === name) || [name, [255, 255, 255]]);
          return [head(t('Line Type'), t('Display')), ...features.map(([name, colour]) => toggle(kept.hidden, name, t(name), css(colour), false)), ...options()];
        }
        if (view.id === 1) {
          const filaments = index ? index.filaments : summary.filaments;
          return [head(t('Filament'), t('Display')), ...filaments.map((filament) => toggle(kept.hidden, `f${filament}`, fmt(t('Filament %d'), filament), filamentColour(summary, filament), false)), ...options()];
        }
        const range = path ? path.range(view.id, this.hiddenFeatures(kept), kept.options.has('Travel'), kept.options.has('Wipe')) : [0, 0, 0];
        const steps = T.steps(range).reverse();
        return [head(t(view.title)), ...steps.map((value) => {
          const swatch = el('span', { className: 'swatch' });
          swatch.style.background = css(T.rangeColour(value, range));
          return el('tr', {}, el('td', {}, swatch, value.toFixed(view.decimals)));
        }), ...(view.id === 2 ? options(['Travel']) : [])];
      };
      return { types: views.map((known) => known.name), rows };
    },

    hiddenFeatures(kept) {
      const hidden = new Set();
      T.FEATURES.forEach(([name], feature) => { if (kept.hidden.has(name)) hidden.add(feature); });
      return hidden;
    },

    // ---- the camera ----

    // The plate shown, as a box on the ground.
    plateBox() {
      return this.scene.plateBox(this.plateId());
    },

    // What the camera turns about (GLCanvas3D.cpp `on_mouse`): in
    // Preview the middle of the plate; in Prepare the middle of what is
    // selected, else of the objects on the plate, else of the plate.
    pivot() {
      const plate = this.plateBox();
      if (this.model.tab !== 'preview') {
        const mine = this.mine();
        const id = this.plateId();
        const box = this.scene.boxOf((placing) => mine.selection.has(placing.id))
          || this.scene.boxOf((placing) => placing.plate === id || this.scene.plates.length === 1);
        if (box) return M.centre(box);
      }
      return plate ? M.centre(plate) : this.camera.target;
    },

    // A named view (Camera::select_view), with the reference's
    // "Default View": from the front and above, the bed in sight.
    view(name) {
      const camera = this.camera;
      this.sized();
      const plate = this.plateBox();
      const origin = this.plateOrigin();
      if (name === 'default') {
        camera.selectView('plate', this.prefs.autoPerspective);
        if (plate) camera.zoomToBox(plate, MARGIN.bed);
        if (origin) this.mine().plateAt = origin.slice();
      } else if (name === 'this plate') {
        // PartPlateList::select_plate_view.
        if (plate) {
          const centre = M.centre(plate);
          camera.lookAt(M.add(centre, [0, 0, camera.distance]), centre, [0, 1, 0]);
        }
        if (origin) this.mine().plateAt = origin.slice();
        camera.selectView('topfront', this.prefs.autoPerspective);
      } else {
        camera.selectView(name, this.prefs.autoPerspective);
      }
      this.prefs.perspective = camera.perspective;
      this.frame();
    },

    // "Fit in all view" (GLCanvas3D::ZoomToFit): from the front and
    // above, what is selected in sight, else the bed. `bed` is the
    // default view.
    fit(what) {
      if (!this.active || !this.model || !this.model.project) return;
      const camera = this.camera;
      if (what === 'bed') {
        this.view('default');
        return;
      }
      this.sized();
      camera.selectView('plate', this.prefs.autoPerspective);
      const mine = this.mine();
      const selected = this.model.tab !== 'preview' ? this.scene.boxOf((placing) => mine.selection.has(placing.id)) : null;
      const plate = this.plateBox();
      if (selected) camera.zoomToBox(selected, MARGIN.box);
      else if (plate) camera.zoomToBox(plate, MARGIN.bed);
      const origin = this.plateOrigin();
      if (origin) this.mine().plateAt = origin.slice();
      this.frame();
    },

    // ---- the mouse and the keys ----

    listen() {
      const canvas = this.canvas;
      let press = null;
      const at = (event) => {
        const box = canvas.getBoundingClientRect();
        return [event.clientX - box.left, event.clientY - box.top];
      };
      canvas.addEventListener('contextmenu', (event) => event.preventDefault());
      canvas.addEventListener('pointerdown', (event) => {
        if (!this.model || press) return;
        canvas.focus({ preventScroll: true });
        canvas.setPointerCapture(event.pointerId);
        const [x, y] = at(event);
        press = { id: event.pointerId, button: event.button, x, y, from: [x, y], moved: false, pivot: null, grab: null, paint: false };
        // A painting tool paints with the button on the object
        // (GLGizmoPainterBase.cpp `on_mouse`); off it, the view turns or
        // moves, and no object is dragged while the tool is open.
        const painter = this.model.tab === 'prepare' && GL.painter && GL.painter.active() ? GL.painter : null;
        if (painter) press.paint = painter.press(x, y, event);
        // In Prepare the left button takes hold of a tool's handle, or of
        // the object under it, which a drag then moves (GLCanvas3D.cpp
        // `on_mouse`); anywhere else it turns the view.
        if (!painter && event.button === 0 && this.model.tab === 'prepare' && GL.edit) press.grab = GL.edit.grab(x, y, event);
        // "Swap pan and rotate mouse buttons": the right button turns.
        const turns = pref('swap_mouse_buttons') ? 2 : 0;
        press.turns = turns;
        if (event.button === turns && !press.grab && !press.paint) {
          // With Ctrl or Cmd, about the point of the bed in the middle
          // of the view.
          const centre = event.ctrlKey || event.metaKey ? this.camera.onGround(this.camera.viewport[0] / 2, this.camera.viewport[1] / 2) : null;
          press.pivot = centre || this.pivot();
        }
        canvas.classList.add('dragging');
      });
      canvas.addEventListener('pointermove', (event) => {
        if (!press || event.pointerId !== press.id) return;
        const [x, y] = at(event);
        const [dx, dy] = [x - press.x, y - press.y];
        // A stroke of paint follows the pointer from the first pixel.
        if (press.paint) {
          press.moved = true;
          GL.painter.drag(x, y);
          return;
        }
        if (!press.moved && Math.hypot(x - press.from[0], y - press.from[1]) < CLICK) return;
        press.moved = true;
        press.x = x;
        press.y = y;
        if (press.grab) {
          GL.edit.drag(press.grab, x, y, event);
        } else if (press.button === press.turns) {
          // The left button turns the view (the right, swapped).
          this.turn(dx, dy, press.pivot);
        } else {
          // The right and the middle button move it.
          this.camera.pan(dx, dy);
        }
        this.frame();
      });
      const release = (event) => {
        if (!press || event.pointerId !== press.id) return;
        const done = press;
        press = null;
        canvas.classList.remove('dragging');
        if (done.paint) { GL.painter.release(); return; }
        if (done.grab && done.moved) GL.edit.drop(done.grab, event.type === 'pointerup');
        // A press that is a tool's own when it is let go where it was
        // ("Cut": a click on its plane flips it).
        else if (!done.moved && event.type === 'pointerup' && done.grab && done.grab.tap) { done.grab.tap(); this.frame(); }
        // A face laid on the bed was the press itself (scene-edit.js).
        else if (!done.moved && event.type === 'pointerup' && !(done.grab && done.grab.kind === 'face')) this.click(done, event);
      };
      // "Camera style: Touchpad": the pointer moved with Alt turns the
      // view, with Shift moves it, no button held.
      let glide = null;
      canvas.addEventListener('pointermove', (event) => {
        if (press || !this.model || !touchpad() || event.buttons) { glide = null; return; }
        const [x, y] = at(event);
        const turning = event.altKey && !event.shiftKey;
        const moving = event.shiftKey && !event.altKey;
        if (!turning && !moving) { glide = null; return; }
        if (glide) {
          const [dx, dy] = [x - glide[0], y - glide[1]];
          if (turning) this.turn(dx, dy, this.pivot());
          else this.camera.pan(dx, dy);
          this.frame();
        }
        glide = [x, y];
      });
      // The handle under the pointer is shown as taken up.
      canvas.addEventListener('pointermove', (event) => {
        if (press || !this.model || this.model.tab !== 'prepare' || !GL.edit) return;
        const [x, y] = at(event);
        if (GL.painter && GL.painter.active()) { GL.painter.hover(x, y); return; }
        if (GL.edit.hover(x, y)) this.frame();
      });
      canvas.addEventListener('pointerleave', () => { if (!press && GL.painter && GL.painter.active()) GL.painter.leave(); });
      canvas.addEventListener('pointerup', release);
      canvas.addEventListener('pointercancel', release);
      canvas.addEventListener('wheel', (event) => {
        event.preventDefault();
        if (!this.model) return;
        // A painting tool takes the wheel with Ctrl (its size) or Alt (its
        // section view).
        if (this.model.tab === 'prepare' && GL.painter && GL.painter.active() && GL.painter.wheel(event)) return;
        // A notch of a wheel is a step; a pad sends many small ones, and
        // a pinch comes as a wheel with Ctrl.
        const notch = event.deltaMode === 1 ? 3 : event.ctrlKey ? 10 : 100;
        const steps = (-event.deltaY / notch) * (pref('reverse_mouse_wheel_zoom') ? -1 : 1);
        const [x, y] = at(event);
        if (pref('zoom_to_mouse')) this.camera.zoomAt(steps, x, y);
        else this.camera.updateZoom(steps);
        this.frame();
      }, { passive: false });
      document.addEventListener('keydown', (event) => this.key(event));
      new ResizeObserver(() => this.draw()).observe(canvas);
      document.addEventListener('themechange', () => this.frame());
      canvas.addEventListener('webglcontextlost', (event) => {
        event.preventDefault();
        this.host.say('The graphics card took the 3D view away; reload the page to have it back.', true);
      });
    },

    // The view turned by a drag of (dx, dy) pixels: about `pivot`, kept
    // upright, or with "Use free camera" about the camera's own axes
    // (Camera.cpp `rotate_local_around_target`); "Orbit speed
    // multiplier" scales it.
    turn(dx, dy, pivot) {
      const step = orbit();
      if (pref('use_free_camera')) this.camera.rotateFree(dx * step, dy * step);
      else this.camera.rotateOnSphere(dx * step, dy * step, true, pivot);
      if (this.prefs.autoPerspective) this.camera.perspective = true;
    },

    // A click: in Prepare it selects (GLCanvas3D.cpp `on_mouse`, the left
    // and right button released without a drag).
    click(press, event) {
      if (this.model.tab !== 'prepare' || !this.scene.data) return;
      // "Cut" takes the right button with no key held: the piece under it
      // goes to the other side (GLGizmoCut.cpp `on_mouse`); on a curved
      // surface's handle, with Shift too, it snaps the handle.
      const handle = GL.cutter && GL.cutter.active() && GL.cutter.controlUnder(press.x, press.y) >= 0;
      if (press.button === 2 && GL.cutter && GL.cutter.active() && (!event.shiftKey || handle) && !event.ctrlKey && !event.metaKey && !event.altKey) {
        GL.cutter.rightClick(press.x, press.y, event);
        return;
      }
      const mine = this.mine();
      const frame = this.matrices();
      const hit = this.scene.pick(this.camera, frame, press.x, press.y, { plates: true });
      if (hit && hit.placing !== undefined) {
        const placing = this.scene.data.placings[hit.placing];
        const id = placing.id;
        // Alt: "Part selection" (GLCanvas3D.cpp `on_mouse`, its volume
        // mode): the part under the pointer alone; with Shift as well,
        // added to the parts of that copy selected, or taken away.
        const at = event.altKey && press.button === 0 ? this.scene.pickPart(frame, press.x, press.y, hit.placing) : null;
        const part = at !== null ? (this.scene.data.objects[placing.object].part_list || [])[at] : null;
        const parts = mine.parts || new Set();
        if (part && (this.scene.data.objects[placing.object].part_list || []).length > 1) {
          if (event.shiftKey && mine.selection.has(id) && mine.selection.size === 1 && parts.size) {
            const chosen = new Set(parts);
            if (chosen.has(part.id)) chosen.delete(part.id); else chosen.add(part.id);
            if (chosen.size) this.selectParts(id, chosen); else this.select([id]);
          } else {
            this.selectParts(id, [part.id]);
          }
        } else if (press.button === 0 && (event.ctrlKey || event.metaKey || event.shiftKey)) {
          // Ctrl or Shift adds an object to those selected, or takes it away.
          const chosen = parts.size ? new Set() : new Set(mine.selection);
          if (chosen.has(id)) chosen.delete(id); else chosen.add(id);
          this.select(chosen);
        } else if (press.button === 2 && parts.size && mine.selection.has(id)) {
          // The right button on a copy whose parts are selected: their menu.
          if (GL.edit) GL.edit.partMenu(event.clientX, event.clientY);
        } else if (press.button === 0 || press.button === 2) {
          // The right button selects too, and opens the object's menu.
          if (press.button === 0 || !mine.selection.has(id) || parts.size) this.select([id]);
          if (press.button === 2 && GL.edit) GL.edit.menu(event.clientX, event.clientY);
        }
      } else if (hit && hit.plate && press.button === 0) {
        // A click on a plate selects the plate and lets go of the objects.
        this.select([]);
        if (hit.plate.id !== null && hit.plate.id !== this.plateId()) this.host.plate(hit.plate.id);
      } else if (hit && hit.plate && press.button === 2 && hit.plate.id !== null && GL.edit) {
        // The right button on a plate opens the plate's menu.
        GL.edit.plateMenu(event.clientX, event.clientY, hit.plate.id);
      } else if (!event.shiftKey) {
        this.select([]);
      }
      this.frame();
    },

    // What is selected: copies of objects, by their `id` in the scene.
    // While parts are selected it is the one copy they are picked in.
    selection() {
      return this.model && this.model.project ? this.mine().selection : new Set();
    },

    // The parts selected, by their `id` (`part_list[].id`): empty unless
    // parts and not objects are selected.
    parts() {
      return this.model && this.model.project ? (this.mine().parts || new Set()) : new Set();
    },

    // Selects these copies and no others, and tells whoever shows the
    // selection elsewhere (the object list, the tools).
    select(ids) {
      if (!this.model || !this.model.project) return;
      this.mine().selection = new Set(ids);
      this.mine().parts = new Set();
      if (GL.edit) GL.edit.selectionChanged();
      this.frame();
    },

    // Selects parts of one object, as they are in one copy of it.
    selectParts(copy, ids) {
      if (!this.model || !this.model.project) return;
      this.mine().selection = new Set([copy]);
      this.mine().parts = new Set(ids);
      if (GL.edit) GL.edit.selectionChanged();
      this.frame();
    },

    key(event) {
      if (!this.active || !this.model || this.model.tab === 'device' || document.querySelector('dialog[open]')) return;
      if (event.target.matches('input, select, textarea') || event.altKey) return;
      const command = event.ctrlKey || event.metaKey;
      const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
      const done = () => { event.preventDefault(); this.frame(); };
      if (command && !event.shiftKey) {
        // Ctrl+7: the plate shown from above. Ctrl+0 to Ctrl+6 (the View
        // menu's), Ctrl+A and Ctrl+E are the menu bar's keys (menu.js).
        if (/^(Digit|Numpad)7$/.test(event.code)) { this.view('this plate'); done(); }
        return;
      }
      if (command) return;
      if (key === 'i' || key === 'o') { this.camera.updateZoom(key === 'i' ? 1 : -1); done(); return; }
      // Escape ("Deselect all") is the menu bar's key (menu.js).
      if (this.model.tab !== 'preview' || !this.path()) return;
      const fast = event.shiftKey ? 5 : 1;
      if (key === 'l' && !event.shiftKey) { this.layerSlider.setOne(!this.layerSlider.one); done(); return; }
      // C ("Show G-code Window") is the menu bar's key (menu.js).
      if (key === 'ArrowLeft' || key === 'ArrowRight') { this.moveSlider.step((key === 'ArrowRight' ? 1 : -1) * fast); done(); return; }
      if (key === 'Home') { this.moveSlider.move(this.moveSlider.least); done(); return; }
      if (key === 'End') { this.moveSlider.move(this.moveSlider.most); done(); }
    },

    // ---- drawing ----

    frame() {
      if (this.pending || !this.active) return;
      this.pending = requestAnimationFrame(() => {
        this.pending = 0;
        this.draw();
      });
    },

    // The camera is told how large the view is.
    sized() {
      this.renderer.resize();
      this.camera.viewport = this.renderer.css.slice();
    },

    // The camera's matrices for the view as it is now.
    matrices() {
      this.sized();
      let box = this.scene.bounds();
      const path = this.path();
      const origin = this.plateOrigin() || [0, 0];
      if (path && path.index.box) {
        box = M.union(box, [M.add(path.index.box[0], [...origin, 0]), M.add(path.index.box[1], [...origin, 0])]);
      }
      this.camera.sceneBox = box;
      return this.camera.matrices(box);
    },

    draw() {
      if (!this.active) return;
      if (this.pending) {
        cancelAnimationFrame(this.pending);
        this.pending = 0;
      }
      const r = this.renderer;
      const model = this.model;
      r.resize();
      if (!model || !model.project || model.tab === 'device' || r.css[0] < 2 || r.css[1] < 2) return;
      const frame = this.matrices();
      const mine = this.mine();
      const plate = this.plateId();
      // From under the bed, the bed is seen through (3DBed.cpp, `bottom`).
      const below = this.camera.forward[2] >= 0;
      r.begin(this.scene.background);
      if (model.tab === 'prepare') {
        this.scene.drawObjects(frame, 0);
        this.scene.drawBed(frame, { current: plate, all: true, below });
        this.scene.drawObjects(frame, 1);
        this.scene.drawSelection(frame, mine.selection, mine.parts, GL.edit && mine.parts && mine.parts.size ? GL.edit.partBox() : null);
        if (GL.edit) GL.edit.draw(frame);
      } else {
        this.scene.drawBed(frame, { current: plate, all: false, below });
        const path = this.path();
        const range = this.range();
        // The objects under the print, seen through; while there is no
        // print, they are all there is to show of the plate.
        this.scene.drawShells(frame, plate);
        if (path && range) {
          const kept = model.kept;
          const view = T.VIEWS.find((known) => known.name === kept.viewType) || T.VIEWS[0];
          const options = kept.options || new Set(['Seams']);
          const hiddenFeatures = this.hiddenFeatures(kept);
          const colours = {};
          for (const filament of path.index.filaments) colours[filament] = hex(this.host.filamentColour(model.summary, filament));
          path.setColours(colours);
          const origin = [...(this.plateOrigin() || [0, 0]), 0];
          path.draw(frame, origin, range.from, range.to, {
            view: view.id,
            range: view.id >= 2 ? path.range(view.id, hiddenFeatures, options.has('Travel'), options.has('Wipe')) : [0, 0, 0],
            hiddenFeatures,
            hiddenFilaments: [...kept.hidden].filter((key) => /^f\d+$/.test(key)).map((key) => Number(key.slice(1))),
            options,
          });
          // The nozzle, while the print is stopped part of the way.
          if (range.to < range.end && range.to - 1 < path.loaded) {
            const at = M.add(path.move(range.to - 1).at, origin);
            r.blend(true, true);
            r.drawMesh(this.marker, frame, M.translation(...at), { tint: [1, 1, 1, 0.5] });
            r.blend(false);
          }
        }
      }
      this.navigator.update(this.camera);
      this.showLabels(frame);
      mine.camera = this.camera.save();
      this.drawn = (this.drawn || 0) + 1;
    },

    // A sliced file's pictures of each plate in `plates` (Plater.cpp
    // `export_3mf`, `generate_thumbnail`): 512 × 512 PNGs with nothing
    // behind the objects; the plate from the default view, lit and
    // unlit, and from above, lit and with each copy in the colour of its
    // number (`numbers`: { copy id: number }, the pick picture a printer
    // that skips objects reads); and the first at 128 × 128. As base64,
    // by plate number. Only the printable copies on each plate are drawn.
    platePictures(plates, numbers) {
      const out = {};
      if (!this.scene || !this.scene.data) return out;
      const r = this.renderer;
      const gl = r.gl;
      const SIZE = 512;
      const target = gl.createFramebuffer();
      gl.bindFramebuffer(gl.FRAMEBUFFER, target);
      const colour = gl.createRenderbuffer();
      gl.bindRenderbuffer(gl.RENDERBUFFER, colour);
      gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA8, SIZE, SIZE);
      gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, colour);
      const depth = gl.createRenderbuffer();
      gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
      gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT24, SIZE, SIZE);
      gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
      const pixels = new Uint8Array(SIZE * SIZE * 4);
      const canvas = document.createElement('canvas');
      canvas.width = SIZE;
      canvas.height = SIZE;
      const context = canvas.getContext('2d');
      const small = document.createElement('canvas');
      small.width = 128;
      small.height = 128;
      const smallContext = small.getContext('2d');
      const png = (from) => from.toDataURL('image/png').slice('data:image/png;base64,'.length);
      const scene = this.scene;
      const draw = (plate, view, how) => {
        const placings = scene.data.placings.filter((placing) => (placing.plate === plate || scene.plates.length <= 1) && placing.printable);
        let box = null;
        for (const placing of placings) if (placing.box) box = M.union(box, placing.box);
        const camera = new GL.Camera();
        camera.perspective = false;
        camera.viewport = [SIZE, SIZE];
        camera.selectView(view);
        // Fitted to the plate's bed, as the reference frames a plate.
        const fit = scene.plateBox(plate) || box;
        if (!fit) return false;
        const whole = box ? M.union(fit, box) : fit;
        camera.zoomToBox(whole, 1.0);
        camera.sceneBox = whole;
        const frame = camera.matrices(whole);
        gl.bindFramebuffer(gl.FRAMEBUFFER, target);
        gl.viewport(0, 0, SIZE, SIZE);
        gl.clearColor(0, 0, 0, 0);
        gl.depthMask(true);
        gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
        gl.enable(gl.DEPTH_TEST);
        gl.depthFunc(gl.LEQUAL);
        gl.disable(gl.BLEND);
        for (const placing of placings) {
          const mesh = scene.meshOf(placing.object);
          if (!mesh) continue;
          // The printed parts alone (`parts_only`).
          const parts = scene.data.objects[placing.object].part_list;
          const ranges = parts ? parts.filter((part) => part.kind === 'part').map((part) => part.triangles) : [null];
          for (const range of ranges) {
            if (how === 'pick') {
              const number = numbers[placing.id] || 0;
              r.drawFlat(mesh, M.multiply(M.multiply(frame.projection, frame.view), placing.matrix), GL.Renderer.pickColour(number), undefined, range);
            } else {
              r.drawMesh(mesh, frame, placing.matrix, { palette: scene.palette, pass: 2, range, emission: how === 'unlit' ? -1 : 0 });
            }
          }
        }
        gl.readPixels(0, 0, SIZE, SIZE, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
        // The frame buffer is upside down to a picture.
        const image = context.createImageData(SIZE, SIZE);
        const row = SIZE * 4;
        for (let y = 0; y < SIZE; y += 1) image.data.set(pixels.subarray((SIZE - 1 - y) * row, (SIZE - y) * row), y * row);
        context.putImageData(image, 0, 0);
        return true;
      };
      try {
        for (const plate of plates) {
          if (!draw(plate, 'iso', 'lit')) continue;
          const pictures = { plate: png(canvas) };
          smallContext.clearRect(0, 0, 128, 128);
          smallContext.imageSmoothingQuality = 'high';
          smallContext.drawImage(canvas, 0, 0, 128, 128);
          pictures.small = png(small);
          draw(plate, 'iso', 'unlit');
          pictures.no_light = png(canvas);
          draw(plate, 'top', 'lit');
          pictures.top = png(canvas);
          draw(plate, 'top', 'pick');
          pictures.pick = png(canvas);
          out[plate] = pictures;
        }
      } finally {
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.deleteFramebuffer(target);
        gl.deleteRenderbuffer(colour);
        gl.deleteRenderbuffer(depth);
        this.frame();
      }
      return out;
    },

    // The names of the objects on the plate shown, over them ("Show Labels").
    showLabels(frame) {
      const data = this.scene.data;
      const wanted = this.prefs.labels && data && this.model.tab === 'prepare';
      this.labels.hidden = !wanted;
      if (!wanted) return;
      const matrix = M.multiply(frame.projection, frame.view);
      const [width, height] = this.renderer.css;
      const plate = this.plateId();
      const mine = this.mine();
      const shown = [];
      data.placings.forEach((placing) => {
        const name = data.objects[placing.object].name;
        if (!name || !placing.box || (placing.plate !== plate && this.scene.plates.length > 1)) return;
        const top = [(placing.box[0][0] + placing.box[1][0]) / 2, (placing.box[0][1] + placing.box[1][1]) / 2, placing.box[1][2]];
        const at = M.point(matrix, top);
        shown.push({ name, x: (at[0] * 0.5 + 0.5) * width, y: (1 - (at[1] * 0.5 + 0.5)) * height, selected: mine.selection.has(placing.id) });
      });
      while (this.labels.children.length > shown.length) this.labels.lastChild.remove();
      while (this.labels.children.length < shown.length) this.labels.append(el('span'));
      shown.forEach((label, at) => {
        const node = this.labels.children[at];
        if (node.textContent !== label.name) node.textContent = label.name;
        node.className = label.selected ? 'selected' : '';
        node.style.left = `${label.x}px`;
        node.style.top = `${label.y}px`;
      });
    },

    // Turns the view round for a number of frames and says how long the
    // frames took: for measuring, not for looking at. The browser shows
    // a frame no sooner than the screen takes one, so `repeat` draws the
    // scene that many times a frame, to find how much room there is.
    async measure(frames = 120, repeat = 1) {
      const gl = this.renderer.gl;
      const pivot = this.pivot();
      const times = [];
      const spans = [];
      let last = performance.now();
      for (let frame = 0; frame < frames; frame += 1) {
        await new Promise((resolve) => requestAnimationFrame(resolve));
        const now = performance.now();
        if (frame) spans.push(now - last);
        last = now;
        this.camera.rotateOnSphere((2 * Math.PI) / frames, 0, true, pivot);
        const start = performance.now();
        for (let again = 0; again < repeat; again += 1) this.draw();
        gl.finish();
        times.push(performance.now() - start);
      }
      const mean = (list) => list.reduce((sum, value) => sum + value, 0) / Math.max(list.length, 1);
      const round = (value) => Math.round(value * 100) / 100;
      return {
        frames,
        repeat,
        draw_ms: round(mean(times)),
        draw_ms_worst: round(Math.max(...times)),
        between_frames_ms: round(mean(spans)),
        between_frames_ms_worst: round(Math.max(...spans)),
        gpu_bytes: this.renderer.bytes,
        renderer: (() => {
          const info = gl.getExtension('WEBGL_debug_renderer_info');
          return info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
        })(),
      };
    },
  };

  GL.stage = stage;
})(window.EdgeGL);
