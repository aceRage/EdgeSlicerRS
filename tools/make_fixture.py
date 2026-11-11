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

pocket is for make_overhang_printable: a slab 8 mm tall with two square
pockets 3 mm up into it from underneath, 100 mm2 and 16 mm2, so a hole size
between the two fills one and keeps the other.

Shapes: mushroom and shelf need supports; tower and bar get an automatic
brim; bridge is a deck across two posts; cube is a plain 20 mm cube;
twotone is two blocks side by side, the first in filament 1 and 6 mm tall,
the second in filament 2 and 4 mm tall, so there is a tool change on every
layer up to 4 mm and none above; comb is a spine with teeth 0.5 to 2 mm
wide standing on the bed, and twocomb the same with the teeth in filament 2,
for the first layer's elephant foot compensation; steps is three tiers with a
top surface on each, the lowest with a hole through it, for ironing. cylinder
is a round post, tube a round post with a round hole through it, and curve a
slab with one long side bowed out on a 80 mm radius, one end a half circle and
the other end's corners rounded to 1.5 mm, for arc fitting (use a source with
enable_arc_fitting=1, or give it as a change). The reference loads the filament preset named in the settings again,
so a change to a filament's own settings, such as filament_type, is undone.

funnel is for what goes by how far a round wall hangs out (a conditional scarf
seam's scarf_overhang_threshold): a round post that widens as it rises, straight
for 2 mm and then each 2 mm more steeply, every 0.2 mm layer's outline lying
0.06, 0.12, 0.18 and at the top 0.24 mm outside the one beneath.

holes is for the hole options (hole_to_polyhole, enable_circle_compensation):
a slab 6 mm tall of ten 16 mm cells in two rows. The near row has round
holes straight down through it, 2, 3, 5, 8 and 12 mm across, each in 64
sides. The far row has a hole that is nearly round (an ellipse 5 by 5.4 mm),
one of eight sides (too few to be taken for round), a round boss 10 mm
across standing on a cell, a round tube standing on another with a 4 mm hole
down through tube and cell, and a block 16 mm tall with a 5 mm hole through
it from front to back, which no layer sees as a hole.

counterbore is for counterbore_hole_bridging: a slab 8 mm tall of eight
20 mm cells in two rows, each with a hole up through it that is wide for
the first 3 mm and narrow above (a layer height that does not divide 3 mm
puts the step inside a layer). The near row's are round in round: 3 mm in
6 mm, 4 in 8, 5 in 12, and 4 in 12 with the narrow hole 2.5 mm and 1 mm off
the middle. The far row has a round 5 mm hole over a hexagonal one 10 mm
across its flats, a 5 mm square one over a round 10 mm, a hexagonal one
5 mm across its flats over a 10 mm square, and a plain round 5 mm hole
straight through, which the option must leave alone.
"""
import json, math, sys, zipfile

def box(x0, y0, z0, dx, dy, dz):
    v = [(x0, y0, z0), (x0 + dx, y0, z0), (x0 + dx, y0 + dy, z0), (x0, y0 + dy, z0),
         (x0, y0, z0 + dz), (x0 + dx, y0, z0 + dz), (x0 + dx, y0 + dy, z0 + dz), (x0, y0 + dy, z0 + dz)]
    t = [(0, 2, 1), (0, 3, 2), (4, 5, 6), (4, 6, 7), (0, 1, 5), (0, 5, 4),
         (1, 2, 6), (1, 6, 5), (2, 3, 7), (2, 7, 6), (3, 0, 4), (3, 4, 7)]
    return v, t

def circle(cx, cy, r, n, start=0.0, sweep=2 * math.pi, closed=True):
    """Points counter-clockwise on a circle: n steps over sweep, the last left off if closed."""
    return [(cx + r * math.cos(start + sweep * i / n), cy + r * math.sin(start + sweep * i / n))
            for i in range(n if closed else n + 1)]

def prism(outline, z0, dz, hole=None):
    """A counter-clockwise outline pulled up from z0 by dz. A convex outline is
    capped as a fan about its middle; with a hole (counter-clockwise too, as
    many points as the outline, starting at the same angle) the caps are the
    strip between the two."""
    n = len(outline)
    v = [(x, y, z0) for x, y in outline] + [(x, y, z0 + dz) for x, y in outline]
    t = []
    def wall(base, flip):
        for i in range(n):
            j = (i + 1) % n
            a, b, c, d = base + i, base + j, base + n + j, base + n + i
            t.extend([(a, c, b), (a, d, c)] if flip else [(a, b, c), (a, c, d)])
    wall(0, False)
    if hole is None:
        mx, my = sum(x for x, _ in outline) / n, sum(y for _, y in outline) / n
        v += [(mx, my, z0), (mx, my, z0 + dz)]
        for i in range(n):
            j = (i + 1) % n
            t.append((2 * n, j, i))
            t.append((2 * n + 1, n + i, n + j))
    else:
        assert len(hole) == n
        v += [(x, y, z0) for x, y in hole] + [(x, y, z0 + dz) for x, y in hole]
        wall(2 * n, True)
        for i in range(n):
            j = (i + 1) % n
            t.extend([(i, 2 * n + j, j), (i, 2 * n + i, 2 * n + j)])
            t.extend([(n + i, n + j, 3 * n + j), (n + i, 3 * n + j, 3 * n + i)])
    return v, t

def frustum(cx, cy, r0, r1, n, z0, dz):
    """A round post in n sides whose radius goes from r0 at z0 to r1 at z0 + dz."""
    v = [(x, y, z0) for x, y in circle(cx, cy, r0, n)] + [(x, y, z0 + dz) for x, y in circle(cx, cy, r1, n)]
    v += [(cx, cy, z0), (cx, cy, z0 + dz)]
    t = []
    for i in range(n):
        j = (i + 1) % n
        t += [(i, j, n + j), (i, n + j, n + i), (2 * n, j, i), (2 * n + 1, n + i, n + j)]
    return v, t

def funnel():
    """Five round sections one on another, each 2 mm tall: the first straight,
    the rest wider at the top by 0.6, 1.2, 1.8 and 2.4 mm of radius."""
    solids, r, z = [], 6.0, 0.0
    for grow in (0.0, 0.6, 1.2, 1.8, 2.4):
        solids.append(frustum(135.5, 136, r, r + grow, 128, z, 2))
        r, z = r + grow, z + 2
    return solids

def curve_outline():
    """A slab about 40 x 16 mm: its far side bows out on a 80 mm radius, its
    right end is a half circle and its left corners are rounded to 1.5 mm."""
    x0, y0, w, h, r = 115.5, 128.0, 32.0, 16.0, 1.5
    points = circle(x0 + r, y0 + r, r, 12, math.pi, math.pi / 2, closed=False)          # near left corner
    points += circle(x0 + w, y0 + h / 2, h / 2, 64, -math.pi / 2, math.pi, closed=False)   # right end
    half = math.asin((w - r) / 2 / 80.0)                                                   # far side, right to left
    cx, cy = x0 + r + (w - r) / 2, y0 + h - 80.0 * math.cos(half)
    points += circle(cx, cy, 80.0, 96, math.pi / 2 - half, 2 * half, closed=False)[1:]
    points += circle(x0 + r, y0 + h - r, r, 12, math.pi / 2, math.pi / 2, closed=False)[1:] # far left corner
    return points

# Shapes that are not boxes: each a list of (vertices, triangles).
MESHES = {
    # a round post 20 mm across, in 128 sides
    "cylinder": [prism(circle(135.5, 136, 10, 128), 0, 10)],
    # a round post 24 mm across with a 12 mm hole through it
    "tube": [prism(circle(135.5, 136, 12, 128), 0, 8, circle(135.5, 136, 6, 128))],
    "curve": [prism(curve_outline(), 0, 6)],
    "funnel": funnel(),
}

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
    # a slab 40 x 20 x 8 mm with two pockets 3 mm up into it from underneath,
    # 10 mm square and 4 mm square: walls round the pockets and a lid on them
    "pocket": [(115.5, 126, 0, 40, 5, 3), (115.5, 141, 0, 40, 5, 3), (115.5, 131, 0, 5, 10, 3),
               (130.5, 131, 0, 10, 10, 3), (140.5, 131, 0, 4, 3, 3), (140.5, 138, 0, 4, 3, 3),
               (144.5, 131, 0, 11, 10, 3), (115.5, 126, 3, 40, 20, 5)],
    # a square frame 5 mm wide round a 30 mm hole, for walls round a hole
    "frame": [(115.5, 116, 0, 40, 5, 6), (115.5, 151, 0, 40, 5, 6), (115.5, 116, 0, 5, 40, 6), (150.5, 116, 0, 5, 40, 6)],
    # a plate with three fins on it, 0.5, 0.8 and 1.1 mm thick, for thin walls
    "fins": [(115.5, 126, 0, 40, 20, 1), (120.5, 131, 1, 0.5, 10, 5), (130.5, 131, 1, 0.8, 10, 5), (140.5, 131, 1, 1.1, 10, 5)],
    # a base 45 x 20 x 6 mm with five bars across it, 1, 2, 3 and 5 mm
    # apart: the base's top shows between them in strips of those widths,
    # so the solid layers under each strip are small bridges over infill
    # of different sizes (dont_filter_internal_bridges, enable_extra_bridge_layer)
    "gaps": [(113, 126, 0, 45, 20, 6)] + [
        (113 + x, 126, 6, w, 20, 3) for x, w in ((0, 8), (9, 6), (17, 6), (26, 6), (37, 8))],
    # two blocks that touch, one in each of two filaments
    "twotone": [(115.5, 126, 0, 20, 20, 6), (135.5, 126, 0, 20, 20, 4)],
    # a spine with seven teeth standing on the bed, 0.5 to 2 mm wide, for
    # what the first layer's elephant foot compensation leaves of thin parts
    "comb": [(115.5, 129, 0, 40, 6, 3)] + [
        (117.5 + 5.5 * i, 135, 0, w, 8, 3) for i, w in enumerate((0.5, 0.7, 0.9, 1.0, 1.1, 1.3, 2.0))],
    # three tiers, each with a top surface: a plate 3 mm tall with a square
    # hole through it, a block on it to 6 mm and a smaller one to 10 mm, for
    # ironing: top surfaces at three heights, one with a hole in it, and
    # two partly covered by what stands on them
    "steps": [(115.5, 121, 0, 4, 30, 3), (127.5, 121, 0, 28, 30, 3), (119.5, 121, 0, 8, 11, 3), (119.5, 140, 0, 8, 11, 3),
              (133.5, 121, 3, 22, 30, 3), (145.5, 126, 6, 10, 20, 4)],
}
# the comb again, its teeth in a second filament
SHAPES["twocomb"] = SHAPES["comb"]
# The filament each box of a shape is printed in, where it is not all the first.
FILAMENTS = {"twotone": [1, 2], "twocomb": [1] + [2] * 7}

def around(cx, cy, half, angles):
    """Where rays from the middle of a square at these angles leave it: the
    square's outline, with a point for each point of a hole in its middle."""
    return [(cx + half / max(abs(math.cos(a)), abs(math.sin(a))) * math.cos(a),
             cy + half / max(abs(math.cos(a)), abs(math.sin(a))) * math.sin(a)) for a in angles]

def cell(cx, cy, z0, dz, hole):
    """A 16 mm square cell about (cx, cy) with a hole down through it, given
    as (rx, ry, sides); none leaves it solid."""
    if hole is None:
        return box(cx - 8, cy - 8, z0, 16, 16, dz)
    rx, ry, n = hole
    angles = [2 * math.pi * i / n for i in range(n)]
    return prism(around(cx, cy, 8, angles), z0, dz,
                 [(cx + rx * math.cos(a), cy + ry * math.sin(a)) for a in angles])

def lying(solid, y0):
    """A prism drawn in x and z and pulled along y from y0: its y and z
    change places, which turns every triangle over."""
    v, t = solid
    return [(x, y0 + z, y) for x, y, z in v], [(a, c, b) for a, b, c in t]

def holes():
    xs = [103.5 + 16 * i for i in range(5)]
    near, far = 128, 144
    solids = [cell(x, near, 0, 6, (d / 2, d / 2, 64)) for x, d in zip(xs, (2, 3, 5, 8, 12))]
    solids.append(cell(xs[0], far, 0, 6, (2.5, 2.7, 64)))          # nearly round
    solids.append(cell(xs[1], far, 0, 6, (3, 3, 8)))               # eight sides
    solids.append(cell(xs[2], far, 0, 6, None))
    solids.append(prism(circle(xs[2], far, 5, 64), 6, 4))          # a boss
    solids.append(cell(xs[3], far, 0, 6, (2, 2, 64)))
    solids.append(prism(circle(xs[3], far, 5, 64), 6, 4, circle(xs[3], far, 2, 64)))  # a tube
    # the hole from front to back: drawn about (x, z) = (xs[4], 8), pulled along y
    solids.append(lying(cell(xs[4], 8, 0, 16, (2.5, 2.5, 64)), far - 8))
    return solids

MESHES["holes"] = holes()

def sided(cx, cy, flat, sides, angles):
    """Where rays from (cx, cy) at these angles leave a regular polygon of
    this many sides, flat mm from its middle to each side, with a side
    square to the ray at angle 0."""
    step = 2 * math.pi / sides
    def reach(a):
        return flat / math.cos((a + step / 2) % step - step / 2)
    return [(cx + reach(a) * math.cos(a), cy + reach(a) * math.sin(a)) for a in angles]

def counterbore():
    n = 72
    angles = [2 * math.pi * i / n for i in range(n)]
    def ring(cx, cy, r):
        return [(cx + r * math.cos(a), cy + r * math.sin(a)) for a in angles]
    xs = [105.5 + 20 * i for i in range(4)]
    near, far = 126, 146
    # each cell: where it is, its hole for the first 3 mm, and its hole above
    cells = [
        (xs[0], near, ring(xs[0], near, 3), ring(xs[0], near, 1.5)),
        (xs[1], near, ring(xs[1], near, 4), ring(xs[1], near, 2)),
        (xs[2], near, ring(xs[2], near, 6), ring(xs[2], near, 2.5)),
        (xs[3], near, ring(xs[3], near, 6), ring(xs[3] + 2.5, near + 1, 2)),          # off the middle
        (xs[0], far, sided(xs[0], far, 5, 6, angles), ring(xs[0], far, 2.5)),         # round over hexagonal
        (xs[1], far, ring(xs[1], far, 5), sided(xs[1], far, 2.5, 4, angles)),         # square over round
        (xs[2], far, sided(xs[2], far, 5, 4, angles), sided(xs[2], far, 2.5, 6, angles)),  # hexagonal over square
        (xs[3], far, ring(xs[3], far, 2.5), ring(xs[3], far, 2.5)),                   # straight through
    ]
    solids = []
    for cx, cy, wide, narrow in cells:
        outline = around(cx, cy, 10, angles)
        solids.append(prism(outline, 0, 3, wide))
        solids.append(prism(outline, 3, 5, narrow))
    return solids

MESHES["counterbore"] = counterbore()

out, source, shape = sys.argv[1:4]
changes = dict(arg.split("=", 1) for arg in sys.argv[4:])
objects = []
solids = MESHES[shape] if shape in MESHES else [box(*dims) for dims in SHAPES[shape]]
for index, (v, t) in enumerate(solids):
    xml = ['  <object id="%d" type="model"><mesh><vertices>' % (index + 1)]
    xml += ['   <vertex x="%.9g" y="%.9g" z="%.9g"/>' % p for p in v]
    xml.append('  </vertices><triangles>')
    xml += ['   <triangle v1="%d" v2="%d" v3="%d"/>' % tri for tri in t]
    xml.append('  </triangles></mesh></object>')
    objects.append("\n".join(xml))
parent = len(solids) + 1
components = "".join('<component objectid="%d"/>' % (i + 1) for i in range(len(solids)))
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
