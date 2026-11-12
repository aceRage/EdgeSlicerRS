#!/usr/bin/env python3
"""Writes apps/desktop/src/api/settings_pages.json: the reference's pages
of settings for its three kinds of preset, each a list of groups of
options in the reference's order, and what the reference says of every
option on them: its label, tooltip, kind, unit, least and greatest value,
the values of a list with their labels, its default and the mode it is
shown in.

- process: "Quality", "Strength", "Speed", "Support", "Multimaterial",
  "Others" (`TabPrint::build`);
- filament: "Filament", "Cooling", "Setting Overrides", "Advanced",
  "Multimaterial", "Dependencies", "Notes" (`TabFilament::build` and
  `add_filament_overrides_page`);
- printer: "Basic information", "Machine G-code", "Multimaterial",
  "Extruder %d" (one for each extruder), "Motion ability", "Notes"
  (`TabPrinter::build_fff`, `build_kinematics_page` and
  `build_unregular_pages`, put in the order the last of them leaves them
  in).

    tools/make_settings_pages.py

Nothing is run: the pages are read from src/slic3r/GUI/Tab.cpp, the
options from `PrintConfigDef` (src/libslic3r/PrintConfig.cpp) of the
pinned `reference/EdgeSlicer`, and the defaults from the table
tools/make_defaults.py writes (crates/edge-config/src/reference_defaults.txt),
which is the reference's own account of them. Run this again whenever the
pin is moved, after make_defaults.py. `--check` writes nothing and fails
when the file is not what would be written.
"""
import json, os, re, sys

here = os.path.dirname(os.path.abspath(__file__))
root = os.path.join(here, "..")
reference = os.path.join(root, "reference", "EdgeSlicer")
out = os.path.join(root, "apps", "desktop", "src", "api", "settings_pages.json")
defaults_file = os.path.join(root, "crates", "edge-config", "src", "reference_defaults.txt")


def read(*path):
    with open(os.path.join(reference, *path), encoding="utf-8") as f:
        return f.read()


# --- C++ as far as these two functions need it -----------------------

ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "\\": "\\", '"': '"', "'": "'", "0": "\0"}


def literal(source, at):
    """The string literal that opens at `source[at]` (a quote), and where it ends."""
    text, at = [], at + 1
    while source[at] != '"':
        c = source[at]
        if c == "\\":
            c = source[at + 1]
            if c == "u":
                text.append(chr(int(source[at + 2:at + 6], 16)))
                at += 6
                continue
            if c == "x":
                digits = re.match(r"[0-9a-fA-F]+", source[at + 2:]).group(0)
                text.append(chr(int(digits, 16)))
                at += 2 + len(digits)
                continue
            text.append(ESCAPES.get(c, c))
            at += 2
            continue
        text.append(c)
        at += 1
    return "".join(text), at + 1


def without_comments(source):
    """`source` with its comments blanked, strings left as they are."""
    result, at, count = [], 0, len(source)
    while at < count:
        c = source[at]
        if c == '"':
            _, end = literal(source, at)
            result.append(source[at:end])
            at = end
        elif c == "'" and at + 2 < count:
            end = source.index("'", at + 2 if source[at + 1] == "\\" else at + 1) + 1
            result.append(source[at:end])
            at = end
        elif source.startswith("//", at):
            end = source.find("\n", at)
            at = count if end < 0 else end
        elif source.startswith("/*", at):
            end = source.index("*/", at) + 2
            result.append(" ")
            at = end
        else:
            result.append(c)
            at += 1
    return "".join(result)


def statements(source):
    """The statements of `source`, each up to its `;`. The braces of a
    block end a statement; those of a value (`line = { "a", "b" }`) are
    part of theirs."""
    found, start, at, depth, count = [], 0, 0, 0, len(source)
    values = []
    while at < count:
        c = source[at]
        if c == '"':
            _, at = literal(source, at)
            continue
        if c == "'":
            at = source.index("'", at + 2 if source[at + 1] == "\\" else at + 1) + 1
            continue
        if c in "([":
            depth += 1
        elif c in ")]":
            depth -= 1
        elif c == "{":
            before = source[start:at].rstrip()
            value = depth > 0 or before.endswith(("=", ",", "{")) or bool(values and values[-1])
            values.append(value)
            if value:
                depth += 1
            else:
                found.append(source[start:at].strip())
                start = at + 1
        elif c == "}":
            if values and values.pop():
                depth -= 1
            else:
                found.append(source[start:at].strip())
                start = at + 1
        elif c == ";" and depth == 0:
            found.append(source[start:at].strip())
            start = at + 1
        at += 1
    return [s for s in found if s]


def strings(expression):
    """Every string literal in an expression, in order."""
    found, at = [], 0
    while True:
        at = expression.find('"', at)
        if at < 0:
            return found
        text, at = literal(expression, at)
        found.append(text)


def text_of(expression):
    """What an expression of literals says: `L("a" "b")`, `"mm"`, `u8"mm³"`.
    Literals side by side are one text; none at all is not a text."""
    found = strings(expression)
    if not found:
        return None
    # L_CONTEXT("text", "context"): the text is the first.
    if "L_CONTEXT" in expression:
        return found[0]
    return "".join(found)


def function_body(source, signature):
    """The body of the function whose definition starts with `signature`."""
    start = source.index(signature)
    at = source.index("{", start)
    depth, begin = 0, at
    while True:
        c = source[at]
        if c == '"':
            _, at = literal(source, at)
            continue
        if c == "'":
            at = source.index("'", at + 2 if source[at + 1] == "\\" else at + 1) + 1
            continue
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return source[begin + 1:at]
        at += 1


# --- The options: PrintConfigDef --------------------------------------

KINDS = {
    "coFloat": "float", "coFloats": "float", "coInt": "int", "coInts": "int",
    "coBool": "bool", "coBools": "bool", "coPercent": "percent", "coPercents": "percent",
    "coFloatOrPercent": "float_or_percent", "coFloatsOrPercents": "float_or_percent",
    "coString": "string", "coStrings": "strings", "coEnum": "enum", "coEnums": "enum",
    "coPoint": "point", "coPoints": "points", "coPoint3": "point3", "coNone": "none",
}
# One value for each nozzle or flow variant; the process pages show them as one.
LISTS = {"coFloats", "coInts", "coBools", "coPercents", "coFloatsOrPercents", "coEnums"}
# A value for each filament (a filament preset's lists) or extruder (a
# printer preset's): every kind that is a list of values.
VECTORS = LISTS | {"coStrings", "coPoints"}
MODES = {"comSimple": "simple", "comAdvanced": "advanced", "comDevelop": "developer"}


def number(expression):
    expression = expression.strip().rstrip("fF")
    try:
        value = float(expression)
    except ValueError:
        return None
    return int(value) if value.is_integer() and abs(value) < 1e15 else value


def constant(name):
    """A constant of libslic3r.h, such as MAXIMUM_EXTRUDER_NUMBER."""
    found = re.search(r"\b" + re.escape(name) + r"\s*=\s*([0-9.]+)", read("src", "libslic3r", "libslic3r.h"))
    return number(found.group(1)) if found else None


def described_field(option, name, value, axis=None):
    """Sets what one `def->name = value` says on `option`."""
    if name in ("label", "full_label", "tooltip", "sidetext", "category", "ratio_over"):
        text = text_of(value)
        if text is not None:
            # (boost::format("Maximum speed %1%") % axis_upper).str()
            option[name] = text.replace("%1%", axis.upper()) if axis else text
    elif name in ("min", "max", "max_literal", "height"):
        parsed = number(value)
        if parsed is None and re.fullmatch(r"\s*[A-Z_]+\s*", value):
            parsed = constant(value.strip())
        if parsed is not None:
            option[name] = parsed
    elif name == "mode":
        option["mode"] = MODES[value.strip()]
    elif name in ("multiline", "full_width", "readonly", "is_code"):
        option[name] = value.strip() == "true"
    elif name == "gui_type":
        option["gui_type"] = value.strip().rsplit("::", 1)[-1]
    elif name in ("enum_values", "enum_labels") and value.strip().startswith("{"):
        # def->enum_values = { "Default", "MZV", ... }
        option[name] = strings(value)


def options(source):
    """Every option `PrintConfigDef` defines, by key. The machine limits
    are made in a loop over the axes (`"machine_max_speed_" + axis.name`),
    an option for each axis, `%1%` in their texts being the axis."""
    source = without_comments(source)
    found = {}
    by_variable = {}
    # The axes' names, from the list the loop goes through.
    listed = re.search(r"std::vector<AxisDefault>\s+axes\s*\{(.*?)\};", source, re.S)
    axes = re.findall(r"\{\s*\"(\w+)\"", listed.group(1))
    for statement in statements(source):
        # `def = this->add("key", coFloat)`, or `auto kept = def = this->add(...)`,
        # or `def = this->add("machine_max_speed_" + axis.name, coFloats)`.
        added = re.match(r"(?:auto\s*\*?\s*|ConfigOptionDef\s*\*\s*)?((?:\w+\s*=\s*)+)this->add(_nullable)?\(\s*\"([^\"]+)\"\s*(\+\s*axis\.name\s*)?,\s*(\w+)\s*\)$", statement)
        if added:
            variables, nullable, key, per_axis, kind = added.groups()
            made = []
            for axis in (axes if per_axis else [None]):
                current = {"kind": kind, "mode": "simple"}
                if nullable:
                    current["nullable"] = True
                found[key + axis if axis else key] = current
                made.append((current, axis))
            for variable in re.findall(r"\w+", variables):
                by_variable[variable] = made
            continue
        field = re.match(r"(\w+)->(\w+)(?:\.(push_back|emplace_back|insert)\((.*)\)|\s*=\s*(.*))$", statement, re.S)
        if not field or field.group(1) not in by_variable:
            continue
        for option, axis in by_variable[field.group(1)]:
            name, pushed, value = field.group(2), field.group(4), field.group(5)
            if pushed is not None:
                if name in ("enum_values", "enum_labels"):
                    # `insert(def->enum_values.begin(), "first")` puts it first.
                    at = 0 if field.group(3) == "insert" else len(option.get(name, []))
                    option.setdefault(name, []).insert(at, text_of(pushed))
                continue
            # `def->sidetext = other->sidetext`: the other option's own.
            copied = re.match(r"(\w+)->(\w+)$", value.strip())
            if copied and copied.group(1) in by_variable:
                other = by_variable[copied.group(1)][0][0].get(copied.group(2))
                if other is not None:
                    option[name] = list(other) if isinstance(other, list) else other
                continue
            described_field(option, name, value, axis)
    copied_for_filaments(source, found)
    return found


def copied_for_filaments(source, found):
    """The filament's own retraction settings, made in a loop from the
    printer's (`add_nullable(std::string("filament_") + opt_key, ...)`):
    each takes the printer option's texts, list and limits, may be "nil",
    and is shown in the advanced mode but for the few named with
    `strcmp` in the loop."""
    at = source.index('std::string("filament_") + opt_key')
    head = source.rindex("for (const char *opt_key : {", 0, at)
    keys = strings(source[head:source.index("})", head)])
    body = function_body(source[source.index("})", head):], ")")
    simple = set(re.findall(r'strcmp\(\s*opt_key\s*,\s*"(\w+)"\s*\)', body))
    for key in keys:
        base = found[key]
        option = {"kind": base["kind"], "nullable": True, "mode": "simple" if key in simple else "advanced"}
        for name in ("label", "full_label", "tooltip", "sidetext", "enum_values", "enum_labels", "min", "max"):
            if name in base:
                option[name] = list(base[name]) if isinstance(base[name], list) else base[name]
        found["filament_" + key] = option


# --- The pages: Tab.cpp -----------------------------------------------

class Reader:
    """Reads the pages a function of Tab.cpp builds, statement by
    statement, into `pages`: groups of lines of option keys.

    What it follows: `add_options_page`, `new_optgroup`,
    `append_single_option_line("key")`; an `Option` shaped before it is
    appended (`is_code`, `multiline`, `full_width`, `height`, `label`); a
    `Line` of several options with its own label and tooltip; a line with a
    widget of the reference's own in place of a field
    (`create_line_with_widget`, whose editor is a dialog); a `for` over a
    list of keys handed to a helper (the filament's override pages); the
    printer's `append_option_line` (an option with a value for the normal
    and the silent mode), in a loop over a list of axes or not; an option
    the tab defines for itself (`ConfigOptionDef def; ... Option option(def,
    "extruders_count")`). Commented-out lines are not read. A group that
    holds only widgets of the reference's own has no options and is left
    out, as is a line without one."""

    def __init__(self, source):
        # Without its comments, whose apostrophes would read as the start
        # of a character.
        self.source = without_comments(source)
        self.pages = []
        self.group = None
        self.lines = {}        # variable -> a line being made
        self.shaped = {}       # variable -> { key, shape }
        self.texts = {}        # variable -> a text (`page_name`)
        self.vectors = {}      # variable -> a list of texts
        self.loop = (None, [])
        self.made = {}         # an option the tab defines itself, by key
        self.numbers = {}      # a whole number named in the function (`gcode_field_height`)
        self.definition = None

    def add(self, keys, label=None, tooltip=None, **more):
        if not keys or self.group is None:
            return
        entry = {"keys": keys}
        if label:
            entry["label"] = label
        if tooltip:
            entry["tooltip"] = tooltip
        entry.update({name: value for name, value in more.items() if value})
        self.group["lines"].append(entry)

    def read(self, signature, helpers=None):
        helpers = helpers or {}
        body = function_body(self.source, signature)
        # A list of keys (`const std::vector<std::string> speed_axes{ ... }`)
        # reads as a block of statements; its texts are taken whole.
        for name, items in re.findall(r"std::vector<std::string>\s+(\w+)\s*\{([^}]*)\}", body):
            self.vectors[name] = strings(items)
        for statement in statements(body):
            for call, helper in helpers.items():
                if re.search(call, statement):
                    self.read(helper, helpers)
                    break
            else:
                self.statement(statement)
        return self

    def statement(self, statement):
        counted = re.match(r"const\s+int\s+(\w+)\s*=\s*([0-9]+)$", statement)
        if counted:
            self.numbers[counted.group(1)] = int(counted.group(2))
            return
        named = re.match(r"(?:const\s+)?wxString\s*&?\s*(\w+)\s*=\s*wxString::Format\(\s*_L\(", statement)
        if named:
            self.texts[named.group(1)] = strings(statement)[0]
            return
        page = re.search(r"add_options_page\(\s*(L\(|\w+\s*,)", statement)
        if page:
            title = strings(statement)[0] if page.group(1).startswith("L") else self.texts[page.group(1).rstrip(", ")]
            entry = {"title": title, "groups": []}
            if "%d" in title:
                # One page for each extruder (Tab.cpp `build_unregular_pages`).
                entry["per"] = "extruder"
            self.pages.append(entry)
            self.group = None
            return
        if re.search(r"new_optgroup\(", statement):
            texts = strings(statement)
            self.group = {"title": texts[0] if texts else "", "lines": []}
            self.pages[-1]["groups"].append(self.group)
            return
        if self.group is None:
            return
        loop = re.match(r"for\s*\(\s*const\s+std::string\s*&?\s*(\w+)\s*:\s*(\w+)\s*\)$", statement)
        if loop:
            self.loop = (loop.group(1), self.vectors.get(loop.group(2), []))
            return
        # for (const std::string opt_key : { "a", "b" }) append_ironing_option(opt_key);
        over = re.match(r"for\s*\(\s*const\s+std::string\s*&?\s*(\w+)\s*:\s*\{(.*)\}\s*\)\s*(\w+)\(\s*\1\s*\)$", statement, re.S)
        if over:
            # The retraction overrides fall back to the printer's value,
            # the ironing ones to the process's (Tab.cpp, the two helpers
            # of `add_filament_overrides_page`).
            fallback = "process" if "ironing" in over.group(3) else "printer"
            for key in strings(over.group(2)):
                self.add([key], override=fallback)
            return
        # append_option_line(optgroup, "machine_max_acceleration_" + axis)
        limits = re.match(r"append_option_line\(\s*optgroup\s*,\s*(.*)\)$", statement, re.S)
        if limits:
            argument = limits.group(1).strip()
            prefix = strings(argument)
            # The loop's variable, alone or after a "prefix" +.
            variable = argument.rsplit("+", 1)[-1].strip()
            if prefix and "+" in argument:
                keys = [prefix[0] + value for value in self.loop[1]] if variable == self.loop[0] else []
            elif prefix:
                keys = [prefix[0]]
            else:
                keys = list(self.loop[1]) if variable == self.loop[0] else []
            for key in keys:
                self.add([key], modes=True)
            return
        # An option the tab defines itself.
        if re.match(r"ConfigOptionDef\s+def$", statement):
            self.definition = {"mode": "simple"}
            return
        definition = re.match(r"def\.(\w+)\s*=\s*(.*)$", statement, re.S)
        if definition and self.definition is not None:
            name, value = definition.groups()
            if name == "type":
                self.definition["kind"] = value.split(",")[0].strip()
            else:
                described_field(self.definition, name, value)
            return
        made = re.match(r"(?:Option\s+|auto\s+)?(\w+)\s*(?:=\s*Option)?\(\s*def\s*,\s*\"([^\"]+)\"\s*\)$", statement)
        if made and self.definition is not None:
            if self.definition.get("gui_type") != "legend":
                self.made[made.group(2)] = dict(self.definition)
                self.shaped[made.group(1)] = {"key": made.group(2), "shape": {}}
            else:
                self.shaped.pop(made.group(1), None)
            return
        if re.search(r"append_single_option_line\(\s*\"", statement):
            self.add([strings(statement)[0]])
            return
        # `option = optgroup->get_option("key")`, shaped by what follows
        # it, then appended as a line of its own or put in a line.
        got = re.match(r"(?:Option\s+|auto\s+)?(\w+)\s*=\s*\w+->get_option\(\s*\"([^\"]+)\"\s*\)$", statement)
        if got:
            self.shaped[got.group(1)] = {"key": got.group(2), "shape": {}}
            return
        shape = re.match(r"(\w+)\.opt\.(\w+)\s*=\s*(.*)$", statement, re.S)
        if shape and shape.group(1) in self.shaped:
            name, value = shape.group(2), shape.group(3).strip()
            if name == "label":
                value = text_of(value)
            elif value in ("true", "false"):
                value = value == "true"
            else:
                value = self.numbers.get(value.split(";")[0].strip(), number(value))
            self.shaped[shape.group(1)]["shape"][name] = value
            return
        appended = re.search(r"append_single_option_line\(\s*(\w+)\s*[,)]", statement)
        if appended and appended.group(1) in self.shaped:
            shaped = self.shaped[appended.group(1)]
            self.add([shaped["key"]], shape=dict(shaped["shape"]))
            return
        # A line of several options under one label.
        started = re.match(r"(?:Line\s+|auto\s+)?(\w*line)\s*=\s*(?:Line\s*)?\{(.*)$", statement, re.S)
        if started:
            texts = strings(started.group(2))
            line = {"label": texts[0] if texts else None, "tooltip": texts[1] if len(texts) > 1 else None, "keys": [], "labels": {}}
            # Line{ L("Fan speed-up time"), optgroup->get_option("fan_speedup_time").opt.tooltip }
            borrowed = re.search(r"get_option\(\s*\"([^\"]+)\"\s*\)\.opt\.tooltip", started.group(2))
            if borrowed:
                line["tooltip"] = None
                line["tooltip_of"] = borrowed.group(1)
            self.lines[started.group(1)] = line
            return
        put = re.match(r"(\w+)\.append_option\((.*)\)$", statement, re.S)
        if put and put.group(1) in self.lines:
            line = self.lines[put.group(1)]
            argument = put.group(2).strip()
            if argument in self.shaped:
                shaped = self.shaped[argument]
                line["keys"].append(shaped["key"])
                if shaped["shape"].get("label"):
                    line["labels"][shaped["key"]] = shaped["shape"]["label"]
            elif strings(argument):
                line["keys"].append(strings(argument)[0])
            return
        ended = re.match(r"\w+->append_line\(\s*(\w+)\s*\)$", statement)
        if ended and ended.group(1) in self.lines:
            line = self.lines.pop(ended.group(1))
            self.add(line["keys"], line["label"], line["tooltip"], tooltip_of=line.get("tooltip_of"), labels=line["labels"])
            return
        # A widget of the reference's own in the option's place: its
        # editor is a dialog of its own (the bed shape, the ramming, the
        # compatible printers).
        widget = re.match(r"create_line_with_widget\(\s*optgroup\.get\(\)\s*,\s*\"([^\"]+)\"", statement)
        if widget:
            self.add([widget.group(1)], widget=True)

    def finish(self):
        # A group nothing was put in (one that holds a widget of the
        # reference's own) is not a group of options.
        for page in self.pages:
            page["groups"] = [group for group in page["groups"] if group["lines"]]
        return self.pages


def process_pages(tab):
    return Reader(tab).read("void TabPrint::build()").finish()


def filament_pages(tab):
    reader = Reader(tab).read(
        "void TabFilament::build()",
        {r"add_filament_overrides_page\(\)": "PageShp TabFilament::add_filament_overrides_page()"},
    )
    return reader.finish(), reader.made


def printer_pages(tab, config):
    basic = Reader(tab).read("void TabPrinter::build_fff()")
    motion = Reader(tab).read("PageShp TabPrinter::build_kinematics_page()")
    unregular = Reader(tab).read("void TabPrinter::build_unregular_pages(bool from_initial_build )")
    made = {**basic.made, **motion.made, **unregular.made}
    pages = basic.finish()
    [kinematics] = motion.finish()
    multimaterial, extruder = unregular.finish()
    # The flavors "Motion ability" is shown for (`is_marlin_flavor`).
    body = function_body(unregular.source, "void TabPrinter::build_unregular_pages(bool from_initial_build )")
    names = re.findall(r"\b(gcf\w+)", re.search(r"is_marlin_flavor\s*=\s*\(([^;]*)\)", body).group(1))
    flavors = dict((value, key) for key, value in re.findall(r"\{\s*\"([\w-]+)\"\s*,\s*(gcf\w+)\s*\}", config))
    kinematics["flavors"] = [flavors[name] for name in names]
    # `build_unregular_pages` puts its pages among build_fff's three
    # ("Basic information", "Machine G-code", "Notes"): "Motion ability"
    # after the two before the extruders, then "Multimaterial" two from
    # the end, then the extruders after the pages before them, which with
    # "Motion ability" in are three. For a flavor without "Motion
    # ability" the same steps put "Multimaterial" and the extruders before
    # "Machine G-code"; the pages keep the first order for every printer.
    before = 2
    pages.insert(before, kinematics)
    before += 1
    pages.insert(len(pages) - 2, multimaterial)
    pages.insert(before, extruder)
    return pages, made


# --- The defaults: the table make_defaults.py writes -------------------

def defaults():
    found = {}
    with open(defaults_file, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line[0] in "#?":
                continue
            key, _, value = line.partition(" =")
            found[key] = value[1:] if value.startswith(" ") else value
    return found


def unescaped(text):
    return re.sub(r"\\(.)", lambda found: "\n" if found.group(1) == "n" else found.group(1), text)


def unquoted(text, several):
    """A default as the table has it, out of its quotes and escapes. One
    that is several texts (joined by semicolons, those with spaces in
    quotes) is given a line each, as the pages show it."""
    if not several:
        if len(text) >= 2 and text[0] == '"' and text[-1] == '"':
            text = text[1:-1]
        return unescaped(text)
    items = re.findall(r'"((?:[^"\\]|\\.)*)"|([^;"]+)', text)
    lines = [unescaped(quoted or bare).strip("\n") for quoted, bare in items]
    return "\n".join(line for line in lines if line)


def book(layout, described, table, made, kind):
    """The pages of one kind of preset and every option on them."""
    listed, missing, undefaulted = {}, [], []
    for page in layout:
        for group in page["groups"]:
            for line in group["lines"]:
                shape = line.pop("shape", {})
                override = line.pop("override", None)
                widget = line.pop("widget", False)
                modes = line.pop("modes", False)
                if modes:
                    # The row is named in full ("Maximum speed X").
                    line["full"] = True
                borrowed = line.pop("tooltip_of", None)
                if borrowed:
                    line["tooltip"] = described[borrowed]["tooltip"]
                for key in line["keys"]:
                    source = described.get(key) or made.get(key)
                    if source is None:
                        missing.append(key)
                        continue
                    option = {"label": source.get("label") or source.get("full_label", ""), "type": KINDS[source["kind"]]}
                    if source.get("full_label"):
                        option["full_label"] = source["full_label"]
                    if source.get("tooltip"):
                        option["tooltip"] = source["tooltip"]
                    if source.get("sidetext"):
                        option["unit"] = source["sidetext"]
                    for name in ("min", "max", "max_literal"):
                        if name in source:
                            option[name] = source[name]
                    if source.get("ratio_over"):
                        option["ratio_over"] = source["ratio_over"]
                    if "enum_values" in source:
                        option["values"] = source["enum_values"]
                        labels = source.get("enum_labels", [])
                        option["labels"] = labels if len(labels) == len(source["enum_values"]) else source["enum_values"]
                    # A list that takes a value typed in as well as one picked.
                    if source.get("gui_type", "").endswith("_open"):
                        option["open"] = True
                    option["mode"] = source["mode"]
                    if source["kind"] in LISTS:
                        option["list"] = True
                    if kind != "process" and source["kind"] in VECTORS:
                        # A value for each filament, or each extruder.
                        option["vector"] = True
                    if source.get("nullable"):
                        option["nullable"] = True
                    if source.get("multiline") or shape.get("multiline"):
                        option["multiline"] = True
                    if shape.get("is_code"):
                        option["code"] = True
                    if source.get("full_width") or shape.get("full_width"):
                        option["full_width"] = True
                    height = shape.get("height", source.get("height"))
                    if height:
                        option["height"] = height
                    if source.get("readonly"):
                        option["readonly"] = True
                    if override:
                        # Off ("nil"), the filament takes the printer's
                        # (or the process's) value of the option without
                        # "filament_" in its name.
                        option["override"] = override
                        option["of"] = key[len("filament_"):]
                    if widget:
                        option["widget"] = True
                    if modes:
                        # A value for the normal mode, then the silent.
                        option["modes"] = True
                    if key in made:
                        # The tab's own: no setting of a preset.
                        option["own"] = True
                    elif key in table and kind != "process" and source["kind"] in VECTORS - {"coPoints"}:
                        # A filament's, or an extruder's, own: the first.
                        several = unquoted(table[key], True) if source["kind"] == "coStrings" else table[key]
                        option["default"] = (several.split("\n") if source["kind"] == "coStrings" else several.split(","))[0]
                    elif key in table:
                        option["default"] = unquoted(table[key], source["kind"] == "coStrings")
                    else:
                        undefaulted.append(key)
                    listed[key] = option
    if missing:
        sys.exit(f"the {kind} pages name options PrintConfigDef does not define: " + ", ".join(missing))
    return {"pages": layout, "options": listed}, undefaulted


# --- What may be set on an object, a part or a height range -------------

def config_class_keys(header, name):
    """The options of one `PRINT_CONFIG_CLASS_DEFINE` of PrintConfig.hpp, in
    order: what `PrintObjectConfig().keys()` and the like give."""
    start = re.search(r"PRINT_CONFIG_CLASS_DEFINE\(\s*" + name + r"\s*,", header).end()
    end = header.find("PRINT_CONFIG_CLASS_", start)
    body = without_comments(header[start:end if end >= 0 else len(header)])
    return re.findall(r"\(\(\s*ConfigOption[\w<>:]*\s*,\s*(\w+)\s*\)\)", body)


def item_keys(pages, tab, factories, preset, header):
    """The options an object, a part and a height range may set, as the
    reference's per-item tabs offer them (Tab.cpp `TabPrintObject`,
    `TabPrintPart`, `TabPrintLayer`: `Preset::print_options()` met with
    PrintObjectConfig and PrintRegionConfig, PrintRegionConfig and
    `part_support_keys()`, and `layer_height` with PrintRegionConfig), each
    in the order of the process pages, which are what they are drawn from;
    and the "Frequent" page `TabPrintModel::build` puts first."""
    body = preset[preset.index("s_Preset_print_options {"):]
    body = without_comments(body[:body.index("};")])
    offered = set(strings(body))
    region = config_class_keys(header, "PrintRegionConfig")
    obj = config_class_keys(header, "PrintObjectConfig")
    part_support = strings(without_comments(function_body(factories, "const std::vector<std::string>& part_support_keys()")))
    on_pages = [key for page in pages for group in page["groups"] for line in group["lines"] for key in line["keys"]]
    kinds = {
        "object": set(obj) | set(region),
        "part": set(region) | set(part_support),
        "range": {"layer_height"} | set(region),
    }
    model = function_body(tab, "void TabPrintModel::build()")
    frequent = re.search(r'add_options_page\(\s*L\("([^"]+)"\)', model).group(1)
    frequent_keys = re.findall(r'append_single_option_line\(\s*"(\w+)"', model)
    made = {kind: [key for key in on_pages if key in keys and key in offered] for kind, keys in kinds.items()}
    made["frequent"] = {"title": frequent, "keys": frequent_keys}
    return made


def main():
    config = read("src", "libslic3r", "PrintConfig.cpp")
    described = options(config)
    tab = read("src", "slic3r", "GUI", "Tab.cpp")
    table = defaults()
    filament, filament_made = filament_pages(tab)
    printer, printer_made = printer_pages(tab, config)
    made = {"made_by": "tools/make_settings_pages.py, from reference/EdgeSlicer; do not edit"}
    undefaulted = []
    for kind, layout, own in (("process", process_pages(tab), {}), ("filament", filament, filament_made), ("printer", printer, printer_made)):
        made[kind], missing = book(layout, described, table, own, kind)
        undefaulted += [f"{kind} {key}" for key in missing]
    made["process"]["items"] = item_keys(
        made["process"]["pages"], tab, config, read("src", "libslic3r", "Preset.cpp"),
        read("src", "libslic3r", "PrintConfig.hpp"))
    text =json.dumps(made, ensure_ascii=False, indent=1) + "\n"
    if "--check" in sys.argv[1:]:
        with open(out, encoding="utf-8") as f:
            if f.read() != text:
                sys.exit(f"{os.path.normpath(out)} is not what the reference gives; run tools/make_settings_pages.py")
        print("up to date")
        return
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    for kind in ("process", "filament", "printer"):
        pages = made[kind]["pages"]
        counts = ", ".join(f"{page['title']} {sum(len(line['keys']) for group in page['groups'] for line in group['lines'])}" for page in pages)
        print(f"{kind}: {len(made[kind]['options'])} options on {len(pages)} pages ({counts})")
    print(f"written to {os.path.normpath(out)}")
    if undefaulted:
        print("without a default in the table: " + ", ".join(undefaulted))


main()
