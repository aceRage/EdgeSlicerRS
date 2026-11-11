'use strict';

// The graphics card's side of the 3D views: one WebGL2 context, the
// programs, and the few kinds of thing that are drawn (shaded meshes,
// flat triangles, lines of a width, pictures). What is drawn where is in
// scene.js and toolpath.js.
(function (GL) {
  const M = GL.math;

  function compile(gl, type, source) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      const log = gl.getShaderInfoLog(shader);
      gl.deleteShader(shader);
      throw new Error(`a shader does not compile: ${log}`);
    }
    return shader;
  }

  class Renderer {
    // Throws when the browser has no WebGL2, or when it cannot be used.
    constructor(canvas) {
      this.canvas = canvas;
      const gl = canvas.getContext('webgl2', {
        antialias: true, alpha: false, depth: true, stencil: false, powerPreference: 'high-performance',
      });
      if (!gl) throw Object.assign(new Error('WebGL2 is not available'), { missing: true });
      this.gl = gl;
      this.programs = {};
      for (const [name, sources] of Object.entries(GL.shaders)) this.programs[name] = this.link(sources);
      this.size = [1, 1];
      this.ratio = 1;
      this.bytes = 0;

      // A line's quad: along it, and across it.
      this.lineCorners = this.staticBuffer(new Float32Array([0, -0.5, 1, -0.5, 0, 0.5, 1, 0.5]));
      // The whole view, for what is drawn over all of it.
      this.screen = gl.createVertexArray();
      gl.bindVertexArray(this.screen);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.staticBuffer(new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1])));
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
      gl.bindVertexArray(null);

      this.mask = null;
      // One pixel, to see what is under the pointer.
      this.picker = gl.createFramebuffer();
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.picker);
      const colour = gl.createRenderbuffer();
      gl.bindRenderbuffer(gl.RENDERBUFFER, colour);
      gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA8, 1, 1);
      gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, colour);
      const depth = gl.createRenderbuffer();
      gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
      gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT24, 1, 1);
      gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    }

    link({ vertex, fragment }) {
      const gl = this.gl;
      const program = gl.createProgram();
      gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, vertex));
      gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, fragment));
      gl.linkProgram(program);
      if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
        throw new Error(`a program does not link: ${gl.getProgramInfoLog(program)}`);
      }
      const uniforms = {};
      const count = gl.getProgramParameter(program, gl.ACTIVE_UNIFORMS);
      for (let at = 0; at < count; at += 1) {
        const name = gl.getActiveUniform(program, at).name.replace(/\[0\]$/, '');
        uniforms[name] = gl.getUniformLocation(program, name);
      }
      return { program, uniforms };
    }

    use(name) {
      const program = this.programs[name];
      this.gl.useProgram(program.program);
      return program.uniforms;
    }

    staticBuffer(data) {
      const gl = this.gl;
      const buffer = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      this.bytes += data.byteLength;
      return buffer;
    }

    // Sizes the drawing to the canvas as it is shown. True when it changed.
    resize() {
      const box = this.canvas.getBoundingClientRect();
      const ratio = window.devicePixelRatio || 1;
      const [width, height] = [Math.max(1, Math.round(box.width * ratio)), Math.max(1, Math.round(box.height * ratio))];
      const changed = this.canvas.width !== width || this.canvas.height !== height;
      if (changed) {
        this.canvas.width = width;
        this.canvas.height = height;
      }
      this.size = [width, height];
      this.css = [box.width, box.height];
      this.ratio = ratio;
      return changed;
    }

    // ---- what can be drawn ----

    // Triangles for the mesh program, from a mesh as it is sent
    // (api/scene.rs `mesh_bytes`): every triangle gets vertices of its
    // own, with the normal of its face, as the reference shades its
    // objects (GLModel::init_from an indexed triangle set).
    meshFromBytes(bytes) {
      const view = new DataView(bytes);
      const magic = String.fromCharCode(...new Uint8Array(bytes, 0, 4));
      if (magic !== 'EDGM' || view.getUint32(4, true) !== 1) throw new Error('that is not a mesh');
      const [count, triangles, flags, alike] = [8, 12, 16, 20].map((at) => view.getUint32(at, true));
      const positions = new Float32Array(bytes, 24, count * 3);
      const corners = new Uint32Array(bytes, 24 + count * 12, triangles * 3);
      const marks = flags & 1 ? new Uint8Array(bytes, 24 + count * 12 + triangles * 12, triangles) : null;
      const out = new ArrayBuffer(triangles * 48);
      const floats = new Float32Array(out);
      const signed = new Int8Array(out);
      const unsigned = new Uint8Array(out);
      let [low, high] = [[Infinity, Infinity, Infinity], [-Infinity, -Infinity, -Infinity]];
      for (let t = 0; t < triangles; t += 1) {
        const [a, b, c] = [corners[t * 3] * 3, corners[t * 3 + 1] * 3, corners[t * 3 + 2] * 3];
        const [ax, ay, az] = [positions[a], positions[a + 1], positions[a + 2]];
        const [ux, uy, uz] = [positions[b] - ax, positions[b + 1] - ay, positions[b + 2] - az];
        const [vx, vy, vz] = [positions[c] - ax, positions[c + 1] - ay, positions[c + 2] - az];
        let [nx, ny, nz] = [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx];
        const length = Math.hypot(nx, ny, nz);
        if (length > 0) { nx /= length; ny /= length; nz /= length; } else { nz = 1; }
        const mark = marks ? marks[t] : alike;
        for (let corner = 0; corner < 3; corner += 1) {
          const from = corners[t * 3 + corner] * 3;
          const vertex = t * 3 + corner;
          for (let axis = 0; axis < 3; axis += 1) {
            const value = positions[from + axis];
            floats[vertex * 4 + axis] = value;
            if (value < low[axis]) low[axis] = value;
            if (value > high[axis]) high[axis] = value;
          }
          signed[vertex * 16 + 12] = Math.round(nx * 127);
          signed[vertex * 16 + 13] = Math.round(ny * 127);
          signed[vertex * 16 + 14] = Math.round(nz * 127);
          unsigned[vertex * 16 + 15] = mark;
        }
      }
      return this.mesh(out, triangles * 3, triangles ? [low, high] : null);
    }

    // Triangles for the mesh program from vertices of 16 bytes each.
    mesh(data, count, box = null) {
      const gl = this.gl;
      const vao = gl.createVertexArray();
      gl.bindVertexArray(vao);
      const buffer = this.staticBuffer(data);
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 16, 0);
      gl.enableVertexAttribArray(1);
      gl.vertexAttribPointer(1, 3, gl.BYTE, true, 16, 12);
      gl.enableVertexAttribArray(2);
      gl.vertexAttribPointer(2, 1, gl.UNSIGNED_BYTE, false, 16, 15);
      gl.bindVertexArray(null);
      return { vao, buffer, count, bytes: data.byteLength, box };
    }

    // A mesh from triangles given as points [x, y, z] with a normal each:
    // [[point, normal], ...], three to a triangle.
    meshFromPoints(points) {
      const out = new ArrayBuffer(points.length * 16);
      const floats = new Float32Array(out);
      const signed = new Int8Array(out);
      points.forEach(([point, normal], at) => {
        floats.set(point, at * 4);
        const unit = M.normalize(normal);
        for (let axis = 0; axis < 3; axis += 1) signed[at * 16 + 12 + axis] = Math.round(unit[axis] * 127);
      });
      return this.mesh(out, points.length);
    }

    // Flat triangles from positions x, y, z, ...
    triangles(positions) {
      const gl = this.gl;
      const vao = gl.createVertexArray();
      gl.bindVertexArray(vao);
      const buffer = this.staticBuffer(new Float32Array(positions));
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
      gl.bindVertexArray(null);
      return { vao, buffer, count: positions.length / 3, bytes: positions.length * 4 };
    }

    // Lines from ends x, y, z, x, y, z for each.
    lines(ends) {
      const gl = this.gl;
      const vao = gl.createVertexArray();
      gl.bindVertexArray(vao);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.lineCorners);
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
      const buffer = this.staticBuffer(new Float32Array(ends));
      for (const [location, offset] of [[1, 0], [2, 12]]) {
        gl.enableVertexAttribArray(location);
        gl.vertexAttribPointer(location, 3, gl.FLOAT, false, 24, offset);
        gl.vertexAttribDivisor(location, 1);
      }
      gl.bindVertexArray(null);
      return { vao, buffer, count: ends.length / 6, bytes: ends.length * 4 };
    }

    // A picture on a flat four-cornered patch: corners as [x, y, z] from
    // the bottom left, anticlockwise.
    patch(corners) {
      const gl = this.gl;
      const [a, b, c, d] = corners;
      const data = new Float32Array([...a, 0, 0, ...b, 1, 0, ...d, 0, 1, ...c, 1, 1]);
      const vao = gl.createVertexArray();
      gl.bindVertexArray(vao);
      const buffer = this.staticBuffer(data);
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 20, 0);
      gl.enableVertexAttribArray(1);
      gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 20, 12);
      gl.bindVertexArray(null);
      return { vao, buffer, count: 4, bytes: data.byteLength };
    }

    // A picture from an image or a canvas, its first row at the top.
    picture(source) {
      const gl = this.gl;
      const texture = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
      gl.generateMipmap(gl.TEXTURE_2D);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      return texture;
    }

    // Colours to look up by number: rows of 256, four bytes a colour.
    palette(rows) {
      const gl = this.gl;
      const texture = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 256, rows, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(256 * rows * 4));
      return { texture, rows };
    }

    setPalette(palette, data) {
      const gl = this.gl;
      gl.bindTexture(gl.TEXTURE_2D, palette.texture);
      gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, 256, palette.rows, gl.RGBA, gl.UNSIGNED_BYTE, data);
    }

    drop(thing) {
      if (!thing) return;
      const gl = this.gl;
      if (thing.vao) gl.deleteVertexArray(thing.vao);
      if (thing.buffer) {
        gl.deleteBuffer(thing.buffer);
        this.bytes -= thing.bytes || 0;
      }
      if (thing.texture) gl.deleteTexture(thing.texture);
    }

    // ---- drawing ----

    begin(background) {
      const gl = this.gl;
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.viewport(0, 0, this.size[0], this.size[1]);
      gl.clearColor(background[0], background[1], background[2], 1);
      gl.depthMask(true);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      gl.enable(gl.DEPTH_TEST);
      gl.depthFunc(gl.LEQUAL);
      gl.disable(gl.BLEND);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      gl.disable(gl.CULL_FACE);
    }

    blend(on, write = !on) {
      const gl = this.gl;
      if (on) gl.enable(gl.BLEND); else gl.disable(gl.BLEND);
      gl.depthMask(write);
    }

    // A shaded mesh. `model` takes it into the scene; `pass` is 0 for
    // what is opaque, 1 for what is not, 2 for all of it. `range`: only
    // these triangles, [first, how many].
    drawMesh(mesh, frame, model, { palette = null, tint = [0, 0, 0, -1], alpha = 1, ghost = false, pass = 2, emission = 0, volume = null, range = null } = {}) {
      const gl = this.gl;
      const u = this.use('mesh');
      const viewModel = M.multiply(frame.view, model);
      gl.uniformMatrix4fv(u.u_view_model, false, viewModel);
      gl.uniformMatrix4fv(u.u_projection, false, frame.projection);
      gl.uniformMatrix3fv(u.u_view_normal, false, M.normalMatrix(viewModel));
      gl.uniformMatrix4fv(u.u_model, false, model);
      gl.uniform4fv(u.u_tint, tint);
      gl.uniform1f(u.u_alpha, alpha);
      gl.uniform1i(u.u_ghost, ghost ? 1 : 0);
      gl.uniform1i(u.u_pass, pass);
      gl.uniform1f(u.u_emission, emission);
      gl.uniform1i(u.u_volume, volume ? 1 : 0);
      if (volume) {
        gl.uniform3fv(u.u_volume_min, volume[0]);
        gl.uniform3fv(u.u_volume_max, volume[1]);
      }
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, palette ? palette.texture : null);
      gl.uniform1i(u.u_palette, 0);
      // A mirrored copy is wound the other way round (3DScene.cpp, a
      // left-handed volume is drawn with the front face clockwise).
      gl.enable(gl.CULL_FACE);
      gl.frontFace(M.determinant3(model) < 0 ? gl.CW : gl.CCW);
      gl.bindVertexArray(mesh.vao);
      if (range) gl.drawArrays(gl.TRIANGLES, range[0] * 3, range[1] * 3);
      else gl.drawArrays(gl.TRIANGLES, 0, mesh.count);
      gl.frontFace(gl.CCW);
      gl.disable(gl.CULL_FACE);
    }

    // Anything with positions first, in one colour. `range`: only these
    // triangles of a mesh, [first, how many].
    drawFlat(thing, matrix, colour, mode = this.gl.TRIANGLES, range = null) {
      const gl = this.gl;
      const u = this.use('flat');
      gl.uniformMatrix4fv(u.u_matrix, false, matrix);
      gl.uniform4fv(u.u_color, colour);
      gl.bindVertexArray(thing.vao);
      if (range) gl.drawArrays(mode, range[0] * 3, range[1] * 3);
      else gl.drawArrays(mode, 0, thing.count);
    }

    // Lines, `width` pixels wide as the page counts pixels.
    drawLines(lines, matrix, colour, width) {
      if (!lines.count) return;
      const gl = this.gl;
      const u = this.use('line');
      gl.uniformMatrix4fv(u.u_matrix, false, matrix);
      gl.uniform4fv(u.u_color, colour);
      gl.uniform2f(u.u_viewport, this.size[0] / 2, this.size[1] / 2);
      gl.uniform1f(u.u_width, Math.max(1, width * this.ratio));
      gl.bindVertexArray(lines.vao);
      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, lines.count);
    }

    drawPicture(patch, texture, matrix, { plain = false, transparent = false } = {}) {
      const gl = this.gl;
      const u = this.use('picture');
      gl.uniformMatrix4fv(u.u_matrix, false, matrix);
      gl.uniform1i(u.u_plain, plain ? 1 : 0);
      gl.uniform1i(u.u_transparent, transparent ? 1 : 0);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.uniform1i(u.u_picture, 0);
      gl.bindVertexArray(patch.vao);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }

    // Marks what `draw` draws as selected: an outline round it, and a
    // faint grey over it (GLCanvas3D.cpp, the selection highlight; the
    // "thin" style's widths and strengths).
    outline(draw, colour) {
      const gl = this.gl;
      const [width, height] = this.size;
      if (!this.mask || this.mask.width !== width || this.mask.height !== height) {
        if (this.mask) {
          gl.deleteTexture(this.mask.texture);
          gl.deleteFramebuffer(this.mask.frame);
        }
        const texture = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        const frame = gl.createFramebuffer();
        gl.bindFramebuffer(gl.FRAMEBUFFER, frame);
        gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
        this.mask = { texture, frame, width, height };
      }
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.mask.frame);
      gl.viewport(0, 0, width, height);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.disable(gl.DEPTH_TEST);
      gl.disable(gl.BLEND);
      draw();
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.viewport(0, 0, width, height);
      const u = this.use('outline');
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, this.mask.texture);
      gl.uniform1i(u.u_mask, 0);
      gl.uniform3fv(u.u_outline_color, colour);
      gl.uniform1f(u.u_fill_alpha, 0.25);
      gl.uniform1f(u.u_outline_width, 1.5 * this.ratio);
      gl.uniform1f(u.u_rim_width, 1.0 * this.ratio);
      gl.uniform1f(u.u_rim_alpha, 0.3);
      gl.uniform2f(u.u_texel, 1 / width, 1 / height);
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
      gl.bindVertexArray(this.screen);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      gl.disable(gl.BLEND);
      gl.enable(gl.DEPTH_TEST);
    }

    // What is under a pixel of the view: `draw(matrix)` draws everything
    // that can be picked, each in the colour of its number, with a
    // projection that shows that one pixel and nothing else. Answers the
    // number, 0 where there is nothing.
    pick(x, y, projection, draw) {
      const gl = this.gl;
      const [width, height] = this.css;
      const [nx, ny] = [(2 * (x + 0.5)) / width - 1, 1 - (2 * (y + 0.5)) / height];
      const one = M.multiply(M.scaling(width, height, 1), M.translation(-nx, -ny, 0));
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.picker);
      gl.viewport(0, 0, 1, 1);
      gl.clearColor(0, 0, 0, 0);
      gl.depthMask(true);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      gl.enable(gl.DEPTH_TEST);
      gl.disable(gl.BLEND);
      draw(M.multiply(one, projection));
      const pixel = new Uint8Array(4);
      gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.viewport(0, 0, this.size[0], this.size[1]);
      return pixel[0] | (pixel[1] << 8) | (pixel[2] << 16);
    }
  }

  // The colour a number is drawn in for picking.
  Renderer.pickColour = (number) => [(number & 255) / 255, ((number >> 8) & 255) / 255, ((number >> 16) & 255) / 255, 1];

  GL.Renderer = Renderer;
})(window.EdgeGL);
