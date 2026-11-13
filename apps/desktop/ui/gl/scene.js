'use strict';

// What stands in the 3D scene: the bed, the plates of a project where the
// reference stands them, and the objects on them. It is drawn as the
// reference draws it (3DBed.cpp, PartPlate.cpp, 3DScene.cpp); the colours
// are the reference's, as its source has them.
(function (GL) {
  const M = GL.math;

  // GLCanvas3D.cpp DEFAULT_BG_LIGHT_COLOR, DEFAULT_BG_LIGHT_COLOR_DARK.
  const BACKGROUND = { light: [0.906, 0.906, 0.906], dark: [0.329, 0.329, 0.353] };
  // 3DBed.cpp DEFAULT_MODEL_COLOR, DEFAULT_MODEL_COLOR_DARK, GROUND_Z.
  const BED = { light: [0.3255, 0.337, 0.337, 1], dark: [0.255, 0.255, 0.283, 1] };
  const BED_GROUND = -0.04;
  // The model stands 0.41 under the ground so that nothing fights with
  // the plane on it (3DBed.cpp `update_model_offset`).
  const BED_MODEL_Z = -0.41 + BED_GROUND;
  // libslic3r/Color.hpp ColorRGB::X, Y, Z.
  const AXES = [[1, 60 / 255, 91 / 255, 1], [100 / 255, 200 / 255, 24 / 255, 1], [47 / 255, 136 / 255, 233 / 255, 1]];
  // PartPlate.cpp: UNSELECT_COLOR, LINE_TOP_COLOR, LINE_TOP_SEL_COLOR and
  // their dark ones, LINE_BOTTOM_COLOR; GROUND_Z, GROUND_Z_GRIDLINE.
  const PLATE = {
    fill: { light: [0.82, 0.82, 0.82, 1], dark: [0.384, 0.384, 0.412, 1] },
    line: { light: [0.89, 0.89, 0.89, 1], dark: [0.431, 0.431, 0.463, 1] },
    lineSelected: { light: [0.5294, 0.5451, 0.5333, 1], dark: [0.298, 0.298, 0.3333, 1] },
    lineBelow: [0.8, 0.8, 0.8, 0.4],
    ground: -0.03,
    gridGround: -0.26,
    // The plate's number and name (wxColour(0xf2, 0x75, 0x4e)).
    text: '#f2754e',
  };
  // PartPlate.cpp: PARTPLATE_ICON_SIZE, PARTPLATE_EDIT_PLATE_NAME_ICON_SIZE,
  // PARTPLATE_ICON_GAP_LEFT, PARTPLATE_TEXT_OFFSET_X2, PARTPLATE_TEXT_OFFSET_Y,
  // all for a bed 200 mm deep.
  const [ICON, NAME, GAP_LEFT, TEXT_X, TEXT_Y] = [16, 9, 3, 1, 1];
  // 3DScene.cpp: UNPRINTABLE_COLOR, MODEL_NEGTIVE_COL, MODEL_MIDIFIER_COL,
  // SUPPORT_BLOCKER_COL, SUPPORT_ENFORCER_COL; the marks are api/scene.rs's.
  const UNPRINTABLE = [0, 0, 0, 0.5];
  const MARKS = { 251: [0.3, 0.3, 0.3, 0.4], 252: [1, 1, 0, 0.6], 253: [1, 0.3, 0.3, 0.4], 254: [0.3, 0.3, 1, 0.4] };
  // The selection's outline and the corners of its box (GLCanvas3D.cpp
  // SELECTION_OUTLINE_COLOR, Selection.cpp).
  const SELECTED = [1, 1, 1];
  // A filament no colour is known for (GLVolume::MODEL_COLOR, in turn).
  const SPARE = ['#ffff00', '#ff8080', '#80ff80', '#8080ff'];

  // Preferences' theme, else the system's (i18n.js `EdgeTheme`).
  const dark = () => !EdgeTheme.light();
  const themed = (pair) => (dark() ? pair.dark : pair.light);

  function rgb(text) {
    const hex = /^#([0-9a-f]{6})/i.exec(text || '');
    if (!hex) return null;
    return [0, 2, 4].map((at) => parseInt(hex[1].slice(at, at + 2), 16) / 255);
  }

  // 3DScene.cpp `adjust_color_for_rendering`: black is lifted far enough
  // to be shaded.
  function forRendering(colour) {
    return colour.every((part) => part < 0.2) ? [0.2, 0.2, 0.2] : colour;
  }

  // The colour each filament is shown in, by number from 1.
  function filamentColour(colours, filament) {
    return forRendering(rgb((colours || [])[filament - 1]) || rgb(SPARE[(filament - 1) % SPARE.length]));
  }

  // What of a line from a to b is inside a convex polygon, or null.
  function clipped(a, b, polygon) {
    let [enter, leave] = [0, 1];
    const d = [b[0] - a[0], b[1] - a[1]];
    let area = 0;
    for (let i = 0; i < polygon.length; i += 1) {
      const [p, q] = [polygon[i], polygon[(i + 1) % polygon.length]];
      area += p[0] * q[1] - q[0] * p[1];
    }
    const turn = area < 0 ? -1 : 1;
    for (let i = 0; i < polygon.length; i += 1) {
      const [p, q] = [polygon[i], polygon[(i + 1) % polygon.length]];
      const normal = [(p[1] - q[1]) * turn, (q[0] - p[0]) * turn];
      const side = normal[0] * (a[0] - p[0]) + normal[1] * (a[1] - p[1]);
      const rate = normal[0] * d[0] + normal[1] * d[1];
      if (Math.abs(rate) < 1e-12) {
        if (side < -1e-9) return null;
      } else if (rate > 0) {
        enter = Math.max(enter, -side / rate);
      } else {
        leave = Math.min(leave, -side / rate);
      }
    }
    if (enter >= leave) return null;
    return [[a[0] + d[0] * enter, a[1] + d[1] * enter], [a[0] + d[0] * leave, a[1] + d[1] * leave]];
  }

  // A cylinder of 16 sides along +Z from the origin, as flat triangles
  // (3DBed.cpp `Bed3D::Axes::render`: smooth_cylinder(16, length / 75, length)).
  function cylinder(radius, length) {
    const out = [];
    for (let side = 0; side < 16; side += 1) {
      const [a, b] = [(side * Math.PI) / 8, ((side + 1) * Math.PI) / 8];
      const [ax, ay, bx, by] = [Math.cos(a) * radius, Math.sin(a) * radius, Math.cos(b) * radius, Math.sin(b) * radius];
      out.push(ax, ay, 0, bx, by, 0, bx, by, length, ax, ay, 0, bx, by, length, ax, ay, length);
      out.push(0, 0, length, ax, ay, length, bx, by, length, 0, 0, 0, bx, by, 0, ax, ay, 0);
    }
    return out;
  }

  // Text as a picture, in the plates' own colour, on nothing.
  function textPicture(text, weight, height) {
    const canvas = document.createElement('canvas');
    const context = canvas.getContext('2d');
    const font = `${weight} ${height}px system-ui, -apple-system, "Segoe UI", sans-serif`;
    context.font = font;
    canvas.width = Math.max(2, Math.ceil(context.measureText(text).width) + 4);
    canvas.height = Math.ceil(height * 1.25);
    context.font = font;
    context.fillStyle = PLATE.text;
    context.textBaseline = 'middle';
    context.fillText(text, 2, canvas.height / 2);
    return canvas;
  }

  class Scene {
    constructor(renderer) {
      this.renderer = renderer;
      this.data = null;
      // An object's mesh, by the scene's key and the object's index.
      this.meshes = new Map();
      this.plates = [];
      this.palette = renderer.palette(1);
      this.bedModel = null;
      this.bedPicture = null;
      this.axes = null;
      this.corners = null;
      this.cornersOf = '';
      // Parts of one object drawn elsewhere while a tool drags them: the
      // object's index, and for each part's index a matrix that takes the
      // object's coordinates to where the part now is in them.
      this.moving = null;
      // The copy a painting tool is open on, which the tool draws:
      // { object (its index), copy (its id) }. The object's other copies are
      // not drawn meanwhile (GLGizmosCommon.cpp `InstancesHider`), and of
      // the copy only what is not printed (modifiers and the rest).
      this.painting = null;
    }

    get background() { return themed(BACKGROUND); }

    // Takes a scene as `scene` answers it, and makes what is drawn of its
    // bed and plates. The objects' meshes come one by one (`addMesh`).
    set(data) {
      const r = this.renderer;
      for (const plate of this.plates) {
        for (const part of [plate.fill, plate.thin, plate.bold, plate.number, plate.name, plate.picture]) r.drop(part);
        for (const picture of [plate.numberPicture, plate.namePicture]) r.drop({ texture: picture });
      }
      r.drop(this.axes);
      this.axes = null;
      this.data = data;
      this.plates = [];
      if (!data) return;
      const bed = data.bed;
      // A project without plates, a bare model and an empty project are
      // shown on one bed.
      const listed = data.plates.length ? data.plates : [{ id: null, name: '', origin: [0, 0] }];
      this.plates = listed.map((plate, index) => this.plateParts(plate, index, bed));
      if (bed.box) {
        // 3DBed.cpp: the axes are a tenth of the build volume's greatest side.
        const size = Math.max(bed.box[2] - bed.box[0], bed.box[3] - bed.box[1], bed.height || 0);
        this.axes = r.triangles(cylinder((0.1 * size) / 75, 0.1 * size));
      }
      const colours = new Uint8Array(256 * 4);
      for (let mark = 0; mark < 256; mark += 1) {
        const colour = MARKS[mark] || [...filamentColour(data.colours, Math.max(mark, 1)), 1];
        colours.set(colour.map((part) => Math.round(part * 255)), mark * 4);
      }
      r.setPalette(this.palette, colours);
    }

    plateParts(plate, index, bed) {
      const r = this.renderer;
      const parts = { id: plate.id, index, origin: plate.origin, box: null };
      if (!bed.box) return parts;
      const [ox, oy] = plate.origin;
      const box = [bed.box[0] + ox, bed.box[1] + oy, bed.box[2] + ox, bed.box[3] + oy];
      parts.box = box;
      const shape = (bed.shape.length >= 3 ? bed.shape : [[bed.box[0], bed.box[1]], [bed.box[2], bed.box[1]], [bed.box[2], bed.box[3]], [bed.box[0], bed.box[3]]])
        .map(([x, y]) => [x + ox, y + oy]);
      parts.shape = shape;
      // The bed's shape as a fan of triangles: beds are rectangles and
      // circles. A bed with a dent in its outline would need more.
      const fan = [];
      for (let at = 1; at + 1 < shape.length; at += 1) fan.push(...shape[0], 0, ...shape[at], 0, ...shape[at + 1], 0);
      parts.fill = r.triangles(fan);

      // The grid: a line every 10 mm from the plate's origin, fewer on a
      // large bed, every fifth one bold; and the bed's outline
      // (2DBed.cpp `calculate_grid_step`, `generate_grid`,
      // PartPlate.cpp `calc_gridlines`).
      const short = Math.min(box[2] - box[0], box[3] - box[1]);
      const step = short >= 6000 ? 100 : short >= 1200 ? 50 : short >= 600 ? 20 : 10;
      const [thin, bold] = [[], []];
      const add = (a, b, count) => {
        const inside = clipped(a, b, shape);
        if (inside) (count % 5 ? thin : bold).push(...inside[0], 0, ...inside[1], 0);
      };
      // Out from the origin each way, as far as the bed goes; a line that
      // falls off the bed's shape is not drawn, but is counted.
      for (let count = 0, x = ox; x >= box[0]; x -= step, count += 1) add([x, box[1]], [x, box[3]], count);
      for (let count = 0, x = ox; x <= box[2]; x += step, count += 1) add([x, box[1]], [x, box[3]], count);
      for (let count = 0, y = oy; y >= box[1]; y -= step, count += 1) add([box[0], y], [box[2], y], count);
      for (let count = 0, y = oy; y <= box[3]; y += step, count += 1) add([box[0], y], [box[2], y], count);
      shape.forEach((point, at) => thin.push(...point, 0, ...shape[(at + 1) % shape.length], 0));
      parts.thin = r.lines(thin);
      parts.bold = r.lines(bold);

      // The plate's number at its bottom right and its name at its top
      // left, sized by the bed's depth (PartPlate.cpp
      // `calc_vertex_for_number`, `generate_plate_name_texture`).
      const factor = (box[3] - box[1]) / 200;
      if (plate.id !== null) {
        const [x, y, size] = [box[2] + GAP_LEFT * factor, box[1], ICON * factor];
        parts.numberPicture = r.picture(textPicture(String(plate.id).padStart(2, '0'), '700', 96));
        parts.number = r.patch([
          [x + TEXT_X * factor, y + TEXT_Y * factor, 0], [x + size - TEXT_X * factor, y + TEXT_Y * factor, 0],
          [x + size - TEXT_X * factor, y + size - TEXT_Y * factor, 0], [x + TEXT_X * factor, y + size - TEXT_Y * factor, 0],
        ]);
        const picture = textPicture(plate.name || t('Untitled'), '700', 64);
        const [high, y0] = [NAME * factor, box[3] + TEXT_Y * factor];
        const wide = (high * picture.width) / picture.height;
        parts.namePicture = r.picture(picture);
        parts.name = r.patch([[box[0], y0, 0], [box[0] + wide, y0, 0], [box[0] + wide, y0 + high, 0], [box[0], y0 + high, 0]]);
      }
      parts.picture = r.patch([[box[0], box[1], 0], [box[2], box[1], 0], [box[2], box[3], 0], [box[0], box[3], 0]]);
      return parts;
    }

    // The plate with this number; the only bed there is when there are no plates.
    plate(id) {
      return this.plates.find((plate) => plate.id === id) || this.plates[0] || null;
    }

    // An object's mesh is kept under the scene's key and the mesh's own
    // number (`scene`'s `objects[].mesh`), which stays the same whatever is
    // done to the object's copies: an edit sends no mesh again.
    meshKey(object) { return `${this.data.key}:${this.data.objects[object].mesh}`; }
    meshOf(object) { return this.data ? this.meshes.get(this.meshKey(object)) : null; }
    addMesh(key, mesh) { this.meshes.set(key, mesh); }

    // Lets go of the meshes of this scene that none of its objects has
    // any more: an object that was deleted, or given another filament.
    prune() {
      if (!this.data) return;
      const held = new Set(this.data.objects.map((object, index) => this.meshKey(index)));
      for (const [key, mesh] of this.meshes) {
        if (key.startsWith(`${this.data.key}:`) && !held.has(key)) {
          this.renderer.drop(mesh);
          this.meshes.delete(key);
        }
      }
    }

    // Lets go of the meshes of scenes that are no longer open.
    keep(keys) {
      for (const [key, mesh] of this.meshes) {
        if (!keys.some((kept) => key.startsWith(`${kept}:`))) {
          this.renderer.drop(mesh);
          this.meshes.delete(key);
        }
      }
    }

    setBedModel(mesh) {
      this.renderer.drop(this.bedModel);
      this.bedModel = mesh;
    }

    setBedPicture(texture) {
      if (this.bedPicture) this.renderer.drop({ texture: this.bedPicture });
      this.bedPicture = texture;
    }

    // ---- where things are ----

    // A plate's bed as a box on the ground.
    plateBox(id) {
      const plate = this.plate(id);
      return plate && plate.box ? [[plate.box[0], plate.box[1], 0], [plate.box[2], plate.box[3], 0]] : null;
    }

    // The room a plate's objects may take.
    plateVolume(id) {
      const box = this.plateBox(id);
      const height = this.data && this.data.bed.height;
      // Just under the bed still counts as on it (3DBed.cpp, the build
      // volume's own allowance).
      return box ? [[box[0][0], box[0][1], -1], [box[1][0], box[1][1], height || 1e9]] : null;
    }

    // The box round some of the copies: those on a plate, those selected.
    boxOf(wanted) {
      let box = null;
      if (!this.data) return null;
      this.data.placings.forEach((placing, index) => {
        if (placing.box && wanted(placing, index)) box = M.union(box, placing.box);
      });
      return box;
    }

    // Everything there is to see: every plate and every object.
    bounds() {
      let box = this.boxOf(() => true);
      for (const plate of this.plates) {
        if (!plate.box) continue;
        const factor = (plate.box[3] - plate.box[1]) / 200;
        box = M.union(box, [
          [plate.box[0], plate.box[1], BED_MODEL_Z - 5],
          [plate.box[2] + (GAP_LEFT + ICON) * factor, plate.box[3] + (TEXT_Y + NAME) * factor, 1],
        ]);
      }
      return box;
    }

    // ---- drawing ----

    // The bed and the plates. In Preview only the plate shown is drawn,
    // and nothing but its body, as the reference draws it there.
    drawBed(frame, { current, all, below }) {
      const r = this.renderer;
      const gl = r.gl;
      const matrix = M.multiply(frame.projection, frame.view);
      const shown = this.plate(current);
      const mode = dark() ? 'dark' : 'light';
      if (this.axes && shown) {
        // At the scene's origin while every plate is shown; at the plate
        // shown when it is the only one (3DBed.cpp `set_axes_mode`).
        const at = all ? [0, 0] : shown.origin;
        const place = M.translation(at[0], at[1], BED_GROUND);
        const turns = [
          [0, 0, -1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1],
          [1, 0, 0, 0, 0, 0, -1, 0, 0, 1, 0, 0, 0, 0, 0, 1],
          M.identity(),
        ];
        turns.forEach((turn, axis) => r.drawFlat(this.axes, M.chain(matrix, place, turn), AXES[axis]));
      }
      if (shown && shown.box && !below) {
        if (this.bedModel) {
          // The vendor's model of the bed, its middle at the bed's.
          const place = M.translation((shown.box[0] + shown.box[2]) / 2, (shown.box[1] + shown.box[3]) / 2, BED_MODEL_Z);
          r.drawMesh(this.bedModel, frame, place, { tint: BED[mode] });
        } else {
          // No model: the bed's shape, a shade darker than the model
          // would be (3DBed.cpp `render_default`).
          r.blend(true);
          r.drawFlat(shown.fill, M.multiply(matrix, M.translation(0, 0, BED_GROUND)), [...BED[mode].slice(0, 3).map((part) => part * 0.8), 1]);
          r.blend(false);
        }
      }
      for (const plate of this.plates) {
        if (!plate.box || (!all && plate !== shown)) continue;
        const selected = plate === shown;
        r.blend(true);
        if (!selected && !below) r.drawFlat(plate.fill, M.multiply(matrix, M.translation(0, 0, PLATE.ground)), PLATE.fill[mode]);
        const colour = below ? PLATE.lineBelow : (selected ? PLATE.lineSelected : PLATE.line)[mode];
        const grid = M.multiply(matrix, M.translation(0, 0, PLATE.gridGround));
        r.drawLines(plate.thin, grid, colour, 1);
        r.drawLines(plate.bold, grid, colour, 2);
        if (selected && !below && this.bedPicture) {
          // The vendor's picture of the bed, as it is, over the whole plate
          // (PartPlate.cpp `render_logo_texture`).
          r.drawPicture(plate.picture, this.bedPicture, M.multiply(matrix, M.translation(0, 0, PLATE.ground + 0.02)), { plain: true });
        }
        if (all) {
          const ground = M.multiply(matrix, M.translation(0, 0, PLATE.ground));
          if (plate.number) r.drawPicture(plate.number, plate.numberPicture, ground, { plain: true, transparent: below });
          if (plate.name) r.drawPicture(plate.name, plate.namePicture, ground, { plain: true, transparent: below });
        }
        r.blend(false);
      }
      gl.depthMask(true);
    }

    // How to draw a copy: the whole of it with its matrix, or, while some
    // of its object's parts are dragged, each part with its own.
    pieces(placing) {
      const moving = this.moving;
      const parts = this.data.objects[placing.object].part_list || [];
      if (!moving || moving.object !== placing.object) return [{ matrix: placing.matrix, range: null }];
      return parts.map((part, at) => ({
        matrix: moving.parts.has(at) ? M.multiply(placing.matrix, moving.parts.get(at)) : placing.matrix,
        range: part.triangles,
      }));
    }

    // Where part `at` of a copy is drawn: with the copy, unless it is
    // being dragged.
    partMatrix(placing, at) {
      const moving = this.moving;
      return moving && moving.object === placing.object && moving.parts.has(at) ? M.multiply(placing.matrix, moving.parts.get(at)) : placing.matrix;
    }

    // The objects: `pass` 0 for what is opaque, 1 for what is seen through.
    drawObjects(frame, pass) {
      if (!this.data) return;
      const r = this.renderer;
      r.blend(pass === 1);
      this.data.placings.forEach((placing) => {
        const mesh = this.meshOf(placing.object);
        if (!mesh) return;
        const volume = placing.plate !== null || this.plates.length === 1 ? this.plateVolume(placing.plate) : null;
        const painting = this.painting && this.painting.object === placing.object;
        if (painting) {
          if (placing.id !== this.painting.copy) return;
          (this.data.objects[placing.object].part_list || []).forEach((part) => {
            if (part.kind !== 'part' && part.triangles) r.drawMesh(mesh, frame, placing.matrix, { palette: this.palette, pass, volume, range: part.triangles });
          });
          return;
        }
        // The copy being cut: each side of the plane in its colour
        // (cut.js), its helper parts as they are; while connectors are
        // placed, what is over the plane is not drawn (`clip`).
        const cutting = this.cutting && this.cutting.copy === placing.id ? this.cutting : null;
        if (cutting) {
          const pieces = cutting.pieces;
          // A bent surface's halves are drawn by the tool in place of
          // the part (curved.js); its helpers as they are.
          if (cutting.hidden) {
            (this.data.objects[placing.object].part_list || []).forEach((part) => {
              if (part.kind !== 'part' && part.triangles) r.drawMesh(mesh, frame, placing.matrix, { palette: this.palette, pass, volume, range: part.triangles });
            });
            return;
          }
          for (const part of this.data.objects[placing.object].part_list || [{ kind: 'part', triangles: null }]) {
            // With pieces given to sides, the pieces stand in for the
            // printed parts, each in its side's colour (`PartSelection::render`).
            if (pieces && part.kind === 'part') continue;
            // A half ghosted is see-through (`apply_side_visibility`).
            const ghost = part.kind === 'part' && cutting.split && cutting.split.alpha && cutting.split.alpha.some((a) => a > 0 && a < 1);
            if (ghost) r.blend(true);
            r.drawMesh(mesh, frame, placing.matrix, { palette: this.palette, pass, volume, range: part.triangles, split: part.kind === 'part' ? cutting.split : null, clip: cutting.clip || null });
            if (ghost) r.blend(false);
          }
          if (pieces && pass === 0) {
            const at = M.translation(...pieces.offset);
            pieces.ranges.forEach((range, piece) => {
              r.drawMesh(pieces.mesh, frame, at, { tint: [...(pieces.sides[piece] ? pieces.upper : pieces.lower), 1], range, volume });
            });
          }
          return;
        }
        for (const piece of this.pieces(placing)) {
          r.drawMesh(mesh, frame, piece.matrix, {
            palette: this.palette,
            // A copy that is not to be printed is a shadow of itself.
            tint: placing.printable ? [0, 0, 0, -1] : UNPRINTABLE,
            pass,
            volume,
            range: piece.range,
          });
        }
      });
      r.blend(false);
    }

    // The objects of one plate as Preview shows them under the print:
    // seen through, in their filaments' colours (GCodeViewer.cpp
    // `render_shells`; GLCanvas3D.hpp `set_shell_transparence`, 0.2).
    drawShells(frame, plate) {
      if (!this.data) return;
      const r = this.renderer;
      r.blend(true);
      for (const placing of this.data.placings) {
        const mesh = this.meshOf(placing.object);
        if (!mesh || !placing.printable || (placing.plate !== plate && this.plates.length > 1)) continue;
        r.drawMesh(mesh, frame, placing.matrix, { palette: this.palette, alpha: 0.2, ghost: true, pass: 1, emission: 0.1 });
      }
      r.blend(false);
    }

    // What is selected (a set of copies, by their `id`): an outline, a
    // faint grey, and the corners of the box round it (Selection.cpp
    // `render_bounding_box`: a fifth of each edge at each corner). With
    // `parts` (part ids of the one copy selected) only those parts, and
    // `box` the box round them.
    drawSelection(frame, selection, parts = null, partBox = null) {
      if (!this.data || !selection.size) return;
      const r = this.renderer;
      const matrix = M.multiply(frame.projection, frame.view);
      // A copy being painted is not outlined: its tool draws it.
      const chosen = this.data.placings.filter((placing) => selection.has(placing.id) && this.meshOf(placing.object) && !(this.painting && this.painting.object === placing.object));
      if (!chosen.length) return;
      const byPart = Boolean(parts && parts.size);
      r.outline(() => {
        for (const placing of chosen) {
          const mesh = this.meshOf(placing.object);
          if (!byPart) {
            for (const piece of this.pieces(placing)) r.drawFlat(mesh, M.multiply(matrix, piece.matrix), [1, 1, 1, 1], undefined, piece.range);
            continue;
          }
          // Only the parts selected.
          (this.data.objects[placing.object].part_list || []).forEach((part, at) => {
            if (parts.has(part.id)) r.drawFlat(mesh, M.multiply(matrix, this.partMatrix(placing, at)), [1, 1, 1, 1], undefined, part.triangles);
          });
        }
      }, SELECTED);
      const box = byPart ? partBox : this.boxOf((placing) => selection.has(placing.id));
      if (!box) return;
      const key = JSON.stringify(box);
      if (key !== this.cornersOf) {
        r.drop(this.corners);
        const ends = [];
        for (const corner of M.corners(box)) {
          for (let axis = 0; axis < 3; axis += 1) {
            const other = corner.slice();
            const far = corner[axis] === box[0][axis] ? box[1][axis] : box[0][axis];
            other[axis] = corner[axis] + (far - corner[axis]) * 0.2;
            ends.push(...corner, ...other);
          }
        }
        this.corners = r.lines(ends);
        this.cornersOf = key;
      }
      r.drawLines(this.corners, matrix, [...SELECTED, 1], 1.5);
    }

    // What is under a pixel: { placing } for a copy of an object, { plate }
    // for a plate's bed, or null.
    pick(camera, frame, x, y, { plates }) {
      if (!this.data) return null;
      const r = this.renderer;
      const hit = r.pick(x, y, frame.projection, (projection) => {
        const matrix = M.multiply(projection, frame.view);
        if (plates) {
          this.plates.forEach((plate, index) => {
            if (plate.fill) r.drawFlat(plate.fill, matrix, GL.Renderer.pickColour(0x800000 + index));
          });
        }
        this.data.placings.forEach((placing, index) => {
          const mesh = this.meshOf(placing.object);
          if (mesh) r.drawFlat(mesh, M.multiply(matrix, placing.matrix), GL.Renderer.pickColour(index + 1));
        });
      });
      if (!hit) return null;
      if (hit >= 0x800000) return { plate: this.plates[hit - 0x800000] };
      return { placing: hit - 1 };
    }

    // Which part of a copy is under a pixel, by its place in the object's
    // list; null where none is (Selection.cpp, its volume mode: Alt).
    pickPart(frame, x, y, index) {
      const placing = this.data && this.data.placings[index];
      const mesh = placing && this.meshOf(placing.object);
      if (!mesh) return null;
      const r = this.renderer;
      const hit = r.pick(x, y, frame.projection, (projection) => {
        const matrix = M.multiply(projection, frame.view);
        (this.data.objects[placing.object].part_list || []).forEach((part, at) => {
          r.drawFlat(mesh, M.multiply(matrix, this.partMatrix(placing, at)), GL.Renderer.pickColour(at + 1), undefined, part.triangles);
        });
      });
      return hit ? hit - 1 : null;
    }
  }

  Scene.filamentColour = filamentColour;
  Scene.dark = dark;
  GL.Scene = Scene;
})(window.EdgeGL);
