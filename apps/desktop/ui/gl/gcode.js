'use strict';

// The G-code window (GCodeViewer.cpp `GCodeWindow::render`): the lines of
// the plate's G-code round the move the moves slider is on, that line
// marked, each line's number, its command, its words and its comment told
// apart, a line longer than 55 characters cut short as the reference cuts
// it (the session does that). It stands under the legend at the right and
// follows the slider; it is not scrolled by itself, the reference's being
// a window of the lines round the move and nothing else, and the wheel
// over it moves the slider a move at a time as the wheel over the slider
// does. [C] and the legend's button show and hide it.
//
// A plate's G-code can be megabytes, so nothing of it is held whole:
// each move's line comes once a print, a stretch at a time
// (`toolpath_lines`, 4 bytes a move), and the lines themselves in blocks of
// CHUNK as they come into sight (`gcode_lines`), the last few blocks kept.
// Only the rows in sight are in the page, made once and filled again.
(function (GL) {
  const { el } = GL.controls;

  // Lines asked for at once, and blocks of them kept for each print.
  const CHUNK = 256;
  const KEPT = 48;
  // Blocks on their way at once: while the slider is dragged only the
  // blocks then in sight are asked for, each when one before it is in.
  const IN_FLIGHT = 2;

  // A line as the reference splits it: the command, its words, the comment.
  function parts(text) {
    const at = text.indexOf(';');
    const code = at < 0 ? text : text.slice(0, at);
    const comment = at < 0 ? '' : text.slice(at);
    const words = code.split(' ').filter(Boolean);
    return [words[0] || '', words.length > 1 ? ` ${words.slice(1).join(' ')}` : '', comment];
  }

  class GcodeWindow {
    // `host`: call and bytes (the session), and `step(by)` to move the
    // moves slider.
    constructor(parent, host) {
      this.host = host;
      this.lines = el('div', { className: 'gl-gcode-lines' });
      this.node = el('div', { className: 'gl-gcode overlay', hidden: true }, this.lines);
      this.node.setAttribute('role', 'region');
      this.node.setAttribute('aria-label', 'G-code');
      parent.append(this.node);
      this.rows = [];
      // What each print has of it: by the print's key.
      this.prints = new Map();
      this.wanted = null;
      this.pending = 0;
      this.node.addEventListener('wheel', (event) => {
        event.preventDefault();
        this.host.step((event.deltaY < 0 ? 1 : -1) * (event.shiftKey || event.ctrlKey || event.metaKey ? 5 : 1));
      }, { passive: false });
    }

    // What is held of a print: each move's line once it has come, the
    // blocks of lines, how many lines the file has.
    held(print) {
      let held = this.prints.get(print.key);
      if (!held) {
        held = { moveLines: null, loading: false, chunks: new Map(), asked: new Set(), flying: 0, total: print.path.index.gcode_lines || 0 };
        this.prints.set(print.key, held);
        // The prints let go of are let go of here too.
        for (const key of [...this.prints.keys()]) if (key !== print.key && this.prints.size > 3) this.prints.delete(key);
      }
      return held;
    }

    // Shows the lines round move `move` (an index) of `print`, placed
    // under the legend; `shown` false hides the window.
    show(print, args, move, shown, place) {
      const total = print && print.path ? print.path.index.gcode_lines || 0 : 0;
      if (!shown || !total || move < 0) {
        this.node.hidden = true;
        this.wanted = null;
        return;
      }
      this.wanted = { print, args, move, place };
      if (!this.pending) this.pending = requestAnimationFrame(() => { this.pending = 0; this.render(); });
    }

    async loadMoveLines(print, held, args) {
      held.loading = true;
      const total = print.path.total;
      const lines = new Uint32Array(total);
      let from = 0;
      try {
        while (from < total) {
          const bytes = await this.host.bytes('toolpath_lines', { ...args, from, count: print.path.index.at_once });
          const header = new DataView(bytes, 0, 24);
          const magic = String.fromCharCode(...new Uint8Array(bytes, 0, 4));
          if (magic !== 'EDGL' || header.getUint32(8, true) !== from) throw new Error('those are not the moves\' lines');
          const count = header.getUint32(12, true);
          if (!count) break;
          lines.set(new Uint32Array(bytes.slice(24, 24 + 4 * count)), from);
          from += count;
        }
        held.moveLines = lines;
      } catch (error) {
        this.host.say(error.message, true);
      }
      held.loading = false;
      this.again();
    }

    async loadChunk(held, chunk, args) {
      held.asked.add(chunk);
      held.flying += 1;
      try {
        const answer = await this.host.call('gcode_lines', { ...args, from: chunk * CHUNK + 1, count: CHUNK });
        held.chunks.set(chunk, answer.lines);
        // The blocks furthest from the one just come are let go of first.
        while (held.chunks.size > KEPT) {
          let far = null;
          for (const known of held.chunks.keys()) if (far === null || Math.abs(known - chunk) > Math.abs(far - chunk)) far = known;
          held.chunks.delete(far);
          held.asked.delete(far);
        }
      } catch (error) {
        held.asked.delete(chunk);
      } finally {
        held.flying -= 1;
      }
      this.again();
    }

    again() {
      if (this.wanted && !this.pending) this.pending = requestAnimationFrame(() => { this.pending = 0; this.render(); });
    }

    // A line's text, or null while its block is on its way.
    text(held, line, args) {
      const chunk = Math.floor((line - 1) / CHUNK);
      const block = held.chunks.get(chunk);
      if (block) return block[(line - 1) % CHUNK] ?? '';
      if (!held.asked.has(chunk) && held.flying < IN_FLIGHT) this.loadChunk(held, chunk, args);
      return null;
    }

    render() {
      const wanted = this.wanted;
      if (!wanted) return;
      const { print, args, move, place } = wanted;
      const held = this.held(print);
      if (!held.moveLines) {
        if (!held.loading) this.loadMoveLines(print, held, args);
        this.node.hidden = true;
        return;
      }
      const line = held.moveLines[Math.min(move, held.moveLines.length - 1)];
      if (!line) {
        this.node.hidden = true;
        return;
      }
      // Under the legend, down to the moves slider.
      this.node.style.top = `${place.top}px`;
      this.node.style.right = `${place.right}px`;
      this.node.style.bottom = `${place.bottom}px`;
      this.node.hidden = false;
      const height = this.lines.clientHeight;
      if (!this.rowHeight) {
        const probe = this.row();
        probe.number.textContent = '0';
        this.lines.append(probe.node);
        this.rowHeight = probe.node.offsetHeight || 17;
        probe.node.remove();
      }
      const count = Math.max(1, Math.floor(height / this.rowHeight));
      // The lines round it, as the reference picks them: the line in the
      // middle, the window kept within the file.
      const total = held.total;
      let start = Math.max(1, line - Math.floor(count / 2));
      const end = Math.min(total, start + count - 1);
      start = Math.max(1, end - count + 1);
      while (this.rows.length < end - start + 1) {
        const row = this.row();
        this.rows.push(row);
        this.lines.append(row.node);
      }
      const digits = String(end).length;
      for (let at = 0; at < this.rows.length; at += 1) {
        const row = this.rows[at];
        const number = start + at;
        if (number > end) {
          row.node.hidden = true;
          continue;
        }
        row.node.hidden = false;
        row.node.classList.toggle('current', number === line);
        row.number.textContent = String(number).padStart(digits, ' ');
        const text = this.text(held, number, args);
        const [command, words, comment] = text === null ? ['', '', ''] : parts(text);
        row.command.textContent = command;
        row.words.textContent = words;
        row.comment.textContent = comment;
      }
      // Ask for the block after the one in sight while it is quiet, so a
      // slider moving on finds it there.
      this.text(held, Math.min(total, end + 1), args);
    }

    row() {
      const number = el('span', { className: 'gl-gcode-number' });
      const command = el('span', { className: 'gl-gcode-command' });
      const words = el('span', { className: 'gl-gcode-words' });
      const comment = el('span', { className: 'gl-gcode-comment' });
      const node = el('div', { className: 'gl-gcode-line' }, number, ' ', command, words, comment);
      return { node, number, command, words, comment };
    }
  }

  GL.GcodeWindow = GcodeWindow;
})(window.EdgeGL);
