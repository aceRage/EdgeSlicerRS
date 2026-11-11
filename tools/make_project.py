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
# PrintConfig.cpp filament_flow_variant_options: lists a filament preset may
# hold one entry of per flow variant, in its filament_flow_support's order.
FLOW_VARIANT = {
    "filament_flow_ratio", "enable_pressure_advance", "pressure_advance",
    "nozzle_temperature_initial_layer", "nozzle_temperature", "filament_max_volumetric_speed",
    "fan_min_speed", "fan_max_speed", "additional_cooling_fan_speed", "filament_retraction_length",
    "filament_retraction_speed", "filament_deretraction_speed", "filament_z_hop_types",
    "filament_wipe_distance", "filament_retract_length_toolchange", "filament_multitool_ramming",
    "filament_multitool_ramming_volume", "filament_multitool_ramming_flow",
    "filament_minimal_purge_on_wipe_tower", "filament_flush_volumetric_speed", "filament_flush_temp",
    "filament_flush_temp_fast", "filament_cooling_before_tower",
}

def standard_entry(key, values, filament):
    """The entry a filament printed at standard flow reads, as the engine's
    edge-config `variants::for_filaments` takes it: a flow-variant list at
    the place of "standard" among its variants, any other list its first."""
    if key not in FLOW_VARIANT:
        return values[0]
    support = filament.get("filament_flow_support") or ["standard"]
    names = filament.get("filament_extruder_variant") or []
    if (support == ["standard"] and len(names) >= 2 and names[0].endswith(" Standard")
            and names[1] == names[0][:-len(" Standard")] + " High Flow"
            and len(filament.get("filament_max_volumetric_speed") or []) >= 2):
        support = ["standard", "high_flow"]
    at = support.index("standard") if "standard" in support else 0
    return values[at] if at < len(values) else values[0]

# A filament preset's lists hold one filament, some of them a value for each
# flow variant; the project's hold one entry for each filament, in order: the
# one a filament printed at standard flow reads. What is not a list is the
# first filament's.
presets = [preset("filament", name) for name, *_ in filaments]
for key, value in presets[0].items():
    if not isinstance(value, list):
        settings[key] = value
for key in sorted({key for filament in presets for key, value in filament.items() if isinstance(value, list)}):
    if key in ("compatible_printers", "compatible_prints") or isinstance(presets[0].get(key, []), str):
        continue
    if any(not filament.get(key) for filament in presets):
        print(f"{key}: not in every filament preset, left to the slicers' default", file=sys.stderr)
        continue
    settings[key] = ["standard" if key == "filament_flow_support" else standard_entry(key, filament[key], filament)
                     for filament in presets]
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
