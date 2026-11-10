#!/usr/bin/env python3
"""Writes a test 3MF whose settings are named presets, as a project saved
from the reference with those presets chosen would have them.

    tools/make_project.py OUT.3mf VENDOR MACHINE PROCESS BED_TYPE SHAPE FILAMENT[=COLOUR] [FILAMENT[=COLOUR] ...] [-- key=value ...]

    tools/make_project.py /tmp/twotone_u1.3mf Snapmaker "Snapmaker U1 (0.4 nozzle)" \
        "0.20mm Standard @Snapmaker U1 (0.4 nozzle)" "Textured PEI Plate" twotone \
        "Generic PLA @U1 0.4 nozzle=#FFFFFF" "Generic PLA @U1 0.4 nozzle=#202020"

The shape is one of tools/make_fixture.py's. Each filament preset named
becomes the next filament of the project, so a shape printed in two
filaments needs two. Needs a built `edge` (cargo build --release).
"""
import json, os, subprocess, sys, tempfile

here = os.path.dirname(os.path.abspath(__file__))
root = os.path.join(here, "..")
edge = os.path.join(root, "target", "release", "edge")
profiles = os.path.join(root, "reference", "EdgeSlicer", "resources", "profiles")
# What names a preset rather than setting anything.
NOT_SETTINGS = {"name", "from", "inherits", "version", "setting_id", "filament_id", "instantiation", "type"}

args = sys.argv[1:]
changes = []
if "--" in args:
    at = args.index("--")
    args, changes = args[:at], args[at + 1:]
out, vendor, machine, process, bed, shape = args[:6]
filaments = [arg.split("=", 1) + [None] for arg in args[6:]]
if not filaments:
    sys.exit("name at least one filament preset")

def preset(kind, name):
    text = subprocess.run([edge, "preset", vendor, kind, name, "--profiles", profiles],
                          capture_output=True, text=True, check=True).stdout
    return {k: v for k, v in json.loads(text[text.index("{"):]).items() if k not in NOT_SETTINGS}

settings = preset("machine", machine)
# A filament's settings are lists with one entry; the project's have one
# entry for each filament, in order.
for index, (name, *_) in enumerate(filaments):
    for key, value in preset("filament", name).items():
        if index == 0:
            settings[key] = value
        elif isinstance(value, list) and isinstance(settings.get(key), list):
            settings[key] = settings[key] + value
settings.update(preset("process", process))
settings["curr_bed_type"] = bed
settings["printer_settings_id"] = machine
settings["print_settings_id"] = process
settings["filament_settings_id"] = [name for name, *_ in filaments]
colours = [colour for _, colour, *_ in filaments]
if any(colours):
    settings["filament_colour"] = [colour or "#FFFFFF" for colour in colours]

with tempfile.TemporaryDirectory() as work:
    source = os.path.join(work, "settings.3mf")
    import zipfile
    with zipfile.ZipFile(source, "w") as z:
        z.writestr("Metadata/project_settings.config", json.dumps(settings, indent=4))
    subprocess.run([sys.executable, os.path.join(here, "make_fixture.py"), out, source, shape] + changes, check=True)
print(f"{out}: {len(settings)} settings, {len(filaments)} filament(s)")
