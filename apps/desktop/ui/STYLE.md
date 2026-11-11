# The page's look

How the desktop app's page is styled, so the next piece of UI follows it
without rereading the CSS. The rule it serves is in `CLAUDE.md`: the look is
the app's own; dark first, light kept working; modern web, streamlined and
spare. What the page *offers* (controls, options, wording) is the C++
application's and is listed in `../UI_PARITY.md`; how it looks is decided
here.

In short: near-black surfaces in clear steps, one accent (the orange) kept
for what is primary, active or set by hand, hairlines in place of boxes and
shadows, small radii, compact rows, small capitals for group labels, a
fixed-width face for values.

## Where things are

| File | Holds |
| --- | --- |
| `style.css`, part 1 | The tokens: every colour, size and step. Dark is the base (`:root`), light is laid over it in one `@media (prefers-color-scheme: light)` block |
| `style.css`, part 2 | The base: the page, text, headings, `.quiet`, `.error`, `.row` |
| `style.css`, part 3 | The components, each written once |
| `style.css`, part 4 | The page's parts, in the order they stand on it |
| `printers.css` | The printer card and Printer Selection: only what is theirs alone |
| `process.css` | The process pages (`process.js`): the switch and the box of text, which came with them, the option row, the pages' tabs, the search's list; and the sidebar's width |
| `gl/gl.css` | What lies over the 3D views |
| `edit.css` | Prepare as an editor: the toolbar and the number fields over the stage, the object list |
| `app.js`, "controls" | `listbox` (the page's own list for a `<select>`) and the tooltips |

The theme follows the system (`prefers-color-scheme`); there is no switch in
the page. A switch, when there is one, sets the light block's values under a
selector of its own beside the media query; nothing else has to change,
because no rule outside part 1 names a colour.

The strip, the tab bar, the stage's older overlays (the plates, the legend,
the sliders, the notes) and Device are not on the tokens yet. They use the older names at the foot of part 1 (`--bg`,
`--panel`, `--line`, `--strip…`, `--bar…`, `--stage…`), which stay until
those parts are restyled. New rules do not use them.

## Tokens

Use a token for every colour, radius, font size and control height. A new
value is a new token in part 1, with the light value beside it, or it is not
needed.

### Surfaces

| Token | For | Dark | Light |
| --- | --- | --- | --- |
| `--surface-app` | Behind everything | `#08090b` | `#e8eaee` |
| `--surface-well` | Sunk: fields, the body of a table, a list to pick in | `#0b0d10` | `#ffffff` |
| `--surface-panel` | The sidebar | `#111418` | `#f4f5f7` |
| `--surface-raised` | Cards, dialogs, a table's head | `#171b20` | `#ffffff` |
| `--surface-float` | What floats: lists, menus, filters, tooltips | `#1d2228` | `#ffffff` |
| `--fill-hover` | Laid over any surface under the pointer | white 6% | ink 5.5% |
| `--fill-active` | Pressed; the row the keys are on | white 11% | ink 10% |

A level is told from the next by its surface and one hairline. Shadows are
for what floats only (`--shadow-float`).

### Text

| Token | For | On panel, dark | On panel, light |
| --- | --- | --- | --- |
| `--text-1` | What is read: values, names, titles | 15.5 : 1 | 16.3 : 1 |
| `--text-2` | Labels, what explains | 8.4 : 1 | 7.1 : 1 |
| `--text-3` | Hints, units, placeholders, group labels | 5.9 : 1 | 4.7 : 1 |
| `--text-off` | What is off (not held to a contrast) | 2.8 : 1 | 2.2 : 1 |

Every level but the last keeps 4.5 : 1 on every surface it is used on.
Check a new pairing before using it.

### Lines

`--line-1` between rows and sections; `--line-2` round a control, a card, a
dialog; `--line-3` round a control under the pointer, and round a tick box
(3 : 1 against the well, the box being all there is to see of it).

### Accent and states

| Token | For |
| --- | --- |
| `--accent`, `--accent-hover`, `--accent-active` | The primary button; a tick; a filter or a sort in force; the mark of a value set by hand; links |
| `--accent-soft` | A tint under what is selected |
| `--on-accent` | Text on the accent (7.6 : 1 dark, 5.2 : 1 light) |
| `--danger`, `--danger-soft` | What went wrong, and the tint under it |
| `--warn`, `--ok` | A warning; done |
| `--focus` | The focus ring: 2 px, the accent |

One accent. If two things on a screen are orange, one of them is not
primary: make it plain.

### Shape, space, type

| Token | Value | For |
| --- | --- | --- |
| `--radius-s`, `-m`, `-l` | 2, 3, 5 px | Swatches, ticks, chips; controls; cards, dialogs, what floats |
| `--space-1` … `--space-6` | 4, 8, 12, 16, 24, 32 px | Gaps and padding |
| `--control-s`, `-m`, `-l` | 22, 28, 32 px | Heights: a chip or an icon button; a field or a button; a table's head |
| `--font-ui` | The system's | Everything |
| `--font-mono` | The system's fixed-width | A value that is a number: a field's number, a size in a table, a nozzle, a path |
| `--size-caps` | 10.5 px | Group labels, in capitals, spaced `--caps-spacing` |
| `--size-small` | 11.5 px | Hints, units, a table's head, a section's title |
| `--size-label` | 12 px | Labels |
| `--size-body` | 13 px | Everything else |
| `--size-title`, `--size-page` | 15, 18 px | A dialog's title; a page's |
| `--time-fast`, `--time`, `--ease` | 90 ms, 140 ms | A colour changing; something appearing |
| `--z-raised`, `-overlay`, `-float`, `-menu`, `-tip` | 1, 10, 30, 40, 50 | A sticky head; over the stage; lists and filters; the menu; a tooltip |

`--mark-chevron`, `--mark-search` and `--mark-check` are the small pictures
drawn into a control (a picture cannot take a token for its colour, so each
has a dark and a light drawing in part 1). `--cover-plate` is the light plate
behind a printer's picture, which is drawn for a light page.

## Components

### Buttons

```html
<button type="button">Cancel</button>                 <!-- plain: an outline -->
<button type="button" class="primary">Confirm</button> <!-- the one that goes on; one to a view -->
<button type="button" class="quiet">…</button>         <!-- no outline until pointed at -->
<button type="button" class="link">Change…</button>    <!-- reads as text -->
<button type="button" class="icon tool" title="settings" aria-label="…"><svg …/></button>
<div class="joined"><button>All</button><button>Clear all</button></div>
```

`.icon` alone only takes a button's own look away (the strip and the stage
size theirs themselves); `.icon.tool` is the square icon button of the
controls. An icon button always has an `aria-label`, and a `title` for its
tooltip. Icons are inline SVG, drawn in strokes of `currentColor`.

### Fields

`input[type=text]`, `[type=number]`, `[type=search]` and `select` are styled
as they stand: 28 px high, on the well, a hairline round them.

- A number has no spinner: it is typed, or stepped with the arrow keys. It
  is set in the fixed-width face.
- A number with a unit: `<span class="unit"><input type="number"> mm</span>`.
  The frame is the span's. `.unit.set` marks a value set by hand with a bar
  of the accent (the sidebar's process fields set it from what the project
  holds).
- A search field carries its magnifier and its own clear mark.
- Tick boxes and radio buttons are the page's own; nothing to add.
- A label is a `<label>`: over its field by default, or to its left in a
  `.grid` (label, then field of 96 px), a `.card-row` or an option row.
- A length that may be a percentage is a text field in a `.unit` with
  `class="number"` (the fixed-width face); its unit is filled in as it is
  typed: "mm" after a number, nothing after one that carries its own %.
- A box of text is a `<textarea>`: on the well, a hairline round it, at
  least two lines, pulled taller by its corner. `class="code"` sets it in
  the fixed-width face without wrapping, for G-code and scripts.
- A field that can also be picked from carries a chevron at its end
  (`button.icon.opt-more`), which opens the values with `showMenu`.

### Switch

```html
<label class="switch">Advance <input type="checkbox" role="switch"></label>
```

Two states of one thing that take effect at once ("Advance"): a tick box
drawn as a track with a knob, the accent when on. Its word comes first, as
the C++ has it. A tick box stays a tick box wherever a setting is on or off.

### Option rows

One setting of many, in a page of them (`process.js` `processRow`):

```html
<div class="opt [set] [off] [unread] [noted] [wide]" data-key="…">
  <span class="opt-head"><label class="opt-label" for="…" title="…">Layer height</label><span class="opt-follow" role="img" title="…"></span></span>
  <span class="opt-field"><!-- a tick box, .unit, a list, a text field --></span>
  <button type="button" class="icon opt-reset" title="…" aria-label="…"><svg …/></button>
</div>
```

- The controls stand in one column (112 px, up to 150 px where the sidebar
  is wider); a label that does not fit goes on to a second line, and is
  never cut. A tick box stands at the head of the column.
- `.set`: set by hand. A bar of the accent at the head of the control, the
  label in bold, and the reset arrow (hidden otherwise). This is what the
  accent means in a form: yours, not the preset's.
- `.off`: greyed out by another setting. The label in `--text-off`, the
  control disabled.
- `.unread` / `.noted`: what the engine does with it, in `.opt-follow`: a
  ring in `--text-3` (not read), a dot in `--warn` (followed with a note).
  Quiet on purpose; the sentence is in the label's tooltip, where the keys
  reach it too.
- `.wide`: a box of text takes the row under its label.
- Rows under a `.process-page > h3` group label; several options of one
  C++ line under an `.opt-line-label`, indented.

### Tabs within a section

`#process-tabs` holds `button.process-tab[role="tab"]`: text buttons on a
hairline, the open one with the accent under it, as many to a line as fit.
A `span.process-count` in a tab is how many of its settings are set by
hand. Left and Right, Home and End move between them.

### Lists: `listbox(select, options)`

A `<select>`'s popup belongs to the system and cannot be styled. Two ways:

1. **Leave the `<select>` alone.** Its closed face is the page's (the
   chevron, the well); its popup is the system's. Enough for a short list
   away from the main flow (the legend's view type).
2. **`listbox(select)`** (`app.js`) gives it a list of the page's own. The
   `<select>` stays in the page, out of sight, and stays what the code
   fills, reads and listens to: `fill()`, `.value`, `.disabled`, `.title`
   and `'change'` work as before, and the list follows whatever is done to
   it. Used for every list of the sidebar.

```js
listbox($('process'));                       // wraps the <select> where it stands
el('div', {}, swatch, listbox(select, { bare: true }));   // a new one: put what it returns in the page
listbox($('printer'), { trigger: $('printer-row'), decorate(row, option) { … } });
```

- `bare`: no frame, for a list inside a card's row or a row with a frame of
  its own.
- `trigger`: an element of the page's as the closed face (it holds a
  `.lb-value`, which is given the selected name). The printer card's head is
  one: all of it opens the list.
- `decorate(row, option)`: adds to a row of the open list (the printers'
  pictures; `row.classList.add('lb-action')` for an entry that is not a
  value but something to do).
- `<optgroup>` becomes a titled group, `<hr>` a line, a `hidden` option is
  left out, a `disabled` one is shown off. An option's `title` is its row's.

Keys (ARIA's select-only combobox; the keys stay on the closed face):
closed, Down, Up, Enter, Space, Home, End open it; open, the arrows, Home,
End, Page Up, Page Down move; Enter, Space or Alt+Up pick; Escape and Tab
close it with nothing changed; typing goes to the next row that starts with
what was typed (one letter again: the next such row). It closes when the
page scrolls or the window is resized. A name too long for the closed face
is cut with an ellipsis and given whole in its tooltip.

### Sections that fold

```html
<section class="fold" id="s-…">
  <h2 class="with-tools">
    <button type="button" class="fold-title" aria-expanded="true">Printer</button>
    <button type="button" class="icon tool" …>…</button>   <!-- optional, after the title -->
  </h2>
  <div class="fold-body">
    <h3>Quality</h3>   <!-- a group's label: small capitals running on into a hairline -->
    …
  </div>
</section>
```

`app.js` folds every `.fold-title` it finds. Titles are written as the C++
application has them; the capitals are the style's (`text-transform`), not
the text's.

### Cards

`.card` is one frame round what belongs together, with a hairline between
its children. `.card-row` is a row of it: what it is on the left (a
`<label>`, an `<h3>`), its value or control on the right. Put a `bare` list
in a row, not a framed one. The printer card and the filament list are
cards; the process fields are not (they are a form, not one object).

### Chips

```js
el('span', { className: 'chip' }, el('span', { className: 'chip-key', textContent: 'Brand:' }), ' Prusa ', removeButton)
```

Something in force, with the button (`.icon`, an `aria-label`) that takes it
away.

### Tables

```html
<div class="…scrolls…"><table class="data-table"> … </table></div>
```

A sticky head on the raised surface; rows of 40 px with hairlines; `th.num`
/ `td.num` for figures (right, fixed-width); `td.na` for a value that is not
known; `tr.on` for a row that is ticked (a bar of the accent at its head, a
tint under it). In a head cell: `button.th-sort` (the title; it holds
`span.th-way` for the arrow, and shows a faint one where a click would sort)
and `button.th-filter` (the funnel; `.on` when its filter is in force). Set
`aria-sort` on the `<th>`. Column widths are the table's own business
(`#pt-table th…` in `printers.css`).

### Dialogs

`<dialog>` is the shell: raised surface, hairline, `--radius-l`, the dark
backdrop. A small one holds a `<form method="dialog">` (title `<h2>`, text,
`.row.end` with the buttons, primary first). A large one sets `padding: 0`
and lays itself out in parts, ending in

```html
<div class="dialog-foot"><div class="grow"><!-- what went wrong --></div><button class="primary">…</button><button>…</button></div>
```

Open with `showModal()` so the keys stay inside it. Escape closes the
innermost thing first.

### Toolbars

```html
<div class="toolbar" role="toolbar" aria-label="Prepare">
  <button type="button" class="icon tool" title="Add [Ctrl+I]" aria-label="Add"><svg …/></button>
  <span class="toolbar-gap"></span>
  <button type="button" class="icon tool" aria-pressed="true" title="Move [M]" aria-label="Move"><svg …/></button>
</div>
```

Icon tools in a row on a floating surface: over the 3D stage, where a row
of the page would not be at home. A tool is 28 px square with a 16 px icon;
`.toolbar-gap` is the hairline between groups. A tool that stays on while
it is in use (Move, Rotate, Scale) carries `aria-pressed`, and the one that
is on is the accent on its tint. That is the toolbar's one use of the
accent: what was done (undo) and what can be added are plain. The keys of a
tool are in its tooltip, in the reference's own wording ("Move [M]"); a
tool that cannot be used now is `disabled`, never hidden. A panel that
belongs to the tool that is on (`.edit-panel`: the number fields) stands
under the toolbar on the same surface, and is the only thing that does.

### Trees: `.tree`

The object list: one frame (`.tree`) with a head row of column titles in
small capitals, group rows (`.obj-plate`: a plate, with what can be done to
it at the right), and item rows on a grid of name, a 22 px tool and a
54 px value. A row that is selected has the accent's tint and a bar of the
accent at its head (`aria-selected="true"`, as `tr.on` in a table); a row
that is off (not to be printed) is `--text-off`. A child row is its
parent's row indented by 22 px. A name too long is cut with an ellipsis and
given whole in its `title`. The value's list is a `bare` listbox, as in a
card's row. A name is renamed in place: the name becomes a field of the
same height, Enter takes it, Escape leaves it.

### What floats

- `.popover`: a small form beside what opened it (a column's filter), inside
  its dialog. `.check-list` inside it is a list of values to tick.
- `#menu` (`showMenu(x, y, items)`): a menu at a point.
- Tooltips: write a `title`. The page shows it in its own look, after a
  moment under the pointer and beside what the keys are on, and puts the
  attribute aside while it does (it is back when the tooltip goes). Nothing
  else is needed; `\n` in a title is a new line.

## Do

- Design in dark, then look at it in light. Both must read.
- Take colours, sizes and radii from the tokens.
- Use a component before writing a rule; when something is needed a second
  time, make it a component in part 3 and say so here.
- Keep rows compact (28 px controls, 32 to 40 px rows) and let hairlines and
  space, not boxes, separate things.
- Numbers that are measured or set: fixed-width face, figures in columns,
  right-aligned in a table.
- Cut a long name with an ellipsis and give it whole in a `title`.
- Make every state visible on dark: hover (a fill), focus (the 2 px ring),
  selected (the accent), off (`--text-off`).
- Keep ids and `data-` hooks when restyling; the code and the tests hold on
  to them.

## Don't

- No colour, pixel size or radius written into a rule outside part 1.
- No gradients, glows or gloss; no shadow on anything that does not float.
- No second accent, and no accent on what is merely there.
- No system-looking control left as it comes: no spinner, no grey tick box,
  no bare `<select>` in the main flow.
- No capitals typed into the text for style; no wording changed for looks.
- No control shown that does nothing.
- No library for what a few lines of the page's own code do (a preference,
  not a rule: say why when one is taken).
- No colour of the page's on what is drawn in the 3D scene itself. The
  handles of the tools there (arrows, rings, grips) are in the axes' own
  colours, which are the reference's (`ColorRGB::X`, `Y`, `Z`), like the
  bed and the objects.
- No motion longer than `--time`, and none that `prefers-reduced-motion`
  does not switch off.

## Checking a change

`cargo run -- --serve <port>` (with `EDGE_DESKTOP_DATA` naming a scratch
folder) and a browser: dark and light, a window of 700 px and a wide one, a
very long preset name, the keys alone (Tab through it; the focus ring must
always be in sight), and the contrast of any new pairing of text and
surface.

The sidebar is 300 px wide in a narrow window and up to 380 px in a wide one
(`clamp(300px, 24vw, 380px)`, in `process.css`): look at a form in it at
both ends.
