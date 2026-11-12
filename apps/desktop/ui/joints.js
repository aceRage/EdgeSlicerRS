'use strict';

// "Cut"'s connectors of the fork's own kinds beside the pin
// (Gizmos/GLGizmoCut.cpp): the Flexi joint (`render_flexi_joint_inputs`,
// `sync_flexi_params`), six print-in-place joints that keep both halves
// parts of one object a gap apart, and the TAC Joint
// (`render_tac_joint_inputs`, `sync_tac_joint_params`), a connector of a
// fixed shape with its socket, printed on its own or with one half. The
// shapes and the checks are the session's (`cut_flexi`, `cut_tac`;
// `edge_engine::flexi`, `edge_engine::tac`). Added to `GL.cutter`
// (cut.js, connectors.js).
(function (GL) {
  const cutter = GL.cutter;

  const fixed = (value, digits = 2) => (Math.round(value * 10 ** digits) / 10 ** digits).toFixed(digits);

  Object.assign(cutter, {
    // The joint new Flexi joints take and what the session says of it; the
    // panel's own ticks (`m_flexi_auto_size`, `m_flexi_hinge_auto_edge`).
    flexi: null,
    flexiInfo: null,
    flexiUi: { auto: true, autoEdge: true },
    // The TAC Joint new ones take, and what the session says of it.
    tac: null,
    tacInfo: null,

    // The section's inscribed radius (`flexi_section_inscribed_radius`).
    inscribed() {
      const reach = this.open && this.open.section && this.open.section.reach;
      return reach ? 0.5 * Math.min(reach[1][0] - reach[0][0], reach[1][1] - reach[0][1]) : 0;
    },

    nozzleNow() {
      const settled = this.host.settled();
      const nozzles = settled && settled.printer && settled.printer.nozzle_diameter;
      return Number(Array.isArray(nozzles) ? nozzles[0] : String(nozzles || '').split(/[;,]/)[0]) || 0.4;
    },

    // The slice's gap closing radius (`flexi_slice_closing_radius`).
    closingRadius() {
      const settled = this.host.settled();
      const value = settled && settled.process && Number(settled.process.slice_closing_radius);
      return Number.isFinite(value) ? value : 0.049;
    },

    // ---- the Flexi joint ----

    // Asks the session about the joint as it is (sized from the section
    // with "Auto size"), takes it back and gives it to the chosen ones.
    async askFlexi(resize = false) {
      const o = this.open;
      let info;
      try {
        info = await this.host.call('cut_flexi', {
          flexi: this.flexi || {}, auto: resize && this.flexiUi.auto, inscribed: this.inscribed(), nozzle: this.nozzleNow(),
          closing: this.closingRadius(), normal: o ? o.axes[2] : [0, 0, 1], x: o ? o.axes[0] : [1, 0, 0], y: o ? o.axes[1] : [0, 1, 0],
        });
      } catch (error) {
        this.host.say(error.message, true);
        return;
      }
      this.flexi = info.flexi;
      // "Auto edge placement" (`flexi_hinge_auto_edge_offset`): the barrel
      // at the section's edge across the hinge's axis.
      if (this.flexi.kind === 3 && this.flexiUi.autoEdge && o && o.section && o.section.reach) {
        const reach = o.section.reach;
        const a = (this.flexi.rotation * Math.PI) / 180;
        const [sx, sy] = [reach[1][0] - reach[0][0], reach[1][1] - reach[0][1]];
        let far = Math.abs(Math.sin(a)) * 0.5 * sx + Math.abs(Math.cos(a)) * 0.5 * sy;
        const rIn = this.inscribed();
        const hl = 0.5 * this.flexi.hinge_length;
        if (rIn > 0 && hl < rIn) far = Math.min(far, Math.sqrt(rIn * rIn - hl * hl));
        const offset = Math.max(0, far - 0.5 * this.flexi.hinge_barrel_dia - 0.1);
        if (Math.abs(offset - this.flexi.hinge_edge_offset) > 1e-6) {
          this.flexi.hinge_edge_offset = offset;
          info = await this.host.call('cut_flexi', { flexi: this.flexi, nozzle: this.nozzleNow(), closing: this.closingRadius(), normal: o.axes[2], x: o.axes[0], y: o.axes[1] });
          this.flexi = info.flexi;
        }
      }
      this.flexiInfo = info;
      // Every Flexi joint takes it (`sync_flexi_params`).
      if (this.open) for (const c of this.open.connectors) if (c.type === 'flexi') this.jointOnto(c);
      this.connectorsChanged();
    },

    setFlexi(changes, resize = false) {
      this.flexi = { ...(this.flexi || {}), ...changes };
      this.askFlexi(resize);
    },

    // ---- the TAC Joint ----

    async askTac() {
      let info;
      try {
        info = await this.host.call('cut_tac', { tac: this.tac || {}, faces: this.faces() });
      } catch (error) {
        this.host.say(error.message, true);
        return;
      }
      this.tac = info.tac;
      this.tacInfo = info;
      if (this.open) for (const at of this.open.selected) {
        const c = this.open.connectors[at];
        if (c && c.type === 'tac') this.jointOnto(c);
      }
      this.connectorsChanged();
    },

    setTac(changes) {
      this.tac = { ...(this.tac || {}), ...changes };
      this.askTac();
    },

    // A connector made a Flexi joint or a TAC Joint as they now are: its
    // size as the outline's (`sync_flexi_params`, `sync_tac_joint_params`).
    jointOnto(c) {
      c.style = 'prism';
      c.shape = 'circle';
      c.radius_tolerance = 0;
      c.height_tolerance = 0;
      if (c.type === 'flexi' && this.flexiInfo) {
        c.flexi = { ...this.flexi };
        c.radius = this.flexiInfo.radius;
        c.height = this.flexiInfo.height;
        c.outline = this.flexiInfo.outline;
        c.jointInvalid = this.flexiInfo.invalid;
      } else if (c.type === 'tac' && this.tacInfo) {
        c.tac = { ...this.tac };
        const s = this.tac.scale;
        c.radius = 0.5 * Math.hypot(4.09 * s + 2 * this.tac.clearance, 3.34 * s + 2 * this.tac.clearance);
        c.height = 11 * s;
        c.outline = this.tacInfo.outline;
        c.jointInvalid = this.tacInfo.invalid;
      }
    },

    // Whether a joint's settings are known yet; asked for when not.
    jointReady(type) {
      if (type === 'flexi' && !this.flexiInfo) { this.askFlexi(true); return false; }
      if (type === 'tac' && !this.tacInfo) { this.askTac(); return false; }
      return true;
    },

    // How far a joint reaches each way along the normal, from the plane.
    jointReach(c) {
      if (c.type === 'tac' && c.tac) {
        const half = 5.5 * c.tac.scale;
        return [c.tac.offset - half, c.tac.offset + half];
      }
      return [-c.height, c.height];
    },

    // ---- the panel ----

    flexiRows(busy, row) {
      const { el } = this.host;
      const rows = [];
      if (!this.jointReady('flexi')) return rows;
      const p = this.flexi;
      const info = this.flexiInfo;
      const mm = t('mm');
      const number = (label, key, min, max, tip, { off = false, digits = 2, step = 0.01, unit = mm } = {}) => {
        const input = el('input', { type: 'number', min: String(min), max: String(max), step: String(step), value: fixed(p[key], digits), title: tip || '', disabled: busy || off });
        input.setAttribute('aria-label', label);
        input.dataset.field = `flexi-${key}`;
        input.addEventListener('change', () => {
          const v = Number(input.value);
          if (!Number.isFinite(v)) { this.panel(); return; }
          this.setFlexi({ [key]: Math.min(max, Math.max(min, v)) });
        });
        return row(label, el('span', { className: 'unit' }, input, unit ? ` ${unit}` : ''));
      };
      const whole = (label, key, min, max, tip, now) => {
        const input = el('input', { type: 'number', min: String(min), max: String(max), step: '1', value: String(now), title: tip || '', disabled: busy });
        input.setAttribute('aria-label', label);
        input.addEventListener('change', () => {
          const v = Math.round(Number(input.value));
          if (Number.isFinite(v)) this.setFlexi({ [key]: Math.min(max, Math.max(min, v)) }); else this.panel();
        });
        return row(label, el('span', { className: 'unit cut-count' }, input));
      };
      const tick = (label, on, act, tip) => {
        const input = el('input', { type: 'checkbox', checked: Boolean(on), disabled: busy });
        input.addEventListener('change', () => act(input.checked));
        return el('label', { className: 'edit-check', title: tip || '' }, input, ` ${label}`);
      };
      const rotation = (tip) => number(t('Rotation'), 'rotation', 0, 180, tip, { digits: 0, step: 1, unit: '°' });
      // "Joint".
      const kind = el('select', { disabled: busy });
      kind.setAttribute('aria-label', t('Joint'));
      [t('Double ring'), t('Ball & socket'), t('Chain link'), t('Hinge'), t('Thread'), t('Bayonet')].forEach((word, at) => kind.append(new Option(word, String(at), false, p.kind === at)));
      kind.addEventListener('change', () => this.setFlexi({ kind: Number(kind.value), gap: 0 }, true));
      rows.push(row(t('Joint'), kind));
      rows.push(tick(t('Auto size from the cut cross-section'), this.flexiUi.auto, (on) => { this.flexiUi.auto = on; if (on) this.askFlexi(true); else this.panel(); }));
      const auto = this.flexiUi.auto;
      const twist = p.kind === 4 || p.kind === 5;
      if (twist) {
        rows.push(number(t('Major diameter'), 'thread_major_dia', 4, 200, t("Outside diameter of the thread, or of the bayonet's lug circle. Auto = 0.7 x the inscribed radius of the cut cross-section."), { off: auto }));
        if (p.kind === 4) {
          rows.push(number(t('Pitch'), 'thread_pitch', 1, 20, t('How far the thread rises per turn. A coarse pitch prints far better than a fine one: 2 mm is the floor on a 0.4 mm nozzle, 3 to 4 mm is the comfortable range.'), { off: auto }));
          rows.push(whole(t('Starts'), 'thread_starts', 1, 4, t('How many thread strands run side by side. An N-start thread does up in 360/N degrees of turn: 2 starts gives a half-turn lid, 4 a quarter-turn one.'), info.thread_starts));
          rows.push(number(t('Turns'), 'thread_turns', 0.25, 10, t('How many full revolutions of thread there are. 1.25 is the usual jar and pill-bottle range: enough to seat without winding it on forever.'), { unit: '' }));
          rows.push(number(t('Lead-in turns'), 'thread_lead_turns', 0, 2, t("How much of a turn, at each end, the thread fades to nothing over. This is the lead-in chamfer: it is what lets the two halves catch at any relative angle instead of having to be lined up."), { unit: '' }));
          rows.push(tick(t('Left-hand thread'), p.thread_left_hand, (on) => this.setFlexi({ thread_left_hand: on })));
        } else {
          rows.push(whole(t('Lugs'), 'bayonet_lugs', 2, 4, t("How many lugs ride in the bayonet's slots, evenly spaced around the plug. Three is the usual compromise between holding power and having wall left between the slots."), info.bayonet_lugs));
          rows.push(number(t('Lug height'), 'bayonet_lug_height', 0.4, 10, t('How far each lug stands out of the plug wall.'), { off: auto }));
          rows.push(number(t('Lug thickness'), 'bayonet_lug_thickness', 0.4, 10, t('How tall each lug is along the axis. This is what carries the pull-out load, so it wants at least a few layers.'), { off: auto }));
          rows.push(number(t('Lug width'), 'bayonet_lug_arc', 5, 90, t('Angular width of one lug, in degrees. It has to share its slice of the bore with the entry channel and the track.'), { unit: '°' }));
          rows.push(number(t('Lock angle'), 'bayonet_lock_angle', 20, 120, t("How far the lid turns to lock, in degrees. 60 to 90 is the usual quarter-turn range; it is clamped so the track cannot run into the next lug's channel."), { unit: '°' }));
          rows.push(number(t('Entry depth'), 'bayonet_entry_depth', 0.5, 40, t("How far the lid pushes straight in before it can be turned. It has to be at least the lug's own thickness.")));
          rows.push(number(t('Detent'), 'bayonet_detent', 0, 3, t('Height of the bump at the end of the track that the lug clicks over, so the lid stops where it locks instead of turning back. 0 leaves the track plain.')));
          rows.push(tick(t('Turn the other way to lock'), p.thread_left_hand, (on) => this.setFlexi({ thread_left_hand: on })));
        }
        rows.push(tick(t('Lid side: the upper half screws on'), p.thread_lid_upper, (on) => this.setFlexi({ thread_lid_upper: on }), t('Which half carries the male thread or the lugged plug - i.e. which one is the cap. The other half gets the bore.')));
        rows.push(rotation(p.kind === 4
          ? t("The thread's start angle: where the first strand begins around the axis. Turning it turns where the lid ends up pointing when it is screwed down.")
          : t('Where the first lug sits around the axis. Turning it turns where the lid ends up pointing when it is locked.')));
      } else if (p.kind === 3) {
        rows.push(whole(t('Knuckles'), 'hinge_knuckles', 1, 9, t('How many knuckles the hinge is split into, alternating between the two halves. An odd count centres the hinge on the point you clicked; an even one shifts it by half a knuckle.'), info.hinge_knuckles));
        rows.push(number(t('Pin diameter'), 'hinge_pin_dia', 0.4, 20, t('Diameter of the pin that runs through every knuckle. The pin is one continuous solid, integral to whichever half the Fold side names.'), { off: auto }));
        rows.push(number(t('Barrel diameter'), 'hinge_barrel_dia', 1, 40, t('Outer diameter of each knuckle. It has to clear the pin, the clearance on both sides and a wall on each side.'), { off: auto }));
        rows.push(number(t('Hinge length'), 'hinge_length', 2, 200, t('Overall length of the knuckle run along the hinge axis. Each knuckle gets an equal share of it, less the gap at each end face.'), { off: auto }));
        rows.push(tick(t('Auto edge placement'), this.flexiUi.autoEdge, (on) => { this.flexiUi.autoEdge = on; if (on) this.askFlexi(); else this.panel(); }));
        rows.push(number(t('Edge offset'), 'hinge_edge_offset', 0, 200, t("How far the barrel sits out from the cut plane's centre, towards the edge of the cut face. The halves can only fold shut when the barrel is at (or past) that edge."), { off: this.flexiUi.autoEdge }));
        rows.push(tick(t('Fold side: the upper half carries the pin'), p.hinge_fold_upper, (on) => this.setFlexi({ hinge_fold_upper: on })));
        rows.push(rotation(t('Turns the hinge about the cut normal. This angle IS the pin axis: it decides which way the part folds, and whether the pin ends up lying flat on the bed (easy to print) or standing up (needs care).')));
      } else if (p.kind === 2) {
        rows.push(number(t('Link length'), 'link_length', 1, 80, t("Overall length of each loop, along the loop's long axis."), { off: auto }));
        rows.push(number(t('Link width'), 'link_width', 1, 80, t("Overall width of each loop, across the loop. The loop's opening has to pass the other loop's wire."), { off: auto }));
        rows.push(number(t('Wire thickness'), 'wire', 0.2, 10, t('Tube radius of the wire each loop is made of.'), { off: auto }));
        rows.push(number(t('Stem depth'), 'stem', 0.2, 20, t("How deep each loop's far end is embedded in its own segment."), { off: auto }));
        rows.push(number(t('Loop tilt'), 'tilt_angle', 0, 30, t('How far the upper ring is tipped inside its own plane, in degrees, so its free end leans clear of the lower ring.'), { unit: '°' }));
        rows.push(rotation(t('Turns the whole link about the cut normal. Both rings turn together, so their planes stay perpendicular to each other; this only chooses where in the cut plane the pair sits - use it to line the rings up with the part, or with the way it prints.')));
      } else {
        rows.push(number(p.kind === 0 ? t('Outer radius') : t('Ball radius'), 'outer_radius', 0.6, 60, t('Outer radius of the joint. Auto = 0.4 x the inscribed radius of the cut cross-section.'), { off: auto }));
        if (p.kind === 0) {
          rows.push(number(t('Ring width'), 'ring_width', 0.4, 20, t('Radial thickness of the ring lip.')));
          rows.push(number(t('Ring height'), 'ring_height', 0.4, 20, t('How far the ring lip reaches into the groove.')));
        } else {
          rows.push(number(t('Opening angle'), 'open_angle', 10, 75, t('Half angle of the socket mouth, in degrees. Smaller keeps the ball captive, larger gives more movement.'), { unit: '°' }));
        }
      }
      rows.push(number(t('Clearance'), 'clearance', info.floor, 2, t('Space left between the joint\'s moving surfaces so they do not fuse. The floor comes from the nozzle diameter of the active printer preset.')));
      rows.push(number(t('Gap'), 'gap', p.clearance, 20, t("Thickness of the cut: how far apart the two segments' faces end up. Each face is set back from the cut plane by half of it, and the joint bridges the gap. A larger gap makes the joint visibly more flexible; it can never be smaller than the clearance.")));
      if (p.kind <= 1) rows.push(number(t('Tilt allowance'), 'tilt', 0, 3, t('Extra headroom carved into the groove so the segment can rock.')));
      if (p.kind === 0) rows.push(number(t('Hub radius'), 'hub_radius', 0, 60, t('Radius of the solid central hub that keeps the joint from collapsing. 0 disables it.'), { off: auto }));
      // Why it cannot be made (`flexi_validate`, untranslated there too),
      // and the panel's warnings.
      if (info.invalid) rows.push(el('p', { className: 'edit-hint cut-note', textContent: info.invalid }));
      for (const warning of info.warnings || []) rows.push(el('p', { className: 'edit-hint cut-note', textContent: warning }));
      rows.push(el('p', { className: 'edit-hint', textContent: p.kind === 4
        ? t('The lid screws on. The two halves print in place as one object; unscrew them after printing.')
        : p.kind === 5
          ? t('The lid pushes in and turns to lock. The two halves print in place as one object.')
          : t('Both halves stay parts of one object so the joint prints in place.') }));
      return rows;
    },

    tacRows(busy, row) {
      const { el } = this.host;
      const rows = [];
      if (!this.jointReady('tac')) return rows;
      const p = this.tac;
      const info = this.tacInfo;
      const mm = t('mm');
      const number = (label, value, min, max, tip, act, { digits = 3, step = 0.01, unit = mm } = {}) => {
        const input = el('input', { type: 'number', min: String(min), max: String(max), step: String(step), value: fixed(value, digits), title: tip || '', disabled: busy });
        input.setAttribute('aria-label', label);
        input.addEventListener('change', () => {
          const v = Number(input.value);
          if (Number.isFinite(v)) act(Math.min(max, Math.max(min, v))); else this.panel();
        });
        return row(label, el('span', { className: 'unit' }, input, unit ? ` ${unit}` : ''));
      };
      // "Placement".
      const place = el('select', { disabled: busy });
      place.setAttribute('aria-label', t('Placement'));
      [t('On build plate'), t('Attached to A'), t('Attached to B')].forEach((word, at) => place.append(new Option(word, String(at), false, p.attachment === at)));
      place.addEventListener('change', () => this.setTac({ attachment: Number(place.value) }));
      rows.push(row(t('Placement'), place));
      rows.push(number(t('Size (%)'), 100 * p.scale, 10, 1000, t('Scale the connector. Clearances remain in millimetres.'), (v) => this.setTac({ scale: v / 100 }), { digits: 1, step: 0.5, unit: '%' }));
      const [sx, sy, sz] = info.size;
      rows.push(el('p', { className: 'edit-hint', textContent: t('Connector: %1$.2f x %2$.2f x %3$.2f mm').replace('%1$.2f', fixed(sx)).replace('%2$.2f', fixed(sy)).replace('%3$.2f', fixed(sz)) }));
      rows.push(number(t('Side clearance'), p.clearance, 0, 5, t('Nominal clearance per side. Adjusts the cutter dimensions; it is not a constant surface offset around every curve. The 0.12 mm default preserves the original cutter.'), (v) => this.setTac({ clearance: v })));
      rows.push(number(t('End clearance'), p.end_clearance, 0, 5, t('Clearance per end. The 0.30 mm default preserves the original cutter.'), (v) => this.setTac({ end_clearance: v })));
      const half = 5.5 * p.scale;
      rows.push(number(t('Cut offset'), p.offset, -half, half, t('Move the connector along the cut normal. Both halves must retain a positive insertion depth.'), (v) => this.setTac({ offset: v })));
      const [a, b] = info.insertion;
      rows.push(el('p', { className: 'edit-hint', textContent: t('Insertion A: %1$.2f mm; B: %2$.2f mm').replace('%1$.2f', fixed(a)).replace('%2$.2f', fixed(b)) }));
      // "Rotation", as every connector has, here to 360 degrees.
      const s = this.connector;
      rows.push(number(t('Rotation'), (s.angle * 180) / Math.PI, 0, 360, t('Rotate the asymmetric profile about the cut normal.'), (v) => this.setConnector('angle', (v * Math.PI) / 180), { digits: 1, step: 0.5, unit: '°' }));
      const reset = el('button', { type: 'button', textContent: t('Reset TAC Joint'), disabled: busy });
      reset.addEventListener('click', () => { this.tac = null; this.setConnector('angle', 0); this.askTac(); });
      rows.push(el('div', { className: 'cut-foot cut-start' }, reset));
      if (info.invalid) rows.push(el('p', { className: 'edit-hint cut-note', textContent: t(info.invalid) }));
      rows.push(el('p', { className: 'edit-hint', textContent: p.attachment === 0
        ? t('The connector is printed as a separate object. The blue outline shows its matching cutter.')
        : t('The connector belongs to the selected half. Only the opposite half gets a socket. Use Mesh Boolean after cutting to merge the connector into the body.') }));
      return rows;
    },
  });
})(window.EdgeGL);
