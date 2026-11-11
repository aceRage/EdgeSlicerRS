#!/usr/bin/env python3
"""Writes a small 3MF to compare this engine with the reference slicer on.

The part is a few boxes in one object, placed where the reference's arranger
puts a single part on a 270 mm bed, so both slicers print it in the same
place. Its settings are those of a project in the reference repository, with
any changes given on the command line.

    tools/make_fixture.py OUT.3mf SETTINGS_FROM.3mf SHAPE [key=value ...]

    tools/make_fixture.py /tmp/mushroom.3mf \
        reference/EdgeSlicer/tests/data/support_corpus/twopart_groups.3mf \
        mushroom enable_support=1 "support_type=normal(auto)"
    target/release/edge-parity /tmp/mushroom.3mf

Shapes: mushroom and shelf need supports; tower and bar get an automatic
brim; bridge is a deck across two posts; cube is a plain 20 mm cube;
twotone is two blocks side by side, the first in filament 1 and 6 mm tall,
the second in filament 2 and 4 mm tall, so there is a tool change on every
layer up to 4 mm and none above; comb is a spine with teeth 0.5 to 2 mm
wide standing on the bed, and twocomb the same with the teeth in filament 2,
for the first layer's elephant foot compensation. The reference loads the filament preset named in the settings again,
so a change to a filament's own settings, such as filament_type, is undone.
"""
import json, sys, zipfile

def box(x0, y0, z0, dx, dy, dz):
    v = [(x0, y0, z0), (x0 + dx, y0, z0), (x0 + dx, y0 + dy, z0), (x0, y0 + dy, z0),
         (x0, y0, z0 + dz), (x0 + dx, y0, z0 + dz), (x0 + dx, y0 + dy, z0 + dz), (x0, y0 + dy, z0 + dz)]
    t = [(0, 2, 1), (0, 3, 2), (4, 5, 6), (4, 6, 7), (0, 1, 5), (0, 5, 4),
         (1, 2, 6), (1, 6, 5), (2, 3, 7), (2, 7, 6), (3, 0, 4), (3, 4, 7)]
    return v, t

SHAPES = {
    # Each is centred where the reference's arranger puts a single part: 135.5 x 136.
    # a 10 mm stem under a 30 mm cap
    "mushroom": [(130.5, 131, 0, 10, 10, 10), (120.5, 121, 10, 30, 30, 4)],
    # a plate, a post on it and a shelf from the post out over the plate
    "shelf": [(115.5, 126, 0, 40, 20, 2), (120.5, 131, 2, 10, 10, 10), (120.5, 131, 12, 30, 10, 3)],
    # a tall thin tower
    "tower": [(132.5, 133, 0, 6, 6, 80)],
    # two posts 30 mm apart and a deck across them
    "bridge": [(110.5, 131, 0, 10, 10, 8), (150.5, 131, 0, 10, 10, 8), (110.5, 131, 8, 50, 10, 3)],
    # a 20 mm cube, for a first print
    "cube": [(125.5, 126, 0, 20, 20, 20)],
    # a long low bar
    "bar": [(35.5, 131, 0, 200, 10, 30)],
    # a square frame 5 mm wide round a 30 mm hole, for walls round a hole
    "frame": [(115.5, 116, 0, 40, 5, 6), (115.5, 151, 0, 40, 5, 6), (115.5, 116, 0, 5, 40, 6), (150.5, 116, 0, 5, 40, 6)],
    # a plate with three fins on it, 0.5, 0.8 and 1.1 mm thick, for thin walls
    "fins": [(115.5, 126, 0, 40, 20, 1), (120.5, 131, 1, 0.5, 10, 5), (130.5, 131, 1, 0.8, 10, 5), (140.5, 131, 1, 1.1, 10, 5)],
    # two blocks that touch, one in each of two filaments
    "twotone": [(115.5, 126, 0, 20, 20, 6), (135.5, 126, 0, 20, 20, 4)],
    # a spine with seven teeth standing on the bed, 0.5 to 2 mm wide, for
    # what the first layer's elephant foot compensation leaves of thin parts
    "comb": [(115.5, 129, 0, 40, 6, 3)] + [
        (117.5 + 5.5 * i, 135, 0, w, 8, 3) for i, w in enumerate((0.5, 0.7, 0.9, 1.0, 1.1, 1.3, 2.0))],
}
# the comb again, its teeth in a second filament
SHAPES["twocomb"] = SHAPES["comb"]
# The filament each box of a shape is printed in, where it is not all the first.
FILAMENTS = {"twotone": [1, 2], "twocomb": [1] + [2] * 7}

out, source, shape = sys.argv[1:4]
changes = dict(arg.split("=", 1) for arg in sys.argv[4:])
objects = []
for index, dims in enumerate(SHAPES[shape]):
    v, t = box(*dims)
    xml = ['  <object id="%d" type="model"><mesh><vertices>' % (index + 1)]
    xml += ['   <vertex x="%g" y="%g" z="%g"/>' % p for p in v]
    xml.append('  </vertices><triangles>')
    xml += ['   <triangle v1="%d" v2="%d" v3="%d"/>' % tri for tri in t]
    xml.append('  </triangles></mesh></object>')
    objects.append("\n".join(xml))
parent = len(SHAPES[shape]) + 1
components = "".join('<component objectid="%d"/>' % (i + 1) for i in range(len(SHAPES[shape])))
model = f'''<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
 <resources>
{chr(10).join(objects)}
  <object id="{parent}" type="model"><components>{components}</components></object>
 </resources>
 <build><item objectid="{parent}"/></build>
</model>
'''
settings = json.loads(zipfile.ZipFile(source).read("Metadata/project_settings.config"))
for key, value in changes.items():
    settings[key] = [value] * len(settings[key]) if isinstance(settings.get(key), list) else value
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", '<?xml version="1.0" encoding="UTF-8"?>\n<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">\n <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>\n <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>\n</Types>\n')
    z.writestr("_rels/.rels", '<?xml version="1.0" encoding="UTF-8"?>\n<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">\n <Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>\n</Relationships>\n')
    z.writestr("3D/3dmodel.model", model)
    if shape in FILAMENTS:
        # As the reference saves it: the object's filament, then each part's own.
        parts = "".join(
            f'\n    <part id="{i + 1}" subtype="normal_part">\n      <metadata key="name" value="{shape} {i + 1}"/>\n'
            f'      <metadata key="extruder" value="{filament}"/>\n    </part>'
            for i, filament in enumerate(FILAMENTS[shape]))
        z.writestr("Metadata/model_settings.config",
                   f'<?xml version="1.0" encoding="UTF-8"?>\n<config>\n  <object id="{parent}">\n'
                   f'    <metadata key="name" value="{shape}"/>\n    <metadata key="extruder" value="1"/>{parts}\n'
                   f'  </object>\n</config>\n')
    z.writestr("Metadata/project_settings.config", json.dumps(settings, indent=4))
