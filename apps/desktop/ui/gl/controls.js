'use strict';

// What is laid over the 3D views to drive them: the cube that shows which
// way the camera looks, the two sliders of Preview, and the legend's rows.
// All of it is the page's own HTML.
(function (GL) {
  function el(tag, props = {}, ...children) {
    const node = Object.assign(document.createElement(tag), props);
    node.append(...children);
    return node;
  }

  // The 3D navigator: a cube that turns with the camera, each face named
  // and a click away (GLCanvas3D.cpp `_render_3d_navigator`, which draws
  // ImGuizmo's; the names are its face labels). A face is the view from
  // that side.
  const FACES = [
    // name, the view it selects, where the face's own right, down and
    // outward point in the scene.
    ['Front', 'front', [1, 0, 0], [0, 0, -1], [0, -1, 0]],
    ['Back', 'rear', [-1, 0, 0], [0, 0, -1], [0, 1, 0]],
    ['Left', 'left', [0, -1, 0], [0, 0, -1], [-1, 0, 0]],
    ['Right', 'right', [0, 1, 0], [0, 0, -1], [1, 0, 0]],
    ['Top', 'top', [1, 0, 0], [0, -1, 0], [0, 0, 1]],
    ['Bottom', 'bottom', [1, 0, 0], [0, 1, 0], [0, 0, -1]],
  ];

  class Navigator {
    constructor(parent, onView) {
      this.half = 26;
      this.cube = el('div', { className: 'gl-cube' });
      for (const [name, view, right, down, out] of FACES) {
        const face = el('button', { type: 'button', className: 'gl-face', textContent: t(name), title: t(name) });
        const h = this.half;
        // The face's own axes and where its middle is, in the scene's.
        face.style.transform = `matrix3d(${right.join(',')},0,${down.join(',')},0,${out.join(',')},0,${out.map((part) => part * h).join(',')},1)`;
        face.addEventListener('click', () => onView(view));
        this.cube.append(face);
      }
      this.node = el('div', { className: 'gl-navigator', title: '' }, this.cube);
      this.node.setAttribute('aria-label', '3D Navigator');
      parent.append(this.node);
    }

    // Turns the cube as the camera sees the scene: the scene's axes to
    // the screen's, whose y runs down.
    update(camera) {
      const [x, y, z] = camera.rotation;
      const m = [x[0], -y[0], z[0], 0, x[1], -y[1], z[1], 0, x[2], -y[2], z[2], 0, 0, 0, 0, 1];
      const text = `matrix3d(${m.map((value) => value.toFixed(5)).join(',')})`;
      if (text !== this.shown) {
        this.cube.style.transform = text;
        this.shown = text;
      }
    }
  }

  // The layers slider: upright at the right of the view, with two thumbs
  // that bound the layers shown, each with the layer's number over its
  // height beside it (IMSlider.cpp; `get_label`). Below it, the switch for
  // one layer mode, in which one thumb shows one layer.
  class LayerSlider {
    constructor(parent, { onChange, onMenu }) {
      this.onChange = onChange;
      this.count = 0;
      this.low = 0;
      this.high = 0;
      this.one = false;
      this.selected = 'high';
      this.label = () => ['', ''];
      this.fill = el('div', { className: 'gl-fill' });
      this.thumbs = {
        high: el('div', { className: 'gl-thumb high' }),
        low: el('div', { className: 'gl-thumb low' }),
      };
      this.labels = {
        high: el('output', { className: 'gl-layer-label high overlay chip-label' }),
        low: el('output', { className: 'gl-layer-label low overlay chip-label' }),
      };
      this.track = el('div', { className: 'gl-track', tabIndex: 0 }, this.fill, this.thumbs.low, this.thumbs.high);
      this.track.setAttribute('role', 'slider');
      this.track.setAttribute('aria-orientation', 'vertical');
      this.track.setAttribute('aria-label', t('Layers'));
      this.oneButton = el('button', { type: 'button', className: 'gl-one overlay', textContent: '1', title: `${t('On/Off one layer mode of the vertical slider')} [L]` });
      this.oneButton.setAttribute('aria-pressed', 'false');
      this.oneButton.addEventListener('click', () => this.setOne(!this.one));
      // What is set at layers: a mark across the track at each, its kind
      // beside it (IMSlider.cpp `draw_ticks`, `draw_custom_label_block`).
      this.ticks = [];
      this.tickLayer = el('div', { className: 'gl-ticks' });
      this.node = el('div', { className: 'gl-layers', hidden: true }, this.tickLayer, this.labels.high, this.labels.low, this.track, this.oneButton);
      parent.append(this.node);

      let dragging = false;
      const at = (event) => {
        const box = this.track.getBoundingClientRect();
        const part = 1 - Math.max(0, Math.min(1, (event.clientY - box.top) / box.height));
        return Math.round(part * Math.max(this.count - 1, 0));
      };
      this.track.addEventListener('pointerdown', (event) => {
        if (event.button !== 0 || !this.count) return;
        // A drag that leaves the track would otherwise select the page's
        // text from here on, painting the legend in the selection's colour.
        event.preventDefault();
        this.track.focus({ preventScroll: true });
        dragging = true;
        this.track.setPointerCapture(event.pointerId);
        const layer = at(event);
        // The thumb nearer to where the track was pressed is taken up.
        if (!this.one) this.select(Math.abs(layer - this.high) <= Math.abs(layer - this.low) ? 'high' : 'low');
        this.move(layer);
      });
      this.track.addEventListener('pointermove', (event) => { if (dragging) this.move(at(event)); });
      for (const end of ['pointerup', 'pointercancel']) this.track.addEventListener(end, () => { dragging = false; });
      // The wheel moves the thumb taken up a layer at a time; five with
      // Shift or Ctrl held.
      this.node.addEventListener('wheel', (event) => {
        event.preventDefault();
        this.step((event.deltaY < 0 ? 1 : -1) * (event.shiftKey || event.ctrlKey || event.metaKey ? 5 : 1));
      }, { passive: false });
      this.node.addEventListener('contextmenu', (event) => {
        event.preventDefault();
        onMenu(event.clientX, event.clientY);
      });
    }

    // The items set at layers: [{ layer, kind, label, title, colour,
    // remove }], `remove` the words of the button that takes it away,
    // shown on the item at the layer the thumb taken up is on.
    setTicks(ticks, onDelete) {
      this.ticks = ticks.map((tick) => {
        const mark = el('span', { className: 'gl-tick-mark' });
        if (tick.colour) mark.style.background = tick.colour;
        const label = el('span', { className: 'gl-tick-label', textContent: tick.label, title: tick.title });
        label.addEventListener('click', () => this.move(tick.layer));
        const remove = el('button', { type: 'button', className: 'gl-tick-delete icon', title: tick.remove });
        remove.setAttribute('aria-label', tick.remove);
        remove.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5,4.5l7,7M11.5,4.5l-7,7"/></svg>';
        remove.addEventListener('click', () => onDelete(tick));
        const node = el('div', { className: `gl-tick ${tick.kind}` }, label, mark, remove);
        return { ...tick, node };
      });
      this.tickLayer.replaceChildren(...this.ticks.map((tick) => tick.node));
      this.show();
    }

    // The item at the layer the thumb taken up is on, if there is one.
    tickHere() {
      const layer = this.one || this.selected === 'high' ? this.high : this.low;
      return this.ticks.find((tick) => tick.layer === layer) || null;
    }

    select(which) {
      this.selected = which;
      this.thumbs.high.classList.toggle('active', which === 'high');
      this.thumbs.low.classList.toggle('active', which === 'low');
    }

    // Puts the thumb taken up at a layer; the other gives way.
    move(layer, tell = true) {
      layer = Math.max(0, Math.min(this.count - 1, Math.round(layer)));
      if (this.one) {
        this.low = layer;
        this.high = layer;
      } else if (this.selected === 'high') {
        this.high = layer;
        this.low = Math.min(this.low, layer);
      } else {
        this.low = layer;
        this.high = Math.max(this.high, layer);
      }
      this.show();
      if (tell) this.onChange(this.low, this.high, this.selected);
    }

    step(by) {
      this.move((this.one || this.selected === 'high' ? this.high : this.low) + by);
    }

    // One layer mode: the layer the upper thumb was on, alone; leaving
    // it, every layer up to it again.
    setOne(one, tell = true) {
      this.one = one;
      this.oneButton.setAttribute('aria-pressed', String(one));
      this.node.classList.toggle('one', one);
      if (one) {
        this.low = this.high;
        this.select('high');
      } else {
        this.low = 0;
      }
      this.show();
      if (tell) this.onChange(this.low, this.high, this.selected);
    }

    // The slider for `count` layers, the thumbs where they are said to be.
    set(count, low, high, label) {
      this.count = count;
      this.label = label;
      this.high = Math.max(0, Math.min(count - 1, high));
      this.low = this.one ? this.high : Math.max(0, Math.min(this.high, low));
      this.node.hidden = count === 0;
      this.select(this.selected);
      this.show();
    }

    show() {
      const part = (layer) => (this.count > 1 ? layer / (this.count - 1) : 0);
      for (const which of ['high', 'low']) {
        const at = `${100 * part(this[which])}%`;
        this.thumbs[which].style.bottom = at;
        this.labels[which].style.bottom = at;
        const [number, height] = this.label(this[which]);
        this.labels[which].textContent = `${number}\n${height}`;
      }
      const here = this.tickHere();
      for (const tick of this.ticks) {
        tick.node.style.bottom = `${100 * part(tick.layer)}%`;
        tick.node.classList.toggle('here', tick === here);
      }
      this.fill.style.bottom = `${100 * part(this.low)}%`;
      this.fill.style.height = `${100 * (part(this.high) - part(this.low))}%`;
      this.track.setAttribute('aria-valuemin', '1');
      this.track.setAttribute('aria-valuemax', String(this.count));
      this.track.setAttribute('aria-valuenow', String(this.high + 1));
      this.track.setAttribute('aria-valuetext', this.low === this.high
        ? `Layer ${this.high + 1} of ${this.count}`
        : `Layers ${this.low + 1} to ${this.high + 1} of ${this.count}`);
    }
  }

  // The moves slider: along the bottom of the view, how far through the
  // layers shown the print has got (IMSlider.cpp, the horizontal one).
  class MoveSlider {
    constructor(parent, { onChange }) {
      this.onChange = onChange;
      this.least = 0;
      this.most = 0;
      this.value = 0;
      this.fill = el('div', { className: 'gl-fill' });
      this.thumb = el('div', { className: 'gl-thumb' });
      this.label = el('output', { className: 'gl-move-label overlay chip-label' });
      this.track = el('div', { className: 'gl-track', tabIndex: 0 }, this.fill, this.thumb, this.label);
      this.track.setAttribute('role', 'slider');
      this.track.setAttribute('aria-label', 'Moves');
      this.node = el('div', { className: 'gl-moves', hidden: true }, this.track);
      parent.append(this.node);
      let dragging = false;
      const at = (event) => {
        const box = this.track.getBoundingClientRect();
        const part = Math.max(0, Math.min(1, (event.clientX - box.left) / box.width));
        return Math.round(this.least + part * (this.most - this.least));
      };
      this.track.addEventListener('pointerdown', (event) => {
        if (event.button !== 0) return;
        // As for the layers slider: no text selection from a drag.
        event.preventDefault();
        this.track.focus({ preventScroll: true });
        dragging = true;
        this.track.setPointerCapture(event.pointerId);
        this.move(at(event));
      });
      this.track.addEventListener('pointermove', (event) => { if (dragging) this.move(at(event)); });
      for (const end of ['pointerup', 'pointercancel']) this.track.addEventListener(end, () => { dragging = false; });
      this.node.addEventListener('wheel', (event) => {
        event.preventDefault();
        this.step((event.deltaY < 0 ? 1 : -1) * (event.shiftKey || event.ctrlKey || event.metaKey ? 5 : 1));
      }, { passive: false });
    }

    move(value, tell = true) {
      this.value = Math.max(this.least, Math.min(this.most, Math.round(value)));
      this.show();
      if (tell) this.onChange(this.value);
    }

    step(by) { this.move(this.value + by); }

    set(least, most, value) {
      this.least = least;
      this.most = Math.max(most, least);
      this.value = Math.max(this.least, Math.min(this.most, value));
      this.node.hidden = this.most <= this.least;
      this.show();
    }

    show() {
      const part = this.most > this.least ? (this.value - this.least) / (this.most - this.least) : 1;
      this.thumb.style.left = `${100 * part}%`;
      this.label.style.left = `${100 * part}%`;
      this.fill.style.width = `${100 * part}%`;
      // Counted from the first move shown, as the reference labels it.
      this.label.textContent = String(this.value - this.least);
      this.track.setAttribute('aria-valuemin', '0');
      this.track.setAttribute('aria-valuemax', String(this.most - this.least));
      this.track.setAttribute('aria-valuenow', String(this.value - this.least));
    }
  }

  GL.controls = { el, Navigator, LayerSlider, MoveSlider };
})(window.EdgeGL);
