'use strict';

// "Variable layer height" (GLCanvas3D.cpp `LayersEditing`): with one object
// selected, a bar beside the stage shows its layers, each coloured by how
// thick it is, and the profile they are laid by; a drag over the bar with
// the left button thins the layers round where it is (adds detail), with
// the right thickens them, with Shift and the left brings them back to the
// object's layer height and with Shift and the right smooths them; the
// wheel widens or narrows the band a stroke reaches. The panel has
// "Adaptive" with "Quality / Speed", "Smooth" with "Radius" and "Keep min",
// and "Reset". Every change is one `scene_edit` (`profile_*`), one step of
// the history; the layers are asked of the session (`layer_profile`).
//
// Not as the reference: a drag is sent when the button is let go, the band
// it reaches drawn while it lasts, where the reference changes the profile
// at every tick of its timer.
(function (GL) {
  // GLCanvas3D.cpp `LayersEditing::THICKNESS_BAR_WIDTH`.
  const BAR_WIDTH = 70;
  // Slicing.cpp `generate_layer_height_texture`: thinner to thicker.
  const PALETTE = [
    [0x1a, 0x98, 0x50], [0x66, 0xbd, 0x63], [0xa6, 0xd9, 0x6a], [0xd9, 0xf1, 0xeb],
    [0xfe, 0xe6, 0xeb], [0xfd, 0xae, 0x61], [0xf4, 0x6d, 0x43], [0xd7, 0x30, 0x27],
  ];
  // How often a held button strokes again, in ms.
  const TICK = 100;

  const vlh = {
    host: null,
    edit: null,
    open: false,
    // The object the layers are of, and its layers as the session gave them.
    object: null,
    layers: null,
    asked: 0,
    // GLCanvas3D.hpp: `m_adaptive_quality`, `m_smooth_params`, `band_width`,
    // `strength`.
    quality: 0.5,
    radius: 5,
    keepMin: false,
    bandWidth: 2,
    strength: 0.005,
    // The pointer over the bar, in mm up the object, and a drag going on.
    hover: null,
    drag: null,

    init(host, edit) {
      this.host = host;
      this.edit = edit;
      const { el } = host;
      this.bar = el('canvas', { className: 'vlh-bar overlay', id: 'vlh-bar', hidden: true });
      this.bar.setAttribute('role', 'img');
      this.bar.setAttribute('aria-label', t('Variable layer height'));
      this.panel = el('div', { className: 'edit-panel overlay vlh-panel', id: 'vlh-panel', hidden: true });
      this.panel.setAttribute('role', 'group');
      this.panel.setAttribute('aria-label', t('Variable layer height'));
      host.stageNode.append(this.bar, this.panel);
      this.buildPanel();
      this.bar.addEventListener('contextmenu', (event) => event.preventDefault());
      this.bar.addEventListener('pointerdown', (event) => this.press(event));
      this.bar.addEventListener('pointermove', (event) => this.move(event));
      this.bar.addEventListener('pointerup', (event) => this.release(event));
      this.bar.addEventListener('pointerleave', () => { if (!this.drag) { this.hover = null; this.draw(); } });
      this.bar.addEventListener('wheel', (event) => this.wheel(event), { passive: false });
      new ResizeObserver(() => this.draw()).observe(host.stageNode);
    },

    buildPanel() {
      const { el } = this.host;
      const row = (...children) => el('div', { className: 'vlh-row' }, ...children);
      const range = (min, max, step, value, set) => {
        const slider = el('input', { type: 'range', min, max, step, value });
        const number = el('input', { type: 'number', min, max, step, value, className: 'vlh-number' });
        const take = (from) => {
          const clamped = Math.min(max, Math.max(min, Number(from.value)));
          if (!Number.isFinite(clamped)) return;
          set(clamped);
          slider.value = clamped;
          number.value = clamped;
        };
        slider.addEventListener('input', () => take(slider));
        number.addEventListener('change', () => take(number));
        return [slider, number];
      };
      const button = (label, act) => {
        const node = el('button', { type: 'button', className: 'quiet vlh-button', textContent: t(label) });
        node.addEventListener('click', act);
        return node;
      };
      const quality = range(0, 1, 0.01, this.quality, (value) => { this.quality = value; });
      quality[0].setAttribute('aria-label', t('Quality / Speed'));
      quality[1].setAttribute('aria-label', t('Quality / Speed'));
      const radius = range(1, 10, 1, this.radius, (value) => { this.radius = Math.round(value); });
      radius[0].setAttribute('aria-label', t('Radius'));
      radius[1].setAttribute('aria-label', t('Radius'));
      const keep = el('input', { type: 'checkbox', id: 'vlh-keep-min' });
      keep.addEventListener('change', () => { this.keepMin = keep.checked; });
      const keepLabel = el('label', { htmlFor: 'vlh-keep-min', className: 'edit-check' }, keep, t('Keep min'));
      // The reference's captions of the mouse (`show_tooltip_information`).
      const hints = el('dl', { className: 'vlh-hints' });
      for (const [what, does] of [
        [`${t('Left mouse button')}:`, t('Add detail')],
        [`${t('Right mouse button')}:`, t('Remove detail')],
        [`${t('Shift+')}${t('Left mouse button')}:`, t('Reset to base')],
        [`${t('Shift+')}${t('Right mouse button')}:`, t('Smoothing')],
        [t('Mouse wheel:'), t('Increase/decrease edit area')],
      ]) {
        hints.append(el('dt', { textContent: what }), el('dd', { textContent: does }));
      }
      this.panel.append(
        row(button('Adaptive', () => this.change({ op: 'profile_adaptive', quality: this.quality })),
          el('span', { className: 'edit-label', textContent: t('Quality / Speed') }), ...quality),
        row(button('Smooth', () => this.change({ op: 'profile_smooth', radius: this.radius, keep_min: this.keepMin })),
          el('span', { className: 'edit-label', textContent: t('Radius') }), ...radius),
        row(keepLabel),
        el('hr'),
        hints,
        row(button('Reset', () => this.change({ op: 'profile_reset' }))),
      );
    },

    // The one object selected, when the tool may be used for it
    // (Plater.cpp `layers_height_allowed`: one object selected, standing
    // above the bed).
    target() {
      return this.edit.splitTarget();
    },

    allowed() {
      return Boolean(this.target()) && this.host.tab() === 'prepare';
    },

    toggle() {
      this.open = !this.open && this.allowed();
      this.refresh();
    },

    // As the scene and the selection now are: the layers asked again for
    // an object edited or another one.
    refresh() {
      if (!this.bar) return;
      if (this.open && !this.allowed()) this.open = false;
      this.bar.hidden = !this.open;
      this.panel.hidden = !this.open;
      if (this.edit.buttons && this.edit.buttons.layers) {
        this.edit.buttons.layers.setAttribute('aria-pressed', String(this.open));
      }
      if (!this.open) {
        this.layers = null;
        return;
      }
      this.ask(this.target().id);
    },

    async ask(object) {
      const project = this.host.project();
      if (!project) return;
      const asked = ++this.asked;
      try {
        const answer = await this.host.call('layer_profile', { project: project.id, object });
        if (asked !== this.asked) return;
        this.object = object;
        this.layers = answer;
        this.draw();
      } catch (error) {
        this.host.say(error.message, true);
      }
    },

    async change(args) {
      if (!this.object) return;
      await this.edit.commit({ ...args, object: this.object });
    },

    // ---- the bar ----

    // Where the bar stands in the stage, in CSS pixels.
    rect() {
      const stage = this.host.stageNode.getBoundingClientRect();
      const inset = 12;
      return { width: BAR_WIDTH, height: Math.max(80, stage.height - 2 * inset - 48), top: inset + 48 };
    },

    // The height up the object at a point of the bar.
    zAt(event) {
      const box = this.bar.getBoundingClientRect();
      const share = (box.bottom - event.clientY) / box.height;
      return Math.max(0, Math.min(1, share)) * (this.layers ? this.layers.height : 0);
    },

    draw() {
      if (!this.bar || this.bar.hidden) return;
      const { width, height, top } = this.rect();
      const ratio = window.devicePixelRatio || 1;
      this.bar.style.width = `${width}px`;
      this.bar.style.height = `${height}px`;
      this.bar.style.top = `${top}px`;
      this.bar.width = Math.round(width * ratio);
      this.bar.height = Math.round(height * ratio);
      const g = this.bar.getContext('2d');
      g.setTransform(ratio, 0, 0, ratio, 0, 0);
      g.clearRect(0, 0, width, height);
      const at = this.layers;
      if (!at || !(at.height > 0)) return;
      const y = (z) => height - (z / at.height) * height;
      // Each layer coloured by its thickness against the layer height
      // (`generate_layer_height_texture`).
      const scale = 2 * Math.max(at.max_layer_height - at.layer_height, at.layer_height - at.min_layer_height) || at.layer_height;
      for (const [low, high] of at.layers) {
        const index = ((0.5 * scale + (high - low - at.layer_height)) * (PALETTE.length - 1)) / scale;
        const first = Math.max(0, Math.min(PALETTE.length - 1, Math.floor(index)));
        const second = Math.min(PALETTE.length - 1, first + 1);
        const t2 = Math.max(0, Math.min(1, index - first));
        const colour = PALETTE[first].map((c, i) => Math.round(c * (1 - t2) + PALETTE[second][i] * t2));
        g.fillStyle = `rgb(${colour.join(',')})`;
        g.fillRect(0, y(Math.min(high, at.height)), width, Math.max(1, y(low) - y(Math.min(high, at.height))));
      }
      // The profile, across the bar by its height (`render_profile`).
      const scaleX = width / (1.12 * at.max_layer_height);
      const style = getComputedStyle(this.host.stageNode);
      g.strokeStyle = style.getPropertyValue('--text-1').trim() || '#fff';
      g.lineWidth = 1.5;
      g.beginPath();
      for (let i = 0; i < at.profile.length; i += 2) {
        const px = at.profile[i + 1] * scaleX;
        const py = y(at.profile[i]);
        if (i === 0) g.moveTo(px, py); else g.lineTo(px, py);
      }
      g.stroke();
      // The object's layer height, the axis the profile is drawn about.
      g.strokeStyle = 'rgba(0, 0, 0, 0.5)';
      g.setLineDash([4, 3]);
      g.beginPath();
      g.moveTo(at.layer_height * scaleX, 0);
      g.lineTo(at.layer_height * scaleX, height);
      g.stroke();
      g.setLineDash([]);
      // The band a stroke reaches, round the pointer.
      const z = this.drag ? this.drag.z : this.hover;
      if (z !== null && z !== undefined) {
        g.fillStyle = 'rgba(255, 255, 255, 0.25)';
        const half = (this.bandWidth / 2 / at.height) * height;
        g.fillRect(0, y(z) - half, width, 2 * half);
        g.fillStyle = 'rgba(0, 0, 0, 0.8)';
        g.fillRect(0, y(z) - 0.5, width, 1);
      }
    },

    // A stroke at the pointer's height (GLCanvas3D.cpp
    // `_perform_layer_editing_action`): the left button thins, the right
    // thickens; with Shift, back to the layer height and smoothing.
    stroke() {
      if (!this.drag) return;
      this.drag.strokes.push({ z: this.drag.z, action: this.drag.action });
    },

    press(event) {
      if (!this.layers || this.edit.busy) return;
      event.preventDefault();
      const right = event.button === 2;
      const action = event.shiftKey ? (right ? 'smooth' : 'reduce') : (right ? 'increase' : 'decrease');
      this.bar.setPointerCapture(event.pointerId);
      this.drag = { action, z: this.zAt(event), strokes: [] };
      this.stroke();
      this.drag.timer = setInterval(() => this.stroke(), TICK);
      this.draw();
    },

    move(event) {
      const z = this.zAt(event);
      if (this.drag) {
        this.drag.z = z;
        this.stroke();
      } else {
        this.hover = z;
      }
      this.draw();
    },

    async release(event) {
      if (!this.drag) return;
      clearInterval(this.drag.timer);
      const { strokes } = this.drag;
      this.drag = null;
      if (this.bar.hasPointerCapture(event.pointerId)) this.bar.releasePointerCapture(event.pointerId);
      this.draw();
      await this.change({ op: 'profile_strokes', strokes, band_width: this.bandWidth, strength: this.strength });
    },

    // The wheel widens or narrows the band, 1.5 to 10 mm (GLCanvas3D.cpp,
    // the mouse wheel over the bar).
    wheel(event) {
      event.preventDefault();
      const turns = -Math.sign(event.deltaY);
      this.bandWidth = Math.max(Math.min(this.bandWidth * (1 + 0.1 * turns), 10), 1.5);
      this.draw();
    },
  };

  GL.vlh = vlh;
})(window.EdgeGL);
