#!/usr/bin/env python3
"""Writes apps/desktop/src/api/i18n/<language>.json: for each catalog the
reference ships (reference/EdgeSlicer/localization/i18n/<language>/*.po),
the translations the desktop app shows, English included.

    tools/make_translations.py            write the tables
    tools/make_translations.py --check    write nothing; fail when a table is not what would be written
    tools/make_translations.py --report   also list every entry left out as broken

The C++ application shows its message ids through gettext, English
included: its build turns each .po into `resources/i18n/<language>/
EdgeSlicer.mo` with `msgfmt --check-format` (CMakeLists.txt,
`gettext_po_to_mo`), and `GUI_App::load_language` loads the one of the
language chosen, `en` too (wxWidgets takes the message ids to be US
English, so English loads `en`). This reads the .po files the way msgfmt
does and keeps what gettext would use:

- the format in full: strings over several lines, C escapes, `msgctxt`
  (a context is kept as gettext keeps it, `context\\u0004msgid`), plurals
  (`msgid_plural`, `msgstr[n]`) with the header's `Plural-Forms`;
- not a fuzzy entry (gettext does not use them), not an obsolete one
  (`#~`), not one left empty, not one that says what its message id says
  (the app shows the message id without a table);
- not an entry whose placeholders do not match its message id's (a `%s`
  or `%1%` lost, added, or of another kind), which would garble a message
  or stop the C++ application on it (boost::format throws): the message
  id is shown instead, and `--report` lists them.

A table is JSON, an entry to a line:

    {"made_by": "...", "language": "de", "nplurals": 2, "plural": "1011...",
     "messages": {
    "msgid": "msgstr",
    "context\\u0004msgid": "msgstr",
     },
     "plurals": {
    "singular msgid": ["form 0", "form 1"],
     }}

`plural` is the header's expression worked out once for n = 0 to 199, a
digit each (the form for n); for a larger n the form is that of
100 + n % 100. That holds for every expression here: they compare n with
numbers under 100 and look at n % 10 and n % 100, which the check below
makes sure of. The page and the session then need no evaluator of C.

Run it again whenever the pin of reference/EdgeSlicer is moved.
"""
import json, os, re, sys

here = os.path.dirname(os.path.abspath(__file__))
root = os.path.join(here, "..")
reference = os.path.join(root, "reference", "EdgeSlicer")
catalogs = os.path.join(reference, "localization", "i18n")
out = os.path.join(root, "apps", "desktop", "src", "api", "i18n")


# --- The .po format ----------------------------------------------------

ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "a": "\a", "b": "\b", "f": "\f", "v": "\v", "\\": "\\", '"': '"', "'": "'", "?": "?"}


# What was found wrong in a file read leniently (the template has a string
# that is never closed).
problems = []


def unquoted(text, where, lenient=False):
    """A quoted .po string, its C escapes read."""
    if len(text) < 2 or text[0] != '"' or text[-1] != '"':
        if not lenient:
            sys.exit(f"{where}: not a quoted string: {text}")
        problems.append(f"{where}: not a quoted string: {text}")
        return ""
    text, found, at = text[1:-1], [], 0
    while at < len(text):
        c = text[at]
        if c != "\\":
            found.append(c)
            at += 1
            continue
        c = text[at + 1]
        if c in ESCAPES:
            found.append(ESCAPES[c])
            at += 2
        elif c == "x":
            digits = re.match(r"[0-9a-fA-F]+", text[at + 2:]).group(0)
            found.append(chr(int(digits, 16)))
            at += 2 + len(digits)
        elif c in "01234567":
            digits = re.match(r"[0-7]{1,3}", text[at + 1:]).group(0)
            found.append(chr(int(digits, 8)))
            at += 1 + len(digits)
        else:
            sys.exit(f"{where}: unknown escape \\{c}")
    return "".join(found)


def entries(path, lenient=False):
    """The entries of a .po file: dicts of `msgctxt`, `msgid`,
    `msgid_plural`, `msgstr` (or `msgstr[n]`), `flags`, `obsolete` and the
    line the entry starts on."""
    found, entry, field = [], None, None

    def finish():
        nonlocal entry
        if entry and "msgid" in entry:
            found.append(entry)
        entry = None

    with open(path, encoding="utf-8") as f:
        lines = f.read().split("\n")
    for number, line in enumerate(lines, 1):
        where = f"{os.path.relpath(path, root)}:{number}"
        line = line.strip()
        obsolete = line.startswith("#~")
        if obsolete:
            line = line[2:].strip()
            # `#~|` is an obsolete entry's previous message id.
            if line.startswith("|"):
                continue
        elif line.startswith("#"):
            if entry and "msgid" in entry:
                finish()
            if entry is None:
                entry = {"flags": set(), "line": number}
            if line.startswith("#,"):
                entry["flags"].update(flag.strip() for flag in line[2:].split(","))
            continue
        if not line:
            finish()
            field = None
            continue
        keyword = re.match(r"(msgctxt|msgid_plural|msgid|msgstr(?:\[\d+\])?)\s+(.*)$", line)
        if keyword:
            name, rest = keyword.groups()
            # A new entry starts at its context, or at its message id when
            # it has none, if the last one was complete.
            if entry is not None and name in ("msgctxt", "msgid") and any(key.startswith("msgstr") for key in entry):
                finish()
            if entry is None:
                entry = {"flags": set(), "line": number}
            entry["obsolete"] = obsolete
            field = name
            entry[field] = unquoted(rest, where, lenient)
        elif line.startswith('"') and field:
            entry[field] += unquoted(line, where, lenient)
        else:
            sys.exit(f"{where}: not .po: {line}")
    finish()
    return found


def header(found):
    """The header's fields (the entry whose message id is empty)."""
    for entry in found:
        if entry["msgid"] == "" and "msgctxt" not in entry:
            fields = {}
            for line in entry.get("msgstr", "").split("\n"):
                name, _, value = line.partition(":")
                if value:
                    fields[name.strip()] = value.strip()
            return fields
    return {}


# --- Plural-Forms --------------------------------------------------------

class Plural:
    """A C expression of n, as Plural-Forms writes it: ?:, ||, &&, ==, !=,
    <, <=, >, >=, +, -, *, /, %, !, (), numbers and n. Read once, then
    worked out for each n; a number n is compared with is kept, to make
    sure the table of 200 holds for every n."""

    def __init__(self, text):
        self.tokens = re.findall(r"\d+|n|\|\||&&|==|!=|<=|>=|[<>?:()+\-*/%!]", text)
        if "".join(self.tokens) != re.sub(r"\s", "", text):
            raise ValueError(f"not a plural expression: {text}")
        self.at = 0
        self.compared = []
        self.tree = self.ternary()
        if self.at != len(self.tokens):
            raise ValueError(f"not a plural expression: {text}")

    def peek(self):
        return self.tokens[self.at] if self.at < len(self.tokens) else None

    def take(self, expected=None):
        token = self.peek()
        if expected is not None and token != expected:
            raise ValueError(f"expected {expected} at {token}")
        self.at += 1
        return token

    def ternary(self):
        condition = self.binary(0)
        if self.peek() == "?":
            self.take()
            yes = self.ternary()
            self.take(":")
            no = self.ternary()
            return ("?", condition, yes, no)
        return condition

    LEVELS = [["||"], ["&&"], ["==", "!="], ["<", "<=", ">", ">="], ["+", "-"], ["*", "/", "%"]]

    def binary(self, level):
        if level == len(self.LEVELS):
            return self.unary()
        left = self.binary(level + 1)
        while self.peek() in self.LEVELS[level]:
            operator = self.take()
            right = self.binary(level + 1)
            # n itself compared with a number (not n % 10).
            if operator in ("==", "!=", "<", "<=", ">", ">="):
                for side, other in ((left, right), (right, left)):
                    if side == "n" and isinstance(other, int):
                        self.compared.append(other)
            left = (operator, left, right)
        return left

    def unary(self):
        token = self.take()
        if token == "!":
            return ("!", self.unary())
        if token == "(":
            inner = self.ternary()
            self.take(")")
            return inner
        if token == "n":
            return "n"
        if token is not None and token.isdigit():
            return int(token)
        raise ValueError(f"unexpected {token}")

    def of(self, n, tree=None):
        tree = self.tree if tree is None else tree
        if tree == "n":
            return n
        if isinstance(tree, int):
            return tree
        operator = tree[0]
        if operator == "?":
            return self.of(n, tree[2]) if self.of(n, tree[1]) else self.of(n, tree[3])
        if operator == "!":
            return int(not self.of(n, tree[1]))
        a = self.of(n, tree[1])
        if operator == "||":
            return int(bool(a) or bool(self.of(n, tree[2])))
        if operator == "&&":
            return int(bool(a) and bool(self.of(n, tree[2])))
        b = self.of(n, tree[2])
        return {
            "==": lambda: int(a == b), "!=": lambda: int(a != b), "<": lambda: int(a < b),
            "<=": lambda: int(a <= b), ">": lambda: int(a > b), ">=": lambda: int(a >= b),
            "+": lambda: a + b, "-": lambda: a - b, "*": lambda: a * b,
            "/": lambda: a // b, "%": lambda: a % b,
        }[operator]()


def plural_forms(fields, where):
    text = fields.get("Plural-Forms", "nplurals=2; plural=(n != 1);")
    found = re.match(r"\s*nplurals\s*=\s*(\d+)\s*;\s*plural\s*=\s*(.*?);?\s*$", text)
    if not found:
        sys.exit(f"{where}: Plural-Forms not read: {text}")
    count, expression = int(found.group(1)), Plural(found.group(2))
    if any(number >= 100 for number in expression.compared):
        sys.exit(f"{where}: Plural-Forms compares n with 100 or more; the table of 200 would not hold")
    forms = [expression.of(n) for n in range(200)]
    # The table stands for every n: check it on a stretch past it.
    for n in range(200, 2000):
        if expression.of(n) != forms[100 + n % 100]:
            sys.exit(f"{where}: Plural-Forms is not periodic in n % 100 past 100")
    if any(form < 0 or form >= count or form > 9 for form in forms):
        sys.exit(f"{where}: Plural-Forms gives a form out of range")
    return count, "".join(str(form) for form in forms)


# --- Placeholders --------------------------------------------------------

PRINTF = re.compile(r"%(?:(\d+)\$)?[-+ #0']*(\d+|\*)?(?:\.(\d+|\*))?(hh|h|ll|l|L|q|j|z|Z|t|I64|I32|I)?([diouxXeEfFgGaAcspn])")
BOOST = re.compile(r"%(\d+)%")
KINDS = {**dict.fromkeys("diouxXc", "int"), **dict.fromkeys("eEfFgGaA", "float"), "s": "text", "p": "pointer", "n": "count"}


def placeholders(text, flags):
    """What a message's arguments go into, as the C++'s formatting reads
    them: boost::format's `%1%`, printf's `%s` or `%1$d` (by the flags
    xgettext gave the entry), and `{…}` (a name of a G-code template, or
    fmt's `{}`). `%%` is a percent sign."""
    found = []
    text = text.replace("%%", "")
    if "boost-format" in flags:
        found += sorted("%" + n + "%" for n in BOOST.findall(text))
        text = BOOST.sub("", text)
    if "c-format" in flags:
        sequence = 0
        for match in PRINTF.finditer(text):
            position, width, precision, _, kind = match.groups()
            if position:
                index = int(position)
            else:
                sequence += 1
                index = sequence
            found.append(f"#{index}:{KINDS[kind]}" + (":*" if width == "*" or precision == "*" else ""))
        found.sort()
    found += sorted(re.findall(r"\{[A-Za-z_0-9]*\}", text))
    return found


# --- The tables ----------------------------------------------------------

def table(language, path):
    found = entries(path)
    fields = header(found)
    where = os.path.relpath(path, root)
    count, plural = plural_forms(fields, where)
    messages, plurals, broken = {}, {}, []
    counted = {"entries": 0, "kept": 0, "empty": 0, "fuzzy": 0, "obsolete": 0, "same": 0, "broken": 0}
    for entry in found:
        if entry["msgid"] == "" and "msgctxt" not in entry:
            continue
        counted["entries"] += 1
        if entry.get("obsolete"):
            counted["obsolete"] += 1
            continue
        if "fuzzy" in entry["flags"]:
            counted["fuzzy"] += 1
            continue
        msgid = entry["msgid"]
        key = entry["msgctxt"] + "\u0004" + msgid if "msgctxt" in entry else msgid
        if "msgid_plural" in entry:
            forms = [entry.get(f"msgstr[{n}]", "") for n in range(count)]
            if not all(forms):
                counted["empty"] += 1
                continue
            sources = [msgid, entry["msgid_plural"]]
            if count == 2 and forms == sources:
                counted["same"] += 1
                continue
            # A form that is not the singular's or the plural's in its
            # placeholders, or two forms run together with "@" between
            # them (an import's leftover in some catalogs).
            wrong = [form for form in forms if placeholders(form, entry["flags"]) != placeholders(entry["msgid_plural"], entry["flags"])
                     and placeholders(form, entry["flags"]) != placeholders(msgid, entry["flags"])
                     or "@" in form and "@" not in msgid]
            if wrong:
                counted["broken"] += 1
                broken.append((entry["line"], msgid, wrong[0]))
                continue
            counted["kept"] += 1
            plurals[key] = forms
            continue
        text = entry.get("msgstr", "")
        if not text:
            counted["empty"] += 1
            continue
        if text == msgid:
            counted["same"] += 1
            continue
        if placeholders(text, entry["flags"]) != placeholders(msgid, entry["flags"]):
            counted["broken"] += 1
            broken.append((entry["line"], msgid, text))
            continue
        counted["kept"] += 1
        messages[key] = text
    return {"language": language, "nplurals": count, "plural": plural, "messages": messages, "plurals": plurals}, counted, broken


def written(language, made):
    """The table as written: JSON, an entry to a line, in the order of
    the message ids, so a change of the catalog reads as a change of its
    lines."""
    line = lambda value: json.dumps(value, ensure_ascii=False, separators=(",", ":"))
    head = {"made_by": f"tools/make_translations.py, from reference/EdgeSlicer localization/i18n/{language}; do not edit",
            "language": language, "nplurals": made["nplurals"], "plural": made["plural"]}
    text = line(head)[:-1] + ',\n"messages":{\n'
    text += ",\n".join(line(key) + ":" + line(value) for key, value in sorted(made["messages"].items()))
    text += '\n},\n"plurals":{\n'
    text += ",\n".join(line(key) + ":" + line(value) for key, value in sorted(made["plurals"].items()))
    return text + "\n}}\n"


def main():
    check = "--check" in sys.argv[1:]
    report = "--report" in sys.argv[1:]
    languages = sorted(name for name in os.listdir(catalogs) if os.path.isdir(os.path.join(catalogs, name)))
    made, stale = {}, []
    for language in languages:
        files = sorted(name for name in os.listdir(os.path.join(catalogs, language)) if name.endswith(".po"))
        if len(files) != 1:
            sys.exit(f"{language}: expected one .po, found {files}")
        path = os.path.join(catalogs, language, files[0])
        result, counted, broken = table(language, path)
        made[language + ".json"] = written(language, result)
        if not check:
            sizes = len(made[language + ".json"].encode("utf-8"))
            print(f"{language}: {counted['kept']} kept of {counted['entries']} ({counted['empty']} empty, {counted['same']} the same as the message id, "
                  f"{counted['fuzzy']} fuzzy, {counted['obsolete']} obsolete, {counted['broken']} broken), {sizes / 1024:.0f} KiB")
        if report:
            for line, msgid, text in broken:
                print(f"  broken {os.path.relpath(path, reference)}:{line}: {msgid!r} -> {text!r}")
    present = sorted(name for name in os.listdir(out) if name.endswith(".json")) if os.path.isdir(out) else []
    if check:
        for name, text in made.items():
            try:
                with open(os.path.join(out, name), encoding="utf-8") as f:
                    if f.read() != text:
                        stale.append(name)
            except FileNotFoundError:
                stale.append(name)
        stale += [name for name in present if name not in made]
        if stale:
            sys.exit(f"not what the reference gives: {', '.join(stale)}; run tools/make_translations.py")
        print("up to date")
        return
    os.makedirs(out, exist_ok=True)
    for name in present:
        if name not in made:
            os.remove(os.path.join(out, name))
    for name, text in made.items():
        with open(os.path.join(out, name), "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
    total = sum(len(text.encode("utf-8")) for text in made.values())
    print(f"{len(made)} tables, {total / 1024:.0f} KiB in all, written to {os.path.normpath(out)}")


main()
