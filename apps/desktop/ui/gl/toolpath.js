'use strict';

// A print as Preview shows it in 3D: every line laid, as wide and as high
// as it is, and the moves and marks between them, drawn the way
// libvgcode draws them (src/libvgcode). The moves come from the session a
// stretch at a time (`toolpath_moves`; the layout of a move is written
// out in api/scene.rs) and go to the graphics card as they come.
(function (GL) {
  const M = GL.math;
  const MOVE = 32;
  const KIND = { start: 0, travel: 1, line: 2, wipe: 3, retract: 4, unretract: 5, seam: 6, toolChange: 7 };

  // libvgcode ViewerImpl.cpp DEFAULT_EXTRUSION_ROLES_COLORS, by the
  // feature's number, and the names the legend gives them
  // (ExtrusionEntity.cpp `role_to_string`).
  const FEATURES = [
    ['Undefined', [230, 179, 179]], ['Inner wall', [255, 230, 77]], ['Outer wall', [255, 125, 56]],
    ['Overhang wall', [31, 31, 255]], ['Sparse infill', [176, 48, 41]], ['Internal solid infill', [150, 84, 204]],
    ['Top surface', [240, 64, 64]], ['Ironing', [255, 140, 105]], ['Bridge', [77, 128, 186]],
    ['Gap infill', [255, 255, 255]], ['Skirt', [0, 135, 110]], ['Support', [0, 255, 0]],
    ['Support interface', [0, 128, 0]], ['Prime tower', [179, 227, 171]], ['Custom', [94, 209, 148]],
    ['Bottom surface', [102, 92, 199]], ['Internal Bridge', [77, 128, 186]], ['Brim', [0, 59, 110]],
    ['Support transition', [0, 64, 0]], ['Multiple', [128, 128, 128]], ['Wall over support', [125, 230, 219]],
    ['Bottom surface over support', [41, 194, 184]],
  ];
  // libvgcode ViewerImpl.cpp DEFAULT_OPTIONS_COLORS, with the legend's
  // names (GCodeViewer.cpp `append_option_item`), in the legend's order.
  // `count` is what `toolpath` calls them; `shown` is whether the
  // reference shows them until told otherwise (libvgcode Settings.hpp).
  const OPTIONS = [
    { name: 'Travel', kind: KIND.travel, colour: [56, 72, 155], count: 'travel', shown: false },
    { name: 'Wipe', kind: KIND.wipe, colour: [255, 255, 0], count: 'wipe', shown: false },
    { name: 'Retract', kind: KIND.retract, colour: [205, 34, 214], count: 'retract', shown: false },
    { name: 'Unretract', kind: KIND.unretract, colour: [73, 173, 207], count: 'unretract', shown: false },
    { name: 'Seams', kind: KIND.seam, colour: [230, 230, 230], count: 'seam', shown: true },
    { name: 'Filament changes', kind: KIND.toolChange, colour: [193, 190, 99], count: 'tool_change', shown: false },
  ];
  // libvgcode include/ColorRange.hpp DEFAULT_RANGES_COLORS, least first.
  const RANGE = [[11, 44, 122], [19, 89, 133], [28, 136, 145], [4, 214, 15], [170, 242, 0], [252, 249, 3], [245, 206, 10], [227, 136, 32], [209, 104, 48], [194, 82, 60], [148, 38, 22]];

  // The views, in the order of the reference's list (GCodeViewer.cpp
  // `update_by_mode`), with the legend's title and how many decimals it
  // shows (`render_legend`). `needs` a print read back from its G-code.
  const VIEWS = [
    { name: 'Line Type', id: 0 },
    { name: 'Filament', id: 1 },
    { name: 'Speed', id: 2, title: 'Speed (mm/s)', decimals: 0, needs: true },
    { name: 'Layer Height', id: 3, title: 'Layer height (mm)', decimals: 2 },
    { name: 'Line Width', id: 4, title: 'Line width (mm)', decimals: 2 },
    { name: 'Flow', id: 5, title: 'Volumetric flow rate (mm³/s)', decimals: 2, needs: true },
    { name: 'Fan Speed', id: 6, title: 'Fan speed (%)', decimals: 0, needs: true },
    { name: 'Temperature', id: 7, title: 'Temperature (℃)', decimals: 0, needs: true },
  ];

  // libvgcode ViewerImpl.cpp `round_to_bin`: two digits are enough to
  // tell values apart.
  function roundToBin(value) {
    const thresholds = [0.095, 0.0095, 0.00095, 0.000095, 0.0000095];
    let at = 0;
    while (at < 4 && value < thresholds[at]) at += 1;
    const scale = 10 ** (at + 2);
    return Math.floor(value * scale + 0.5) / scale;
  }

  // libvgcode OptionTemplate.cpp: two cones base to base, 16 sides, one
  // across and one high. Position and normal for each vertex.
  function diamond() {
    const out = [];
    const step = (2 * Math.PI) / 16;
    for (let side = 0; side < 16; side += 1) {
      const [a, b] = [side * step, (side + 1) * step];
      const [ca, sa, cb, sb] = [Math.cos(a), Math.sin(a), Math.cos(b), Math.sin(b)];
      out.push(0, 0, 0.5, 0, 0, 1, 0.5 * ca, 0.5 * sa, 0, ca, sa, 0, 0.5 * cb, 0.5 * sb, 0, cb, sb, 0);
      out.push(0, 0, -0.5, 0, 0, -1, 0.5 * cb, 0.5 * sb, 0, cb, sb, 0, 0.5 * ca, 0.5 * sa, 0, ca, sa, 0);
    }
    return new Float32Array(out);
  }

  // Where the nozzle is: an arrow pointing down at it, its tip half a
  // millimetre above (GCodeViewer.cpp, the marker: stilized_arrow(16,
  // 1.5, 3.0, 0.8, 3.0) turned over, `m_model_z_offset` 0.5).
  function arrow() {
    const [tipRadius, tipHeight, stemRadius, stemHeight, lift] = [1.5, 3.0, 0.8, 3.0, 0.5];
    const points = [];
    const step = (2 * Math.PI) / 16;
    const [neck, top] = [lift + tipHeight, lift + tipHeight + stemHeight];
    for (let side = 0; side < 16; side += 1) {
      const [a, b] = [side * step, (side + 1) * step];
      const [ca, sa, cb, sb] = [Math.cos(a), Math.sin(a), Math.cos(b), Math.sin(b)];
      const slope = (c, s) => [c * tipHeight, s * tipHeight, -tipRadius];
      // The tip, the ring round its top, the stem and its end.
      points.push([[0, 0, lift], slope((ca + cb) / 2, (sa + sb) / 2)], [[tipRadius * cb, tipRadius * sb, neck], slope(cb, sb)], [[tipRadius * ca, tipRadius * sa, neck], slope(ca, sa)]);
      for (const [p, q, r] of [[[tipRadius * ca, tipRadius * sa], [tipRadius * cb, tipRadius * sb], [stemRadius * cb, stemRadius * sb]], [[tipRadius * ca, tipRadius * sa], [stemRadius * cb, stemRadius * sb], [stemRadius * ca, stemRadius * sa]]]) {
        points.push([[...p, neck], [0, 0, 1]], [[...q, neck], [0, 0, 1]], [[...r, neck], [0, 0, 1]]);
      }
      const [pa, pb] = [[stemRadius * ca, stemRadius * sa], [stemRadius * cb, stemRadius * sb]];
      points.push([[...pa, neck], [ca, sa, 0]], [[...pb, neck], [cb, sb, 0]], [[...pb, top], [cb, sb, 0]]);
      points.push([[...pa, neck], [ca, sa, 0]], [[...pb, top], [cb, sb, 0]], [[...pa, top], [ca, sa, 0]]);
      points.push([[0, 0, top], [0, 0, 1]], [[...pa, top], [0, 0, 1]], [[...pb, top], [0, 0, 1]]);
    }
    return points;
  }

  class Toolpath {
    // `index` is what `toolpath` answered.
    constructor(renderer, index) {
      this.renderer = renderer;
      this.index = index;
      this.total = index.moves;
      this.loaded = 0;
      this.layers = index.layers;
      // The moves as they were sent, kept to read a move's own numbers
      // from and to work the ranges out of.
      this.records = new Uint8Array(this.total * MOVE);
      this.view = new DataView(this.records.buffer);
      this.ranges = new Map();
      const gl = renderer.gl;

      this.vao = gl.createVertexArray();
      gl.bindVertexArray(this.vao);
      // libvgcode SegmentTemplate.cpp: eight vertices, eight triangles.
      this.corners = renderer.staticBuffer(new Float32Array([0, 1, 2, 3, 4, 5, 6, 7]));
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 1, gl.FLOAT, false, 0, 0);
      this.elements = gl.createBuffer();
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.elements);
      gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, new Uint16Array([0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 5, 0, 5, 6, 0, 6, 1, 5, 4, 7, 5, 7, 6]), gl.STATIC_DRAW);
      this.buffer = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
      gl.bufferData(gl.ARRAY_BUFFER, Math.max(this.total * MOVE, MOVE), gl.STATIC_DRAW);
      renderer.bytes += this.total * MOVE;
      for (let location = 1; location <= 8; location += 1) {
        gl.enableVertexAttribArray(location);
        gl.vertexAttribDivisor(location, 1);
      }
      gl.bindVertexArray(null);
      this.bound = -1;

      // The marks: where each is, the height and width of the line it is
      // on, and what it is; and which move each one is.
      this.marks = [];
      this.markMoves = [];
      this.markVao = gl.createVertexArray();
      gl.bindVertexArray(this.markVao);
      gl.bindBuffer(gl.ARRAY_BUFFER, renderer.staticBuffer(diamond()));
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 24, 0);
      gl.enableVertexAttribArray(1);
      gl.vertexAttribPointer(1, 3, gl.FLOAT, false, 24, 12);
      this.markBuffer = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, this.markBuffer);
      for (let location = 2; location <= 4; location += 1) {
        gl.enableVertexAttribArray(location);
        gl.vertexAttribDivisor(location, 1);
      }
      gl.bindVertexArray(null);
      this.marksSent = 0;
      this.markBound = -1;

      this.palette = renderer.palette(3);
      this.paletteOf = '';
    }

    // Takes a stretch of moves as `toolpath_moves` answered it.
    append(bytes) {
      const header = new DataView(bytes, 0, 24);
      const magic = String.fromCharCode(...new Uint8Array(bytes, 0, 4));
      if (magic !== 'EDGT' || header.getUint32(4, true) !== 1 || header.getUint32(20, true) !== MOVE) {
        throw new Error('those are not moves');
      }
      const [from, count] = [header.getUint32(8, true), header.getUint32(12, true)];
      if (from !== this.loaded || from + count > this.total) throw new Error('the moves came out of turn');
      const moves = new Uint8Array(bytes, 24, count * MOVE);
      this.records.set(moves, from * MOVE);
      const gl = this.renderer.gl;
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
      gl.bufferSubData(gl.ARRAY_BUFFER, from * MOVE, moves);
      for (let at = from; at < from + count; at += 1) {
        const kind = this.records[at * MOVE + 28];
        if (kind < KIND.retract) continue;
        const view = this.view;
        this.marks.push(
          view.getFloat32(at * MOVE, true), view.getFloat32(at * MOVE + 4, true), view.getFloat32(at * MOVE + 8, true),
          view.getUint16(at * MOVE + 16, true) / 1000, view.getUint16(at * MOVE + 18, true) / 1000, kind,
        );
        this.markMoves.push(at);
      }
      this.loaded = from + count;
      this.ranges.clear();
      return count;
    }

    get complete() { return this.loaded >= this.total; }

    // The first move of a layer, and the one past its last.
    layerStart(layer) { return this.layers.first[Math.max(0, Math.min(layer, this.layers.first.length - 1))]; }
    layerEnd(layer) { return layer + 1 < this.layers.first.length ? this.layers.first[layer + 1] : this.total; }

    // A move's own numbers.
    move(at) {
      const [view, from] = [this.view, at * MOVE];
      const height = view.getUint16(from + 16, true) / 1000;
      const width = view.getUint16(from + 18, true) / 1000;
      const feedrate = view.getFloat32(from + 12, true);
      return {
        at: [view.getFloat32(from, true), view.getFloat32(from + 4, true), view.getFloat32(from + 8, true)],
        feedrate,
        height,
        width,
        temperature: view.getUint16(from + 22, true),
        layer: view.getUint32(from + 24, true),
        kind: this.records[from + 28],
        feature: this.records[from + 29],
        filament: this.records[from + 30],
        fan: this.records[from + 31],
        flow: feedrate * (height * (width - height) + 0.7853982 * height * height),
      };
    }

    // The least and the greatest value of a view over the lines whose
    // feature is shown, and how many values there are, counted the
    // reference's way (ViewerImpl.cpp `update_color_ranges`,
    // ColorRange.cpp `update`): [least, greatest, count].
    range(view, hiddenFeatures, travels, wipes) {
      const key = `${view}|${[...hiddenFeatures].sort()}|${travels}|${wipes}|${this.loaded}`;
      if (this.ranges.has(key)) return this.ranges.get(key);
      let [least, most, count] = [Infinity, -Infinity, 0];
      const add = (value) => {
        if (value !== least && value !== most) count += 1;
        if (value < least) least = value;
        if (value > most) most = value;
      };
      const records = this.records;
      for (let at = 0; at < this.loaded; at += 1) {
        const from = at * MOVE;
        const kind = records[from + 28];
        const feature = records[from + 29];
        const line = kind === KIND.line && !hiddenFeatures.has(feature);
        if (view === 2) {
          if (line || (kind === KIND.travel && travels) || (kind === KIND.wipe && wipes)) add(this.view.getFloat32(from + 12, true));
        } else if (line) {
          const height = this.view.getUint16(from + 16, true) / 1000;
          const width = this.view.getUint16(from + 18, true) / 1000;
          // The file's own lines, whose height is a guess, do not set
          // the range of heights or of widths.
          if (view === 3 && feature !== 14) add(roundToBin(height));
          else if (view === 4 && feature !== 14) add(roundToBin(width));
          else if (view === 5) add(roundToBin(this.view.getFloat32(from + 12, true) * (height * (width - height) + 0.7853982 * height * height)));
          else if (view === 6) add(roundToBin(records[from + 31]));
          else if (view === 7) add(roundToBin(this.view.getUint16(from + 22, true)));
        }
      }
      const range = count ? [least, most, count] : [0, 0, 0];
      this.ranges.set(key, range);
      return range;
    }

    // The values a range's legend lists, least first (ColorRange.cpp `get_values`).
    static steps(range) {
      const [least, most, count] = range;
      if (count === 0) return [];
      if (count === 1) return [least];
      if (count === 2) return [least, most];
      return RANGE.map((_, at) => least + (at * (most - least)) / (RANGE.length - 1));
    }

    // The colour of a value in a range, as the shader works it out.
    static rangeColour(value, range) {
      const [least, most, count] = range;
      if (!count) return RANGE[0];
      const step = (most - least) / (RANGE.length - 1);
      const t = step > 0 ? (Math.max(least, Math.min(most, value)) - least) / step : 0;
      const low = Math.max(0, Math.min(RANGE.length - 1, Math.floor(t)));
      const high = Math.min(low + 1, RANGE.length - 1);
      return RANGE[low].map((part, at) => Math.floor((1 - (t - low)) * part + (t - low) * RANGE[high][at]));
    }

    setColours(filaments) {
      const key = JSON.stringify(filaments);
      if (key === this.paletteOf) return;
      this.paletteOf = key;
      const data = new Uint8Array(256 * 3 * 4);
      FEATURES.forEach(([, colour], at) => data.set([...colour, 255], at * 4));
      for (let filament = 1; filament < 256; filament += 1) {
        // libvgcode Types.hpp DUMMY_COLOR for a filament no colour is known for.
        data.set([...(filaments[filament] || [64, 64, 64]), 255], (256 + filament) * 4);
      }
      RANGE.forEach((colour, at) => data.set([...colour, 255], (512 + at) * 4));
      this.renderer.setPalette(this.palette, data);
    }

    // Draws the moves from `from` up to but not `to`. `origin` is where
    // the plate stands; `show` says what is shown and how it is coloured.
    draw(frame, origin, from, to, show) {
      const r = this.renderer;
      const gl = r.gl;
      to = Math.min(to, this.loaded);
      // A move goes from the one before it, so the first has no line.
      const first = Math.max(from, 1);
      const count = to - first;
      if (count <= 0) return;
      const u = r.use('segments');
      gl.uniformMatrix4fv(u.u_view, false, frame.view);
      gl.uniformMatrix4fv(u.u_projection, false, frame.projection);
      gl.uniform3fv(u.u_camera, frame.eye);
      gl.uniform3fv(u.u_origin, origin);
      gl.uniform1i(u.u_view_type, show.view);
      gl.uniform3fv(u.u_range, show.range);
      let features = 0;
      for (let feature = 0; feature < FEATURES.length; feature += 1) if (!show.hiddenFeatures.has(feature)) features |= 1 << feature;
      gl.uniform1i(u.u_features, features);
      let hidden = 0;
      for (const filament of show.hiddenFilaments) if (filament >= 1 && filament <= 32) hidden |= 1 << (filament - 1);
      gl.uniform1i(u.u_hidden_filaments, hidden);
      gl.uniform1i(u.u_travels, show.options.has('Travel') ? 1 : 0);
      gl.uniform1i(u.u_wipes, show.options.has('Wipe') ? 1 : 0);
      gl.uniform3fv(u.u_travel_color, OPTIONS[0].colour);
      gl.uniform3fv(u.u_wipe_color, OPTIONS[1].colour);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, this.palette.texture);
      gl.uniform1i(u.u_palette, 0);

      gl.bindVertexArray(this.vao);
      if (this.bound !== first) {
        // Each instance reads the move before it and itself: the same
        // buffer, bound a move apart.
        gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
        const [before, here] = [(first - 1) * MOVE, first * MOVE];
        gl.vertexAttribPointer(1, 3, gl.FLOAT, false, MOVE, before);
        gl.vertexAttribPointer(2, 1, gl.SHORT, false, MOVE, before + 20);
        gl.vertexAttribPointer(3, 3, gl.FLOAT, false, MOVE, here);
        gl.vertexAttribPointer(4, 1, gl.FLOAT, false, MOVE, here + 12);
        gl.vertexAttribPointer(5, 2, gl.UNSIGNED_SHORT, false, MOVE, here + 16);
        gl.vertexAttribPointer(6, 1, gl.SHORT, false, MOVE, here + 20);
        gl.vertexAttribPointer(7, 1, gl.UNSIGNED_SHORT, false, MOVE, here + 22);
        gl.vertexAttribPointer(8, 4, gl.UNSIGNED_BYTE, false, MOVE, here + 28);
        this.bound = first;
      }
      gl.drawElementsInstanced(gl.TRIANGLES, 24, gl.UNSIGNED_SHORT, 0, count);
      this.drawMarks(frame, origin, from, to, show);
    }

    drawMarks(frame, origin, from, to, show) {
      let kinds = 0;
      for (const option of OPTIONS) if (option.kind >= KIND.retract && show.options.has(option.name)) kinds |= 1 << option.kind;
      if (!kinds || !this.markMoves.length) return;
      const r = this.renderer;
      const gl = r.gl;
      const moves = this.markMoves;
      const search = (move) => {
        let [low, high] = [0, moves.length];
        while (low < high) {
          const middle = (low + high) >> 1;
          if (moves[middle] < move) low = middle + 1; else high = middle;
        }
        return low;
      };
      // A mark where the nozzle now is shows too (ViewerImpl.cpp
      // `update_enabled_entities`: the range reaches one move on).
      const [first, last] = [search(from), search(to + 1)];
      if (last <= first) return;
      gl.bindVertexArray(this.markVao);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.markBuffer);
      if (this.marksSent !== moves.length) {
        gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(this.marks), gl.STATIC_DRAW);
        this.marksSent = moves.length;
        this.markBound = -1;
      }
      if (this.markBound !== first) {
        gl.vertexAttribPointer(2, 3, gl.FLOAT, false, 24, first * 24);
        gl.vertexAttribPointer(3, 2, gl.FLOAT, false, 24, first * 24 + 12);
        gl.vertexAttribPointer(4, 1, gl.FLOAT, false, 24, first * 24 + 20);
        this.markBound = first;
      }
      const u = r.use('options');
      gl.uniformMatrix4fv(u.u_view, false, frame.view);
      gl.uniformMatrix4fv(u.u_projection, false, frame.projection);
      gl.uniform3fv(u.u_origin, origin);
      // libvgcode Shaders.hpp: 0.75 on a Mac, 1.5 elsewhere.
      gl.uniform1f(u.u_scaling, /Mac|iPhone|iPad/.test(navigator.platform || '') ? 0.75 : 1.5);
      gl.uniform1i(u.u_kinds, kinds);
      const colours = new Float32Array(24);
      for (const option of OPTIONS) colours.set(option.colour.map((part) => part / 255), option.kind * 3);
      gl.uniform3fv(u.u_colors, colours);
      gl.enable(gl.CULL_FACE);
      gl.drawArraysInstanced(gl.TRIANGLES, 0, 96, last - first);
      gl.disable(gl.CULL_FACE);
    }

    dispose() {
      const gl = this.renderer.gl;
      gl.deleteBuffer(this.buffer);
      gl.deleteBuffer(this.elements);
      gl.deleteBuffer(this.markBuffer);
      gl.deleteVertexArray(this.vao);
      gl.deleteVertexArray(this.markVao);
      gl.deleteTexture(this.palette.texture);
      this.renderer.bytes -= this.total * MOVE;
    }
  }

  Toolpath.FEATURES = FEATURES;
  Toolpath.OPTIONS = OPTIONS;
  Toolpath.VIEWS = VIEWS;
  Toolpath.KIND = KIND;
  Toolpath.arrow = arrow;
  Toolpath.moveSize = MOVE;
  GL.Toolpath = Toolpath;
  void M;
})(window.EdgeGL);
