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
| `settings.css` | The pages of settings (`settings.js`; the process's, a filament's, the printer's): the switch and the box of text, which came with them, the option row, the pages' tabs, the search's list, the panel a filament's or the printer's pages open in; and the sidebar's width |
| `gl/gl.css` | What lies over the 3D views that `gl/` draws: the navigator, the sliders, the position window, the labels |
| `hosts.css` | Sending to printers (`hosts.js`): "Physical Printer" and "Send G-code to printer host" (`dialog.host-dialog`), the tick with its words (`label.tick`), the upload queue, and Device with a printer's page and its files |
| `edit.css` | Prepare as an editor: where its toolbar and the number fields stand over the stage, the object list; a plate's own tools, its lock mark and "Plate Settings" (`plates.js`) |
| `app.js`, "controls" | `listbox` (the page's own list for a `<select>`) and the tooltips |

The theme follows the system (`prefers-color-scheme`); there is no switch in
the page. A switch, when there is one, sets the light block's values under a
selector of its own beside the media query; nothing else has to change,
because no rule outside part 1 names a colour.

Every part of the page is on the tokens: the project strip, the tab bar,
the sidebar, the stage and what lies over it, Device and the dialogs. The
older names (`--bg`, `--panel`, `--strip…`, `--bar…`, `--stage…`) are gone.

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

The strip and the tab bar stand on `--surface-app`, the sidebar on
`--surface-panel`, Device on `--surface-app`.

| Token | For | Dark | Light |
| --- | --- | --- | --- |
| `--surface-stage` | The 3D stage, until the scene is drawn over it: the reference's scene background (`gl/scene.js`) | `#54545a` | `#e7e7e7` |
| `--surface-overlay` | What lies over the stage (`.overlay`, `.toolbar`): the float surface, a little see-through | 96 % | 95 % |
| `--overlay-blur` | The scene blurred behind an overlay (`backdrop-filter`) | | |
| `--line-swatch` | The edge of a swatch of colour, which has to show on any colour | | |
| `--scroll-thumb`, `--scroll-thumb-hover` | A scrollbar's thumb | | |

The overlay is opaque enough that `--text-3` keeps 4.5 : 1 on it over the
brightest scene behind it (white behind dark, black behind light); make it
no thinner.

### Text

| Token | For | On panel, dark | On panel, light |
| --- | --- | --- | --- |
| `--text-1` | What is read: values, names, titles | 15.5 : 1 | 16.3 : 1 |
| `--text-2` | Labels, what explains | 8.4 : 1 | 7.1 : 1 |
| `--text-3` | Hints, units, placeholders, group labels | 5.9 : 1 | 4.7 : 1 |
| `--text-off` | What is off (not held to a contrast) | 2.8 : 1 | 2.2 : 1 |
| `--code-command` | A G-code line's command, in the G-code window | 9.6 : 1 on the overlay | 6.9 : 1 on white |

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
| `--bar-height` | 40 px | The tab bar, and the strip's head beside it, whose hairlines meet |
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

`.icon` alone only takes a button's own look away; `.icon.tool` is the
square icon button of the controls (22 px; 28 px in a toolbar). An icon
button always has an `aria-label`, and a `title` for its tooltip. Icons
are inline SVG, drawn in strokes of `currentColor`; no characters (« ▾ ⤢)
for icons.

A primary button that cannot be used loses the accent (a grey fill), and
so does the menu joined to it while it is not pointed at: an accent on
what cannot be pressed would be an accent on what is merely there.

A button with its menu (the slice button) is two buttons in a `.joined`,
the second holding `<svg class="chevron">`.

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
- A secret ("API Key / Password") is `input[type=password]`, styled as a text field. It never holds the secret: the page is told only whether one is set, which the placeholder says ("Set (type to change it)"), with a `button.link` "Clear" beside it.
- A box of text is a `<textarea>`: on the well, a hairline round it, at
  least two lines, pulled taller by its corner. `class="code"` sets it in
  the fixed-width face without wrapping, for G-code and scripts.
- A field that can also be picked from carries a chevron at its end
  (`button.icon.opt-more`), which opens the values with `showMenu`.

### A tick with its words

```html
<label class="tick"><input type="checkbox"> Switch to Device tab after upload.</label>
```

A tick box or a radio button on one line with what it does, in `--text-1`,
the whole line clickable (`hosts.css`). Options that belong to one thing
("Time-lapse", "Heated Bed Leveling", the plate's side) stand together in
`.send-options`, set in by a hairline at the left. A warning about a choice
is `p.warn` (`--warn`).

### Switch

```html
<label class="switch">Advance <input type="checkbox" role="switch"></label>
```

Two states of one thing that take effect at once ("Advance"): a tick box
drawn as a track with a knob, the accent when on. Its word comes first, as
the C++ has it. A tick box stays a tick box wherever a setting is on or off.

### Option rows

One setting of many, in a page of them (`settings.js` `settingsRow`; the
same row on every kind's pages):

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
- `.fixed`: shown, not offered (set in a dialog of the reference's own, or
  not a setting to change here): as `.off`, the reason in the label's
  tooltip.
- A filament's override (`input.opt-override` at the head of `.opt-head`):
  a tick box before the label turns the filament's own value on; off, the
  field is disabled and shows the value in force in its place.
- Rows under a `.process-page > h3` group label; several options of one
  C++ line under an `.opt-line-label`, indented. An extruder's own option
  is named `key#n` in `data-key`; field ids are `po-` (process), `fo-`
  (filament), `mo-` (printer) and the name.

### Tabs

```html
<div class="tabs [scrolls]" role="tablist" aria-label="…">
  <button type="button" role="tab" aria-selected="true">[<svg …/>] Prepare</button>
</div>
```

Text (with an icon, if it has one) on a hairline, the open one with 2 px
of the accent under it and its text in bold. One look for the main tabs
(`#tabs`, 40 px, with the C++ tab icons) and the tabs of the pages of
settings (`#process-tabs` in the sidebar, `#preset-tabs` in the panel:
`button.process-tab`, 28 px and smaller type; a `span.process-count` in a
tab is how many of its settings are set by hand).

`.scrolls` keeps the tabs on one line: where they do not fit they are
scrolled sideways, by a trackpad, by the wheel (`app.js` turns it), by the
keys (Left, Right, Home, End, which give the next tab the keys), and the
open tab is always scrolled into sight. The tab cut at the edge is what
says there are more. Every row of pages' tabs is this, the process's, a
filament's and the printer's alike: six tabs do not fit the sidebar's
300 px, and two rows of tabs read as two rows of choices, the lower of
which moves when the upper is picked. An overflow menu was the other way;
it would hide pages behind a click and need its own keys, and the row is
short enough that scrolling it is little to ask.

### The panel: `#preset-panel`

A filament's or the printer's pages take the place of the sidebar's
sections (`#side.presets-open` hides `#side-scroll`) and the sidebar widens
to `clamp(340px, 30vw, 520px)` while they are open (`#plater.presets-open`);
the status line and the profiles folder stay under them. Its head
(`#preset-head`): the way back (`button.icon.tool`, a chevron, "Back to the
sidebar [Esc]"), the title in `--size-title` with the preset's name under
it cut with an ellipsis, the reset-all arrow and "Advance". For a filament,
`#preset-filaments` holds a numbered swatch for each filament
(`aria-pressed` on the one shown, ringed in the accent). Then the search
and the tabs (`.tabs.scrolls`, as the Process section's). All of that is
the panel's `.fold-pin`, pinned at the top of its scroll (`#preset-scroll`)
as the Process head is in the sidebar's (see "Sections that fold"); the
page, or the search's list in its place, scrolls under it. Escape anywhere
in it but in a list or a box of text goes back, and the keys go back to
what opened it (not to a swatch moved by).

Opened by an edit icon (`button.icon.tool`, a pencil, "Click to edit
preset"): in the last column of a filament's row, and at the right of the
printer card's head (`.printer-head`), outside the part that opens the list
of printers. It is `--text-2` until pointed at.

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

In the sidebar's scroll each section's title bar stays at the top while
its section is in sight (`position: sticky`). A section whose head is more
than its title (Process: the title, the preset, the search, the pages'
tabs) puts the head in a `.fold-pin`, which stays in sight while the
section's body scrolls under it:

```html
<section class="fold" id="s-process">
  <div class="fold-pin">
    <h2 class="with-tools">…</h2>
    <div class="fold-body"><!-- what stays: the preset, the search, the tabs --></div>
  </div>
  <div class="fold-body"><!-- what scrolls under it --></div>
</section>
```

`app.js` keeps the head's height as `--pinned` on the section, and
whatever is gone to or given the keys in the body has a `scroll-margin` of
it, so it is never left under the head; a page opened from the head's tabs
starts under it. The search's list (`role="listbox"`, `.found-row`s) stands
in the body in place of the page and has no scroll of its own; while it is
shown the body is at least the scroll's height (`--room`), and typing in
the search brings the head to the top, so the list is read under it.
Folding hides both bodies. The panel of a filament's or the printer's
pages has its head pinned the same way, with the same list. The
sidebar's order is Printer, Filament Management, Objects, Process: the
long one last, where it can have the whole height when scrolled to.

### What opens in place

`<details><summary>` (the stage's "About this print") carries the same
chevron as a section that folds, drawn by the style; the system's triangle
is hidden.

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

A dialog of settings in rows ("Plate Settings") is a `.card` of
`.card-row`s inside the form: the label on the left in `--text-2`, the
control on the right 240 px wide (a `bare` listbox, a text field); what a
row opens (an order, ranges of layers) is a row of its own under it, and
what is wrong stands in `--danger` at the left of the buttons. A list that
is set by the code is set with its value and a `data-value` attribute, so
that its closed face (which follows the `<select>`'s attributes) shows it.
A box that only tells something ("Plate bed types reset") is `#confirm`
with its second button hidden (`inform` in `app.js`).

### An order of filaments

```html
<div class="ps-order" role="list"><button class="ps-chip" role="listitem" data-filament="3"><span class="swatch"></span>3</button>…</div>
```

What the reference draws as a row of colours to drag ("First layer
filament sequence", a range of later layers): a chip for each filament,
its swatch and its number in the fixed-width face, on the well with a
hairline round it. A chip is dragged into place by the pointer (the one
held is ringed in the accent), or given the keys and moved with Left and
Right; its `aria-label` says where it stands ("Filament 3, 2 of 7").

### Swatches

`span.swatch`: a small square of a colour that means something (a
filament, a line type), 10 px, `--radius-s`, with a `--line-swatch` edge so
that white and black both show. The colours in it are data (the
reference's line-type colours, the project's filament colours): never a
token.

### Toolbars

```html
<div class="toolbar" role="toolbar" aria-label="Prepare">
  <button type="button" class="icon tool" title="Add [Ctrl+I]" aria-label="Add"><svg …/></button>
  <span class="toolbar-gap"></span>
  <button type="button" class="icon tool" aria-pressed="true" title="Move [M]" aria-label="Move"><svg …/></button>
</div>
```

Icon tools in a row on the overlay surface: over the 3D stage, where a row
of the page would not be at home. `.toolbar.vertical` is a column (the
plates); a text tool in one is a `button.quiet` ("View", "Fit part"). A tool is 28 px square with a 16 px icon;
`.toolbar-gap` is the hairline between groups. A tool that stays on while
it is in use (Move, Rotate, Scale, Lay on face) carries `aria-pressed`, and the one that
is on is the accent on its tint. That is the toolbar's one use of the
accent: what was done (undo) and what can be added are plain. The keys of a
tool are in its tooltip, in the reference's own wording ("Move [M]"); a
tool that cannot be used now is `disabled`, never hidden. A panel that
belongs to the tool that is on (`.edit-panel`: the number fields) stands
under the toolbar on the same surface, and is the only thing that does.
A tool whose thing is set by hand ("Customize current plate" on a plate
that sets anything of its own) carries `.set`: a dot of the accent at its
corner, the mark of what is yours, not a pressed state.

A plate's own tools (`#plate-tools`, `.toolbar.vertical.plate-tools`) are
the one toolbar that stands in the scene rather than at the stage's edge:
`plates.js` places it at the far right corner of the plate shown each time
the scene is drawn, and hides it while that corner is out of sight. Only
the plate shown has it.

A tool with nothing to type ("Lay on face") has the same panel with its
title and one line of what to do (`p.edit-hint`, `--text-2`). What a tool
draws in the scene over an object (the faces of "Lay on face") keeps the
reference's own colours, as the handles do; it is not the page's accent.

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
same height, Enter takes it, Escape leaves it. A part of an object (`.obj-row.child.part`)
is a child row with no tool: in its name the mark of its kind first
(`svg.obj-kind`, 13 px, a stroke of `currentColor` like every icon; one
drawing for each kind in `objects.js`), then the name (`.obj-label`, cut
with an ellipsis); for a part or a modifier its filament as a swatch and
a `bare` list whose first entry is "default" (closed, it shows the number
the part then prints in). It is selected like any row (the accent's tint
and bar), and only parts of one object are selected at once. A part that
is not printed (`.helper`: a modifier, a negative part, a support blocker
or enforcer) has its name in `--text-3`, slanted; what it is is in the
name's tooltip.

A plate that is locked has a lock drawn after its name at its row and in
the corner of its tile (`svg.lock-mark`, 11 px, `--text-3`): a mark, not
a control; the lock is changed from the plate's tools or its menu.

What is said when a click is refused ("Selection conflicts" and why) goes
in the status line, which keeps a `\n` as a new line (`white-space:
pre-line`); no dialog is opened for it.

### What lies over the stage

Everything over the 3D stage is on one surface: `.overlay` (a panel: the
legend, "About this print", the progress, a note) or `.toolbar` (tools),
both `--surface-overlay` with `--line-2` round it, `--radius-l` and
`--shadow-float`; `.overlay.chip-label` is a value's small label (a
slider's layer and height, the move's number) in the fixed-width face. No
button or list on the stage has a look of its own: they are the page's,
on that surface, in light as in dark.

Where each stands (`#stage` sets the margins, `--stage-inset` all round):

| Where | What |
| --- | --- |
| Top left | The plates (`#plates`, a vertical toolbar): "Slice all" (or "All Plates", the statistics) over a hairline, then each plate's number; the one shown is the accent on its tint, ✓ in `--ok` for sliced, ! in `--danger` for failed, a lock at the bottom left of a locked one. The right button or the menu key on a tile opens the plate's menu |
| By the plate shown | Its own tools (`#plate-tools`), at its far right corner |
| Top, from `--stage-left` | Prepare's toolbar, centred; under it "Object Operations" (`.edit-panel`) |
| Top right, `--stage-right` in | The legend (`--legend-width`); the layer slider's labels keep the band right of it |
| Right | The layers slider and its one-layer button; on the slider, what is set at layers (below) |
| Right, under the legend | The G-code window (below); beside the legend, at its left, where the legend leaves too little room under it |
| Bottom left | The navigator cube, and Fit / View (a toolbar) under it; "About this print" beside them |
| Bottom | The moves slider; the position window over its right end; the progress at the right |
| Middle | What there is to say where there is nothing to show (`#empty`) |

Where the stage is too narrow for the legend and "About this print" side by
side (a container query on `#stage`), the legend's height leaves the foot of
the stage to the notes. Sliders: a track on the overlay surface, the part
shown in the accent, thumbs of the accent ringed in `--surface-float`; the
one the keys move is ringed in the accent, and the focus ring is a second
ring of `--focus` outside it. The navigator's faces, the labels' frames
over the objects and the axes' letters keep the reference's colours.

### The G-code window

`.gl-gcode` (`gl/gcode.js`): an `.overlay` panel of lines of G-code in the
fixed-width face at `--size-small`, 17 px a line, as wide as 55
characters and a line number, so it keeps its width as the slider moves.
Each line's number and comment in `--text-3`, its command in
`--code-command` (the one colour the page has for code: a muted yellow on
dark, a dark ochre on light), its words in `--text-1`. The line of the
move shown is marked as a selected row is: the accent's tint and a bar of
the accent at its head. It does not scroll; the wheel over it moves the
moves slider. Its button in the legend's head (`#legend-gcode`, a `.tool`
with `aria-pressed`) has the accent's tint while the window is on.

### What is set at layers (the layers slider's ticks)

A mark across the slider's track at each layer something is set at
(`.gl-tick-mark`: `--text-1`, or the filament's colour for a filament
change, ringed in the float surface so it shows on the track's fill), and
its kind in a small label at the track's left (`.gl-tick-label`: the
overlay surface, `--line-2`, `--size-caps`, the reference's word: "Pause",
"Custom", "Template", "Color"). Its tooltip says what it writes. At the
layer the thumb taken up is on, the label drops under the thumb's own
label and is framed in the accent, and a 18 px `.icon` button with a
cross (`.gl-tick-delete`, titled "Delete Pause" and the rest) stands at
the track's right. The slider's menu (`#menu`) lists what can be added
there or done to what is there; a filament in a menu has its swatch
before its name (`more.swatch` of `showMenu`'s entries).

The dialog for custom G-code (`#custom-gcode`) is a small dialog: title,
the reference's sentence as the label, a `textarea.code` of six lines,
"OK" (off while the box is empty) and "Cancel".

### A printer's page (Device)

With a connection, Device is a bar on the panel's surface (the `.tabs`
"Printer page" / "Storage", the address in the fixed-width face at the
right in `--text-3`, the tool buttons) over the printer's page, which fills
the rest. The page in the frame is the printer's: the app does not style it
and puts `--cover-plate` behind it, as a browser puts white behind a page
that sets no background. "Storage" is a `.data-table` in a well, its row's
"Print" and "Delete" plain buttons at the right; what is wrong or empty is
said in the well.

### The upload queue

A large dialog (`#queue-dialog`): a `.data-table` in a well, a row selected
as a ticked row is (`tr.on`), the progress a thin bar of the accent with
the percentage beside it, the state in the colour it means (`--ok`
completed, `--danger` an error with its message, `--text-3` called off).
"Cancel selected" and "Show error message" at the left of the foot, "Close"
the primary.

### Scrollbars

A thin thumb on nothing, `--scroll-thumb`, the same in Chromium (the
standard `scrollbar-width` and `scrollbar-color`) and in WebKit (the
window on a Mac, which reads only `::-webkit-scrollbar`). Rows that scroll
sideways (`.tabs.scrolls`) and the strip's and the plates' columns show
none.

### What floats

- `.popover`: a small form beside what opened it (a column's filter), inside
  its dialog. `.check-list` inside it is a list of values to tick.
- `#menu` (`showMenu(x, y, items)`): a menu at a point. An entry with
  `items` opens a menu of its own beside it (`#submenu`, the same look as
  `.menu`), marked with a chevron at its end; it opens by the pointer, a
  click or Right, and Left or Escape closes it. An entry's `title` is its
  tooltip (the C++ help text).
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
folder) and a browser: dark and light, the window's least size (900 × 560)
and its first (1280 × 820) and a wide one, a very long preset name, the
keys alone (Tab through it; the focus ring must always be in sight), and
the contrast of any new pairing of text and surface.

Then the window itself (`cargo build`, then `target/debug/edge-desktop`):
it is WebKit, not Chromium, and the page is built into the binary, so
build after every change to it. What differs there and has to be written
for both: scrollbars (`::-webkit-scrollbar` as well as the standard
properties), `backdrop-filter` (with its `-webkit-` twin),
`backface-visibility` (with its `-webkit-` twin). The window follows the
system's light or dark; started with `-NSRequiresAquaSystemAppearance YES`
after its arguments it is light whatever the system is, which checks light
without touching the system's setting.

The sidebar is 300 px wide in a narrow window and up to 380 px in a wide one
(`clamp(300px, 24vw, 380px)`, in `settings.css`), 340 to 520 px while a
filament's or the printer's pages are open: look at a form in it at both
ends, and at a box of G-code at 900 px, the window's minimum.
