#!/usr/bin/env python3
"""Writes apps/desktop/src/api/process_pages.json: the pages of the
reference's process settings ("Quality", "Strength", "Speed", "Support",
"Multimaterial", "Others"), each a list of groups of options in the
reference's order, and what the reference says of every option on them:
its label, tooltip, kind, unit, least and greatest value, the values of a
list with their labels, its default and the mode it is shown in.

    tools/make_process_pages.py

Nothing is run: the pages are read from `TabPrint::build`
(src/slic3r/GUI/Tab.cpp), the options from `PrintConfigDef`
(src/libslic3r/PrintConfig.cpp) of the pinned `reference/EdgeSlicer`, and
the defaults from the table tools/make_defaults.py writes
(crates/edge-config/src/reference_defaults.txt), which is the reference's
own account of them. Run this again whenever the pin is moved, after
make_defaults.py. `--check` writes nothing and fails when the file is not
what would be written.
"""
import json, os, re, sys

here = os.path.dirname(os.path.abspath(__file__))
root = os.path.join(here, "..")
reference = os.path.join(root, "reference", "EdgeSlicer")
out = os.path.join(root, "apps", "desktop", "src", "api", "process_pages.json")
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
# One value for each nozzle or flow variant; the pages show them as one.
LISTS = {"coFloats", "coInts", "coBools", "coPercents", "coFloatsOrPercents", "coEnums"}
MODES = {"comSimple": "simple", "comAdvanced": "advanced", "comDevelop": "developer"}


def number(expression):
    expression = expression.strip().rstrip("fF")
    try:
        value = float(expression)
    except ValueError:
        return None
    return int(value) if value.is_integer() and abs(value) < 1e15 else value


def options(source):
    """Every option `PrintConfigDef` defines, by key."""
    source = without_comments(source)
    found = {}
    current = None
    by_variable = {}
    for statement in statements(source):
        # `def = this->add("key", coFloat)`, or `auto kept = def = this->add(...)`.
        added = re.match(r"(?:auto\s*\*?\s*|ConfigOptionDef\s*\*\s*)?((?:\w+\s*=\s*)+)this->add(_nullable)?\(\s*\"([^\"]+)\"\s*,\s*(\w+)\s*\)$", statement)
        if added:
            variables, nullable, key, kind = added.groups()
            current = {"kind": kind, "mode": "simple"}
            if nullable:
                current["nullable"] = True
            found[key] = current
            for variable in re.findall(r"\w+", variables):
                by_variable[variable] = current
            continue
        field = re.match(r"(\w+)->(\w+)(?:\.(push_back|emplace_back|insert)\((.*)\)|\s*=\s*(.*))$", statement, re.S)
        if not field or field.group(1) not in by_variable:
            continue
        option = by_variable[field.group(1)]
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
            other = by_variable[copied.group(1)].get(copied.group(2))
            if other is not None:
                option[name] = list(other) if isinstance(other, list) else other
            continue
        if name in ("label", "full_label", "tooltip", "sidetext", "category", "ratio_over"):
            text = text_of(value)
            if text is not None:
                option[name] = text
        elif name in ("min", "max", "max_literal", "height"):
            parsed = number(value)
            if parsed is not None:
                option[name] = parsed
        elif name == "mode":
            option["mode"] = MODES[value.strip()]
        elif name in ("multiline", "full_width", "readonly", "is_code"):
            option[name] = value.strip() == "true"
        elif name == "gui_type":
            option["gui_type"] = value.strip().rsplit("::", 1)[-1]
    return found


# --- The pages: TabPrint::build ---------------------------------------

def pages(source):
    """The pages of the process tab: groups of lines of option keys."""
    body = without_comments(function_body(source, "void TabPrint::build()"))
    found, group, line, shaped = [], None, None, {}

    def add(keys, label=None, tooltip=None):
        entry = {"keys": keys}
        if label:
            entry["label"] = label
        if tooltip:
            entry["tooltip"] = tooltip
        group["lines"].append(entry)

    for statement in statements(body):
        page = re.search(r"add_options_page\(\s*L\(", statement)
        if page:
            found.append({"title": strings(statement)[0], "groups": []})
            group = None
            continue
        opened = re.search(r"new_optgroup\(", statement)
        if opened:
            group = {"title": strings(statement)[0], "lines": []}
            found[-1]["groups"].append(group)
            continue
        if group is None:
            continue
        single = re.search(r"append_single_option_line\(\s*\"", statement)
        if single:
            add([strings(statement)[0]])
            continue
        # `option = optgroup->get_option("key")`, shaped by what follows
        # it, then appended as a line of its own.
        got = re.match(r"(?:Option\s+)?option\s*=\s*optgroup->get_option\(", statement)
        if got:
            shaped = {"key": strings(statement)[0], "shape": {}}
            continue
        shape = re.match(r"option\.opt\.(\w+)\s*=\s*(.*)$", statement)
        if shape and shaped:
            name, value = shape.groups()
            shaped["shape"][name] = value.strip() == "true" if value.strip() in ("true", "false") else number(value)
            continue
        if re.search(r"append_single_option_line\(\s*option\b", statement) and shaped:
            add([shaped["key"]])
            group["lines"][-1]["shape"] = shaped["shape"]
            shaped = {}
            continue
        # A line of several options under one label.
        started = re.match(r"(?:Line\s+)?line\s*=\s*\{(.*)$", statement, re.S)
        if started:
            texts = strings(statement)
            line = {"label": texts[0] if texts else None, "tooltip": texts[1] if len(texts) > 1 else None, "keys": []}
            continue
        if re.match(r"line\.append_option\(", statement) and line is not None:
            line["keys"].append(strings(statement)[0])
            continue
        if re.match(r"optgroup->append_line\(\s*line\s*\)", statement) and line is not None:
            add(line["keys"], line["label"], line["tooltip"])
            line = None
    # A group nothing was put in (one that holds a widget of the
    # reference's own, the support sets) is not a group of options.
    for page in found:
        page["groups"] = [group for group in page["groups"] if group["lines"]]
    return found


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


def main():
    described = options(read("src", "libslic3r", "PrintConfig.cpp"))
    layout = pages(read("src", "slic3r", "GUI", "Tab.cpp"))
    table = defaults()
    listed, missing, undefaulted = {}, [], []
    for page in layout:
        for group in page["groups"]:
            for line in group["lines"]:
                shape = line.pop("shape", {})
                for key in line["keys"]:
                    source = described.get(key)
                    if source is None:
                        missing.append(key)
                        continue
                    option = {"label": source.get("label", ""), "type": KINDS[source["kind"]]}
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
                    if key in table:
                        option["default"] = unquoted(table[key], source["kind"] == "coStrings")
                    else:
                        undefaulted.append(key)
                    listed[key] = option
    if missing:
        sys.exit("TabPrint::build names options PrintConfigDef does not define: " + ", ".join(missing))
    made = {
        "made_by": "tools/make_process_pages.py, from reference/EdgeSlicer; do not edit",
        "pages": layout,
        "options": listed,
    }
    text = json.dumps(made, ensure_ascii=False, indent=1) + "\n"
    if "--check" in sys.argv[1:]:
        with open(out, encoding="utf-8") as f:
            if f.read() != text:
                sys.exit(f"{os.path.normpath(out)} is not what the reference gives; run tools/make_process_pages.py")
        print("up to date")
        return
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    counts = ", ".join(f"{page['title']} {sum(len(line['keys']) for group in page['groups'] for line in group['lines'])}" for page in layout)
    print(f"{len(listed)} options on {len(layout)} pages ({counts}) written to {os.path.normpath(out)}")
    if undefaulted:
        print("without a default in the table: " + ", ".join(undefaulted))


main()
