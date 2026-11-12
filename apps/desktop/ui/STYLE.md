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
| `presets.css` | A person's own presets (`presets.js`): the preset row with its cog, the dialog that asks for a name, "Transfer or discard changes" and "Compare presets" |
| `edit.css` | Prepare as an editor: where its toolbar and the number fields stand over the stage, the object list; a plate's own tools, its lock mark and "Plate Settings" (`plates.js`) |
| `objsettings.js` | The Process section's "Global" / "Objects" and an item's pages (drawn by `settings.js`); its rules are in `settings.css` and `edit.css` |
| `app.js`, "controls" | `listbox` (the page's own list for a `<select>`) and the tooltips |
| `menu.css` | The menu bar in a browser (`#menubar`, its menus `.menubar-panel`), "Keyboard Shortcuts" (`#shortcuts`) and Preferences (`#prefs`) |

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
| `--surface-track` | Sunk under a row of controls: the pages' tabs | `#08090b` | `#e3e6ea` |
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
| `--ink-on-light`, `--ink-on-dark` | A figure on a swatch of colour, by the colour's lightness, whatever the theme | `#000`, `#fff` | the same |
| `--paint-ring`, `--paint-ring-edge` | The painting tools' ring round the pointer, and its edge | white 90 %, black 35 % | the same |
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
| `--menubar-height` | 28 px | The menu bar, in a browser only, over both |
| `--side-min`, `--side-panel-min`, `--stage-min` | 300, 340, 420 px | The sidebar dragged narrowest; the sidebar while a filament's or the printer's pages are open; the stage left beside a sidebar dragged wide (the session's `SIDEBAR_MIN`, `STAGE_MIN`, `src/api/layout.rs`) |
| `--grip` | 8 px | What is dragged at an edge: the sidebar's |
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

Two states of one thing that take effect at once ("Advance", which the
`en` catalog shows as "Advanced"): a tick box drawn as a track with a
knob, the accent when on. Its word comes first, as the C++ has it. A tick box stays a tick box wherever a setting is on or off.

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
  is wider); a list's column takes up to 58 % of the row (210 px at most),
  its values running longer, the more so in other languages. A label that
  does not fit goes on to a second line, and is never cut. A tick box
  stands at the head of the column.
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
  C++ line under an `.opt-line-label`, indented. A titled group folds
  (below, "Groups that fold"). An extruder's own option
  is named `key#n` in `data-key`; field ids are `po-` (process), `fo-`
  (filament), `mo-` (printer) and the name.

### Groups that fold

```html
<h3 [class="folded"]><button type="button" class="group-fold" aria-expanded="true" aria-controls="process-group-7" title="Click to collapse / expand">Seam</button><span class="group-count" title="1 setting changed">1</span></h3>
<div class="opt-group [folded]" id="process-group-7">…rows…</div>
```

Each titled group of a page of settings folds from its title (the owner's
choice; `settings.js`, "groups that fold"): the title is a button in the
group label's own small capitals, with the chevron of a section that folds
(down open, right folded), `--text-2` when folded; Enter or Space folds
it, `aria-expanded` says which. After the title, `span.group-count`, the
number of its settings set by hand in the accent, shown open or folded so
nothing set is hidden. Folded, its rows are gone (`.opt-group.folded`) and
everything else about them goes on: the rules still show, hide and grey
them, a reset still puts them back, and its title stays (it is hidden only
when the rules hide every row, as before).

Beside the search (`.search-row`) two `.tool`s unfold and fold every group
of the page shown ("Expand all groups", two chevrons down; "Collapse all
groups", two up), in `--text-2` until pointed at; the row is hidden with
the search where an item has no pages.

Whether a group is folded is kept by the session (`layout_set`) by kind
(process, filament, printer, item), page and group, each by the message
id, so it is the same in every language and the three kinds and an
item's pages never share it; what was never folded or unfolded by hand is
as `src/api/layout.rs` `FOLDED_AT_FIRST` has it. A setting searched for
and gone to opens its folded group for the while; it is folded again when
the search is next typed in or cleared, or another page is opened.

### Two states of whose ("Global" / "Objects")

```html
<div class="joined segmented" role="group" aria-label="…">
  <button type="button" aria-pressed="true">Global</button><button type="button" aria-pressed="false">Objects</button>
</div>
```

Two joined buttons (`.joined.segmented`, `settings.css`) where a choice
changes what a whole section is about rather than one setting: the one in
force has the accent's line round it on the accent's tint and its word in
`--text-1` and bold, the other is plain in `--text-2`; `--control-s` high,
`--size-small`. A `.set` on a button puts the dot of what is set by hand
at its corner (as a toolbar tool's), here "Objects" while anything has
settings of its own. Beside it, what the section is now about
(`.process-item`: `--text-1`, cut with an ellipsis, whole in its title).
A switch (above) stays for a thing that is on or off; this is for which
of two.

### Tabs

```html
<div class="tabs" role="tablist" aria-label="…">
  <button type="button" role="tab" aria-selected="true">[<svg …/>] Prepare</button>
</div>
```

Text (with an icon, if it has one) on a hairline, the open one with 2 px
of the accent under it and its text in bold: the main tabs (`#tabs`,
40 px, with the C++ tab icons) and Device's.

### The pages' tabs: `.tabs.pages`

```html
<div id="process-tabs" class="tabs pages" role="tablist" aria-label="…">
  <button type="button" role="tab" class="process-tab" aria-selected="true">Quality<span class="process-count">2</span></button>
  …<span class="tab-break" aria-hidden="true"></span>…
</div>
```

The tabs of the pages of settings (`#process-tabs` in the sidebar, for the
project's pages and an item's; `#preset-tabs` in the panel, for a
filament's and the printer's): each tab a bounded control, so where one
ends and the next begins is never in doubt, and the open one
unmistakable. They stand on a sunk track (`--surface-track`, a `--line-1`
hairline round it, `--radius-l`, 2 px in and between); each tab is
`--surface-raised` with `--line-2` round it (`--line-3` under the
pointer), 26 px high, `--size-label`, its words in `--text-2`; the open
one has the accent round it on `--accent-soft`, its words in `--text-1`
and bold, as the segmented "Global" / "Objects" has. A `span.process-count`
in a tab is how many of its settings are set by hand, in the accent.

They wrap (the owner's choice; they were one row that scrolled): on one
row where they fit, else on as few rows as hold them, the tabs kept in
order and shared out so the widest row is as narrow as it can be (six on
two rows are three and three), every row filled to the track's width, its
tabs growing alike. `settings.js` `layPageTabs` measures each tab as it
would be open (in bold, so opening another moves none between rows) and
puts a `.tab-break` between rows, each time the tabs are drawn and when
the row's width changes (`app.js`). The process's six take two rows at
the sidebar's 300 px in English, German and French (with counts on two
of them), one in Chinese; the printer's ten (four extruders) four rows
at 340 px in German. Left and Right go along them in reading order,
across the rows, Home and End to the first and the last.

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
and the tabs (`.tabs.pages`, as the Process section's). All of that is
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

### A preset list and its cog

```html
<div class="preset-row"><label id="process-row">Preset <select id="process"></select></label><button class="icon tool preset-cog" …>…</button></div>
```

A preset list (the process's, a filament's, the printer's) is a listbox
with the reference's groups as `<optgroup>`s, "User presets" before
"System presets", and the preset in force named with "* " before it while
something is set over it (`Presets.fillList`, `Presets.label`). Its cog
(`button.preset-cog`, a 16 px gear in strokes, `--text-2` until pointed
at) stands at the right of the list on the same baseline, and in the head
of a filament's or the printer's pages before the reset arrow; it opens a
`#menu` of what can be done to the preset ("Save current %s", "Rename
preset" and "Delete this preset" only for one's own, a line, "Compare
presets"). No entry is shown that cannot be used.

### Dialogs of presets

- **A name** (`dialog.preset-dialog`, 440 px): the title, the caption over
  one text field, a line under it that says what the name would do
  (`.pn-message`: `--text-3`; `.warn` in `--warn` when it saves over a
  preset of one's own; `.invalid` in `--danger`, with OK off), and OK
  (primary) and Cancel. The line keeps its height so the buttons do not
  move as it is typed.
- **A table of options** (`dialog.preset-dialog.wide`, 820 px at most:
  "Transfer or discard changes", "Compare presets"): the head (title and
  what it is about, or the pickers in a grid of three: kind, left, right,
  with the tick under them), then a `.data-table.pc-table` in a well that
  scrolls (`.pc-scroll`): a page's title on the raised surface in
  `--text-1`, a group's in small capitals, then a row for each option,
  its label then the two values in the fixed-width face, each cut with an
  ellipsis and whole in its tooltip (an option's key is the label's). The
  buttons in `.dialog-foot`, the one that goes on primary ("Transfer",
  "Close").

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
control on the right 310 px wide, at most 60 % of the row (a `bare`
listbox, a text field: the longest choice in French is read whole); what a
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

### The list of filaments

```html
<div class="filaments-head"><h3>Filaments</h3><button class="icon tool" id="filament-remove" …/><button class="icon tool" id="filament-add" …/></div>
<div id="apply-all" class="card apply-all"><select …></select><button>Apply All</button></div>
<div id="filaments" class="card"><div class="filament"><button class="swatch swatch-pick">1</button>…list…<button class="icon tool">…pencil…</button></div>…</div>
```

The list's head is its group label running into its hairline, then the
two `.tool`s that take the last filament away (a minus, there only with
more than one) and add one (a plus, off at the most there can be). A
row's number stands on its colour (`button.swatch.swatch-pick`, 22 px,
the number in white or ink by the colour's brightness), and is the
button that opens the colour picker; the row's menu is on the right
button and the menu key, as an object's is in the list. "Apply All"
is a card of its own between the head and the list (before what it acts on): a `bare` listbox and a small plain
button (`--control-s` high), never the accent, there only where presets
are chosen here.

### Flushing volumes

```html
<button id="flush-open" class="flush-open">Flushing volumes</button>   <!-- in .filaments-head, with more than one filament -->
<dialog id="flush-dialog"><h2>…</h2><div class="flush-scroll"><table class="flush-table">…</table></div><div class="flush-under">…</div><div class="dialog-foot">…</div></dialog>
```

The reference's "Flushing volumes for filament change" (`filaments.js`):
a well holding a table of boxes, a row for each filament flushed out and
a column for each flushed in, each head a numbered swatch in the
filament's colour; the corner reads "To" over "From" across a hairline.
Rows alternate with `--fill-hover`; a box from a filament to itself is
read-only and plain. A box under the least its filament flushes, or over
the most, and with it the "Suggestion" line, are in `--danger`. Under
the well: "Re-calculate" beside the two lines of text, then
"Multiplier" with its field. The well scrolls both ways and its heads
stay put, so seven filaments and more fit at 900 × 560.
### A colour to pick: `.colour-picker`

`filaments.js`: a panel that floats beside the swatch that opened it (the
float surface, `--line-2`, `--radius-l`, `--shadow-float`), 220 px wide:
its title (the filament), a grid of chips of colours to start from
(`.cp-chip`, the one picked ringed in the accent), a square of
saturation and brightness (`.cp-area`, its knob ringed so it shows on any
colour; arrows move it, Shift by more) over a strip of hue
(`input.cp-hue`, drawn as the page's own: no system track), the colour
as a swatch and as text in the fixed-width face (typed, it is taken as
soon as it is a colour), then OK (primary) and Cancel. Escape or a click
outside cancels; Enter in the text is OK. The colours in it are data,
set by the code; the rules name none.

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
it is in use (Move, Rotate, Scale, Lay on face, Cut) carries `aria-pressed`, and the one that
is on is the accent on its tint. That is the toolbar's one use of the
accent: what was done (undo) and what can be added are plain. The keys of a
tool are in its tooltip, in the reference's own wording ("Move [M]"); a
tool that cannot be used now is `disabled`, never hidden. The painting
tools (L, P, H, N) stand in their own group after "Lay on face", as the
reference orders its gizmos. A panel that
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

The options beside "Arrange" (`#arrange-panel`, `.edit-panel` with
`.arrange-panel`) stand in the same place while the Arrange tool is
pressed, and only one of the two panels is open at a time: "Spacing" as a
slider with its number in a `.unit`, a quiet line, the switches as
`.edit-check`s between hairlines (`.arrange-rule`), and at the foot
"Arrange" (`.primary`) and "Reset".

A tool with nothing to type ("Lay on face") has the same panel with its
title and one line of what to do (`p.edit-hint`, `--text-2`). What a tool
draws in the scene over an object (the faces of "Lay on face") keeps the
reference's own colours, as the handles do; it is not the page's accent.

### A painting tool's panel (`paint.js`, `paint.css`)

The painting tools' panel is an `.edit-panel` with `.paint-panel`: the
tool's name, then rows of a label at the left and its control
(`.paint-row`), the help, and the buttons at its foot (`.paint-foot`,
plain buttons; "Erase all painting" is never the accent). Its own
components:

- **The tools** (`.paint-tools`): icon buttons of a control's height in a
  row, the one in use `aria-pressed` with the accent on its tint, as a
  toolbar's tool. Each has its name as `title` and `aria-label`.
- **A value with its slider** (`.paint-slide`): a range input (the accent
  as its colour) taking the room left, then the number in a `.unit` of
  82 px. Both set the value; the number is typed or stepped with the
  arrow keys, the wheel with Ctrl steps it on the stage.
- **Filaments to paint with** (`.paint-filaments`): a square of each
  filament's colour with its number in the fixed-width face, black or
  white (`--ink-on-light`, `--ink-on-dark`) by the colour's lightness; the
  one painted with ringed in the accent. Its tooltip is the reference's
  ("Shortcut Key 1", "Filament 10").
- **What the buttons do** (`.paint-help`): two columns at `--size-small`,
  the keys in `--text-3`, under a hairline.
- **The engine's note** (`.paint-note`): a quiet line in `--text-3` with
  what the engine does not follow of the paint.

The panel scrolls when the stage is low. The circle cursor
(`.paint-cursor`) is a ring round the pointer, drawn over the stage in
`--paint-ring` with a `--paint-ring-edge` hairline so it shows on any
model; it lets the pointer through. The colours of the paint in the
scene are the reference's (enforcers, blockers, the filaments'), drawn
with the scene's own shading, never the page's accent.

### "Cut"'s panel (`cut.js`, `edit.css`)

An `.edit-panel` with `.cut-panel`, like a painting tool's, but standing
at the stage's right under the toolbar's row rather than in the middle:
the plane and its grip are over the middle of the object, which the
panel must not hide (under 760 px it goes to the stage's foot as every
tool's panel does, and scrolls). Its rows:

- **A label and what it is about** (`.cut-row`): the label in
  `--text-2` at the left, then a value ("Build Volume", `.cut-size` in
  the fixed-width face), a `.cut-line` (the axis letter in its colour, a
  `.unit` number field, the reset as an icon tool), or a slider with its
  number (`.cut-slide`, as `.paint-slide`).
- **Three letters to press** ("Mirror cut"): plain buttons joined
  (`.cut-line.joined`), the letters in the fixed-width face.
- **One of two** ("Mode": "Planar" / "Dovetail"): two joined buttons
  (`.joined.segmented`), the chosen one `aria-pressed`; not a list, which
  for two choices hides one of them behind a click.
- **A group's heading** (`.cut-group`, "Groove", "Multiple"): small
  capitals in `--text-3`, as the sidebar's section heads. Its rows are
  `.cut-row`s, a value and its tolerance side by side as two `.unit`
  fields (88 px each), the reset after them.
- **Armed** (`.cut-armed`, "Pick face" while a click will lay the plane):
  the button in the accent's colour and outline over `--accent-soft`,
  `aria-pressed`, the hint beside it.
- **A cut being made again** ("Edit cut..."): the reference's notice at
  the top as an `.edit-hint`, the missing half's sentence as a
  `.cut-warning`. "Keep cut editable" is a tick under "Cut to parts", its
  tooltip the reference's.
- **Connectors being placed** (`connectors.js`): the panel holds them
  alone, "Connectors" as a group heading with "Remove connectors" (↺),
  "Type" as three joined buttons, "Style" and "Shape" as lists (off where
  the type has no choice), each number with its tolerance beside it and a
  reset, the keys and the pointer as the painting tools' help, "Cancel"
  and the primary "Confirm connectors" at the foot. In the scene the part
  over the plane is not drawn, the cut face is a flat light grey, and the
  connectors take the reference's colours (data, not tokens).
- **A pin's drawing** (`.pin-drawing`): an inline SVG 84 px tall across the
  panel, the pin's outline in the reference's amber, its flats dashed in
  its pale blue, its slits filled in its red (data, not tokens), the cut
  a `--text-3` line with "lower" and "upper" either side; the pin's other
  rows are ticks and number fields as the rest of the panel.
- **The surface** (`curved.js`): "Surface" and each half's "Visible" /
  "Ghost" / "Hidden" as joined buttons under a "Show halves" group
  heading; the curved sheet's counts as narrow number fields
  (`.unit.cut-count`), its "Smooth" and "Reset surface" plain buttons at
  the left (`.cut-foot.cut-start`), "Cut face updating…" a `.cut-note`.
  In the scene the sheet is drawn in the plane's grey, see-through, its
  handles the grip's grey balls (yellow under the pointer); the halves the
  booleans made stand in for the part in cyan and magenta.
- **The fork's joints** (`joints.js`): "Type" a list of the six kinds;
  a Flexi joint's "Joint", a TAC Joint's "Placement" lists; their numbers
  narrow fields with their unit, those "Auto size" sets off while it is
  on; counts as `.unit.cut-count`; ticks as the pin's; what cannot be
  made and the warnings `.cut-note`, the notes `.edit-hint`; "Reset TAC
  Joint" a plain button at the left. A TAC Joint's socket is drawn round
  it while placed in the reference's faint blue (data, not tokens).
- **The drawn surface** (`drawn.js`): "Direction" a list, "Extension",
  "Extension angle", "Angle", "Depth" and "Smoothing" each a slider with
  its number as "Thickness" has them ("Extension angle" under its "Set"
  tick, off with it), "Edit points" a tick, "Close loop", "Cut along the
  line" and "Clear line" plain buttons at the left; what to do next an
  `.edit-hint`, a stroke refused and what the line cannot do
  `.cut-warning`s. In the scene the plane and its grips are not drawn:
  the line is an amber ribbon over everything, an open line's ends amber
  balls (lighter under the pointer, green when letting go would close
  the loop), the points being edited the grip's grey balls (yellow under
  the pointer), the surface the plane's grey a quarter there (pink when
  the cut cannot be made); the reference's colours, data, not tokens.
- **A side and what is done with it** (`.cut-side`): a 10 px swatch of
  the side's tint in the scene (cyan, magenta: the reference's, data, not
  tokens) and its name in `--text-1`, then its ticks; a tick never breaks
  in its words, and the ticks go to a second line where the row is too
  narrow (German at 900 px). A tick that is off is `--text-off`.
- **A note about a value** (`.cut-note`, "Remove from"): `--warn`, at
  `--size-small`, as the reference colours it.
- **A warning** (`.cut-warning`): `--danger`, "Warning: …".
- The help is the painting tools' (`.paint-help`); "Perform cut" the one
  primary button, at the foot (`.cut-foot`), off while nothing is kept;
  "Reset cut" a plain button at the left (`.cut-foot.cut-start`).

What the tool draws in the scene keeps the reference's colours: the plane
(light grey, pink when the cut cannot be made), the section in white, the
grip's stalk yellow, its ball grey (lit yellow or in an axis's colour
under the pointer), its cones in the axes' red and green, the two sides
cyan and magenta through the mesh shader's colour plane. With pieces
given to sides, each piece is drawn in its side's tint; "Pick face"
lights the face under the pointer in white at 0.75, a little off the
surface. A dovetail's groove is drawn over the plane in the plane's
grey at 0.4, its grabbers the grip's ball and a cube.

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

An item with settings of its own (an object, a part, a height range)
has after its name a `button.icon.tool.obj-own` (in `.obj-name-cell`,
which lays the name and it in a row): three sliders in strokes of the
accent, the mark of what is set by hand, its tooltip the reference's
("Click the icon to reset all settings of the object"); a click takes them
away, one step. "Layers" (`.obj-row.child.layers`) and each height range
(`.obj-row.child.range`, a further 16 px in) are child rows at
`--size-label`, selected as any row is.

When "Layers" is selected the Process section lists the ranges' heights
(`.range-page`): a `.range-row` each, "Range" in `--text-2`, two `.unit`
text fields in the fixed-width face with "mm", a dash in `--text-3`, and
the two `.tool`s "Remove height range" (a minus) and "Add height range"
(a plus). What there is not to show ("Select an object, a part or a height
range…") is a `p.quiet.item-hint` in the page's place.

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
| Top, from `--stage-left` | Prepare's toolbar, centred; under it "Object Operations" (`.edit-panel`) or a painting tool's panel (`.paint-panel`); "Cut"'s (`.cut-panel`) under it at the right. Where the stage is narrower than 760 px the toolbar takes two rows and the panel stands under both |
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
window on a Mac, which reads only `::-webkit-scrollbar`). The strip's and
the plates' columns show none.

### What floats

- `.popover`: a small form beside what opened it (a column's filter), inside
  its dialog. `.check-list` inside it is a list of values to tick.
- `#menu` (`showMenu(x, y, items)`): a menu at a point. An entry with
  `items` opens a menu of its own beside it (`#submenu`, the same look as
  `.menu`), marked with a chevron at its end; it opens by the pointer, a
  click or Right, and Left or Escape closes it. An entry's `title` is its
  tooltip (the C++ help text). An entry that is one of several choices
  (`checked`) carries the accent's tick when it is the one in force, in
  the menu beside as in the first.
- The menu bar (`menu.js`, `menu.css`): in the app's window it is the
  system's, and the page draws nothing. In a browser it is a row of
  `--menubar-height` across the top of the page, over the strip and the
  tab bar (the body becomes a grid of two rows when `body.has-menubar`, so
  the strip's head and the tab bar still meet), on `--surface-app` with
  one hairline under it; the menus' names are quiet text (`--text-2`,
  `--size-label`), a fill under the one open. A menu is `.menu` (as
  above) under its name: each entry's words at the left and its key in a
  column at the right (`--text-3`, `--size-small`), a chevron for a menu
  beside it, the accent's tick for what is on. An entry that is off is in
  `--text-off` and keeps its tooltip (it is `aria-disabled`, not
  `disabled`, so the pointer still reaches it): for one not built the
  tooltip is what it waits for, and its words end in a faint " ·". A
  recent project whose file is gone is struck through. A menu that is
  open is not drawn again under the pointer while the session's answer
  changes; it is, once it closes. Left and Right go along the bar, Up and
  Down in a menu, Right into a menu beside, Escape back.
- Tooltips: write a `title`. The page shows it in its own look, after a
  moment under the pointer and beside what the keys are on, and puts the
  attribute aside while it does (it is back when the tooltip goes). Nothing
  else is needed; `\n` in a title is a new line.

## Words and languages

The page's words are the C++'s message ids put through `t()` (`i18n.js`),
or the page's own, left as they are; `../UI_PARITY.md` section 16 says
which and why, and a test keeps them so. For the look:

- **The language control** is in Preferences (the menu bar: the
  application menu on a Mac, File elsewhere), "General", "Language": the
  page's own list, by the names the reference gives the languages; OK
  with another reloads the page in it. No flag and no icon for it. (It
  was the last entry of the strip's menu until there was a Preferences.)
- **Nothing is shown before it is in its language**: `html.words-wait`
  hides the page (`visibility`) until `i18n.js` has the table and has put
  the static page in it.
- **Fonts**: `--font-ui` is the system's, and `<html lang>` is the
  language chosen, which is how the system picks its CJK face (PingFang,
  Hiragino on a Mac). No font is loaded.
- **Room for longer words**: German and French run a third longer than
  English, and a few lists much more. Design a row so that its label can
  take a second line and its value is cut with an ellipsis (whole in its
  tooltip) only where it is a name of the person's (a preset, a file); a
  word of the reference's should fit. `text-transform: uppercase` is safe
  (the group labels): CJK has no case.
- A tooltip with keys is the message id, then the keys in brackets
  ("Undo [Ctrl+Z]"); write it as `` `${t('Undo')} [Ctrl+Z]` ``.

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
page in German and in Chinese (Preferences, "Language"), the
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
filament's or the printer's pages are open, until it is dragged: then it
is the width dragged to, from 300 to 640 px, never leaving the stage less
than 420 px (392 px at most in a 900 px window), and at least 340 px while
those pages are open. Look at a form in it at both ends, and at a box of
G-code at 900 px, the window's minimum.

### The sidebar's edge

```html
<div id="side-grip" role="separator" tabindex="0" aria-orientation="vertical" aria-controls="side" aria-label="Sidebar width" title="…"></div>
```

The sidebar's right edge is dragged to widen or narrow it (`app.js`, "the
sidebar's width"): a strip of `--grip` over the hairline, half of it on the
stage, with the resize pointer; a short mark of `--line-3` in its middle
says it is there, and the whole edge is a 2 px line of the accent while it
is pointed at, given the keys or dragged. It is a window splitter (ARIA's
`separator` with its value): Left and Right step it 16 px (64 with Shift),
Home and End go to the least and the most the window allows, Enter or a
double-click gives the usual width back. The drag takes the pointer
(pointer capture) and selects nothing (`body.side-dragging`); the stage
follows it as it goes (the 3D view is drawn again by its own
`ResizeObserver`, the same frame). The width is kept by the session
(`layout_set`, section 8 of `../UI_PARITY.md`), so the window and a
browser share it, and put on the page by `i18n.js` before it is first
shown. Not shown where the page is one column (under 761 px).
