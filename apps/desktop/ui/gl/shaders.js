'use strict';

// The 3D views' shaders, for WebGL2 (GLSL ES 3.00). Each says which of the
// reference's it is modelled on; the lights and their strengths are the
// reference's own numbers.
(function (GL) {
  const head = '#version 300 es\nprecision highp float;\nprecision highp int;\n';

  // The two lights every shaded thing is lit by, in eye space
  // (resources/shaders/140/gouraud.vs, gouraud_light.vs).
  const lights = `
#define INTENSITY_CORRECTION 0.6
// normalized values for (-0.6/1.31, 0.6/1.31, 1./1.31)
const vec3 LIGHT_TOP_DIR = vec3(-0.4574957, 0.4574957, 0.7624929);
#define LIGHT_TOP_DIFFUSE    (0.8 * INTENSITY_CORRECTION)
#define LIGHT_TOP_SPECULAR   (0.125 * INTENSITY_CORRECTION)
#define LIGHT_TOP_SHININESS  20.0
// normalized values for (1./1.43, 0.2/1.43, 1./1.43)
const vec3 LIGHT_FRONT_DIR = vec3(0.6985074, 0.1397015, 0.6985074);
#define LIGHT_FRONT_DIFFUSE  (0.3 * INTENSITY_CORRECTION)
#define INTENSITY_AMBIENT    0.3
// x = diffuse and ambient, y = specular.
vec2 lit(vec3 eye_position, vec3 eye_normal) {
  vec2 intensity;
  float NdotL = max(dot(eye_normal, LIGHT_TOP_DIR), 0.0);
  intensity.x = INTENSITY_AMBIENT + NdotL * LIGHT_TOP_DIFFUSE;
  intensity.y = LIGHT_TOP_SPECULAR * pow(max(dot(-normalize(eye_position), reflect(-LIGHT_TOP_DIR, eye_normal)), 0.0), LIGHT_TOP_SHININESS);
  NdotL = max(dot(eye_normal, LIGHT_FRONT_DIR), 0.0);
  intensity.x += NdotL * LIGHT_FRONT_DIFFUSE;
  return intensity;
}
`;

  const S = {};

  // Objects, the bed's model, the shells in Preview and the tool marker:
  // resources/shaders/140/gouraud.vs and gouraud.fs (objects, with the
  // part outside the print volume darkened), gouraud_light.vs and
  // gouraud_light.fs (the rest, with `u_emission`).
  //
  // A vertex is 16 bytes: position, the face's normal, and a mark that
  // says which colour of `u_palette` its triangle takes. `u_tint` with an
  // alpha of zero or more is used in place of the palette.
  S.mesh = {
    vertex: `${head}${lights}
layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in float a_mark;
uniform mat4 u_view_model;
uniform mat4 u_projection;
uniform mat3 u_view_normal;
uniform mat4 u_model;
uniform sampler2D u_palette;
uniform vec4 u_tint;
// Multiplies the alpha of what is drawn; with u_ghost, every colour is
// drawn at that alpha whatever its own (the shells of Preview).
uniform float u_alpha;
uniform bool u_ghost;
// 0: only what is opaque. 1: only what is not. 2: everything.
uniform int u_pass;
// The normals' matrix of u_model, for what faces down (u_slope).
uniform mat3 u_model_normal;
out vec2 v_intensity;
out vec4 v_color;
out vec3 v_world;
out float v_world_nz;
void main() {
  vec4 color = u_tint.a >= 0.0 ? u_tint : texelFetch(u_palette, ivec2(int(a_mark + 0.5), 0), 0);
  if (u_ghost) color.a = 1.0;
  bool opaque = color.a * u_alpha >= 0.999;
  if ((u_pass == 0 && !opaque) || (u_pass == 1 && opaque)) {
    gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
    return;
  }
  vec3 eye_normal = normalize(u_view_normal * a_normal);
  vec4 position = u_view_model * vec4(a_position, 1.0);
  v_intensity = lit(position.xyz, eye_normal);
  v_color = vec4(color.rgb, color.a * u_alpha);
  v_world = (u_model * vec4(a_position, 1.0)).xyz;
  v_world_nz = normalize(u_model_normal * a_normal).z;
  gl_Position = u_projection * position;
}`,
    fragment: `${head}
uniform float u_emission;
// The print volume the object belongs in: least and greatest corners.
// Fragments outside it are darker (gouraud.fs, print_volume, rectangle).
uniform bool u_volume;
uniform vec3 u_volume_min;
uniform vec3 u_volume_max;
// A section view (the painting tools' "Section view"): what has
// dot(normal, p) > offset in the world is cut away (u_clip: normal, offset).
uniform bool u_clipped;
uniform vec4 u_clip;
// What faces down more steeply than this is marked (mm_gouraud.fs, its
// slope detection: under -2, nothing): what is near the bed light blue, an
// overhang half red.
uniform float u_slope;
in vec2 v_intensity;
in vec4 v_color;
in vec3 v_world;
in float v_world_nz;
out vec4 out_color;
void main() {
  if (u_clipped && dot(v_world, u_clip.xyz) > u_clip.w) discard;
  vec3 color = v_color.rgb;
  if (u_slope > -2.0) {
    if (v_world.z < 0.1 && v_world.z > -0.1) color = vec3(0.73, 1.0, 1.0);
    else if (v_world_nz < u_slope - 0.0001) color = color * 0.5 + vec3(0.78, 0.0, 0.0) * 0.5;
  }
  if (u_volume && (any(lessThan(v_world, u_volume_min)) || any(greaterThan(v_world, u_volume_max))))
    color = mix(color, vec3(0.0), 0.3333);
  out_color = vec4(vec3(v_intensity.y) + color * (v_intensity.x + u_emission), v_color.a);
}`,
  };

  // One colour, unlit: resources/shaders/140/flat.vs and flat.fs. The
  // bed's plane, the plates' fills, the axes, and objects drawn as what
  // they are for picking and for the selection's mask.
  S.flat = {
    vertex: `${head}
layout(location = 0) in vec3 a_position;
uniform mat4 u_matrix;
void main() { gl_Position = u_matrix * vec4(a_position, 1.0); }`,
    fragment: `${head}
uniform vec4 u_color;
out vec4 out_color;
void main() { out_color = u_color; }`,
  };

  // Lines of a width in pixels, each a quad facing the viewer: what the
  // reference's dashed_thick_lines geometry shader makes of a line
  // (resources/shaders/140/dashed_thick_lines.gs), without the dashes.
  // WebGL draws lines one pixel wide and has no geometry shaders.
  S.line = {
    vertex: `${head}
layout(location = 0) in vec2 a_corner;
layout(location = 1) in vec3 a_from;
layout(location = 2) in vec3 a_to;
uniform mat4 u_matrix;
uniform vec2 u_viewport;
uniform float u_width;
void main() {
  vec4 from = u_matrix * vec4(a_from, 1.0);
  vec4 to = u_matrix * vec4(a_to, 1.0);
  vec2 a = from.xy / from.w * u_viewport;
  vec2 b = to.xy / to.w * u_viewport;
  vec2 along = b - a;
  float reach = length(along);
  vec2 across = reach > 0.0 ? vec2(-along.y, along.x) / reach : vec2(0.0, 1.0);
  vec4 here = mix(from, to, a_corner.x);
  here.xy += across * a_corner.y * u_width / u_viewport * here.w;
  gl_Position = here;
}`,
    fragment: `${head}
uniform vec4 u_color;
out vec4 out_color;
void main() { out_color = u_color; }`,
  };

  // A picture on the bed: resources/shaders/140/printbed.vs and
  // printbed.fs. With u_plain, the picture's own colour and alpha (the
  // plates' numbers and names); else the reference's bed, the picture
  // over a grey that darkens towards the corners.
  S.picture = {
    vertex: `${head}
layout(location = 0) in vec3 a_position;
layout(location = 1) in vec2 a_uv;
uniform mat4 u_matrix;
out vec2 v_uv;
void main() {
  v_uv = a_uv;
  gl_Position = u_matrix * vec4(a_position, 1.0);
}`,
    fragment: `${head}
const vec3 back_color_dark = vec3(0.235, 0.235, 0.235);
const vec3 back_color_light = vec3(0.365, 0.365, 0.365);
uniform sampler2D u_picture;
uniform bool u_plain;
uniform bool u_transparent;
in vec2 v_uv;
out vec4 out_color;
void main() {
  vec4 fore = texture(u_picture, v_uv);
  if (u_plain) {
    out_color = vec4(fore.rgb, u_transparent ? fore.a * 0.25 : fore.a);
    return;
  }
  vec3 back = mix(back_color_light, back_color_dark, smoothstep(0.0, 0.5, length(abs(v_uv) - vec2(0.5))));
  out_color = vec4(mix(back, fore.rgb, fore.a), u_transparent ? fore.a : 1.0);
}`,
  };

  // The lines of a print and the moves between them: libvgcode's
  // segments shader (src/libvgcode/src/Shaders.hpp). Each move is a box
  // with a rhombus for a cross-section, as wide and as high as the line
  // laid; only the half facing the viewer is built, from eight vertices,
  // and each end has a tip that fills the outside of a bend.
  //
  // libvgcode keeps its moves in textures and looks them up by number;
  // here the same moves are one buffer of 32-byte records (the layout is
  // in api/scene.rs), bound twice a record apart, so each instance has
  // the move before it (where it starts) and itself (where it ends).
  // What each move's colour is, the reference works out on the processor
  // and uploads again when the view changes; here the shader works it
  // out, so changing the view or hiding a feature uploads nothing.
  S.segments = {
    vertex: `${head}
#define POINTY_CAPS
#define FIX_TWISTING
layout(location = 0) in float a_corner;
layout(location = 1) in vec3 a_from;
layout(location = 2) in float a_from_turn;
layout(location = 3) in vec3 a_to;
layout(location = 4) in float a_feedrate;
layout(location = 5) in vec2 a_height_width;
layout(location = 6) in float a_turn;
layout(location = 7) in float a_temperature;
layout(location = 8) in vec4 a_what;
uniform mat4 u_view;
uniform mat4 u_projection;
uniform vec3 u_camera;
// Where the plate stands in the scene.
uniform vec3 u_origin;
// 0 line type, 1 filament, 2 speed, 3 layer height, 4 line width, 5 flow,
// 6 fan speed, 7 temperature.
uniform int u_view_type;
// Row 0: a colour for each feature. Row 1: for each filament. Row 2: the
// eleven colours of a range, least first.
uniform sampler2D u_palette;
// Least, greatest, and how many different values there are.
uniform vec3 u_range;
// A bit for each feature that is shown, and for each of the first 32
// filaments that is hidden.
uniform int u_features;
uniform int u_hidden_filaments;
uniform bool u_travels;
uniform bool u_wipes;
uniform vec3 u_travel_color;
uniform vec3 u_wipe_color;
out vec3 v_color;
out vec3 v_color_direct;

const vec3 light_top_dir = vec3(-0.4574957, 0.4574957, 0.7624929);
const float light_top_diffuse = 0.6 * 0.8;
const float light_top_specular = 0.6 * 0.25;
const float light_top_shininess = 20.0;
const vec3 light_front_dir = vec3(0.6985074, 0.1397015, 0.6985074);
const float light_front_diffuse = 0.6 * 0.2;
const float ambient = 0.2;
const float emission = 0.15;
const vec3 UP = vec3(0.0, 0.0, 1.0);
// libvgcode Types.hpp: DEFAULT_TRAVELS_RADIUS_MM, DEFAULT_WIPES_RADIUS_MM.
const float travel_size = 0.1;

float direct_lighting(vec3 eye_position, vec3 eye_normal) {
  float top_diffuse = light_top_diffuse * max(dot(eye_normal, light_top_dir), 0.0);
  float front_diffuse = light_front_diffuse * max(dot(eye_normal, light_front_dir), 0.0);
  float top_specular = light_top_specular * pow(max(dot(-normalize(eye_position), reflect(-light_top_dir, eye_normal)), 0.0), light_top_shininess);
  return top_diffuse + front_diffuse + top_specular;
}

vec3 palette(int row, int index) {
  return texelFetch(u_palette, ivec2(index, row), 0).rgb * 255.0;
}

// libvgcode ColorRange::get_color_at, a linear range.
vec3 ranged(float value) {
  if (u_range.z < 0.5) return palette(2, 0);
  value = clamp(value, u_range.x, u_range.y);
  float step_size = (u_range.y - u_range.x) / 10.0;
  float t = step_size > 0.0 ? (value - u_range.x) / step_size : 0.0;
  int low = clamp(int(t), 0, 10);
  int high = min(low + 1, 10);
  float part = t - float(low);
  return floor((1.0 - part) * palette(2, low) + part * palette(2, high));
}

void main() {
  int vertex_id = int(a_corner + 0.5);
  int kind = int(a_what.x + 0.5);
  int feature = int(a_what.y + 0.5);
  int filament = int(a_what.z + 0.5);
  float height = travel_size;
  float width = travel_size;
  bool shown = false;
  if (kind == 2) {
    height = a_height_width.x * 0.001;
    width = a_height_width.y * 0.001;
    bool hidden = filament >= 1 && filament <= 32 && ((u_hidden_filaments >> (filament - 1)) & 1) == 1;
    shown = ((u_features >> feature) & 1) == 1 && !hidden;
  } else if (kind == 1) {
    shown = u_travels;
  } else if (kind == 3) {
    shown = u_wipes;
  }
  if (!shown) {
    gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
    return;
  }

  vec3 pos_a = a_from + u_origin;
  vec3 pos_b = a_to + u_origin;
  // A line is laid under the nozzle: its middle is half its height down
  // (ViewerImpl.cpp, where the positions are uploaded).
  if (kind == 2) {
    pos_a.z -= 0.5 * height;
    pos_b.z -= 0.5 * height;
  }
  vec3 line = pos_b - pos_a;
  float line_len = length(line);
  vec3 line_dir;
  if (line_len < 1e-4)
    line_dir = vec3(1.0, 0.0, 0.0);
  else
    line_dir = line / line_len;
  vec3 line_right_dir;
  if (abs(dot(line_dir, UP)) > 0.9)
    line_right_dir = normalize(cross(vec3(1.0, 0.0, 0.0), line_dir));
  else
    line_right_dir = normalize(cross(line_dir, UP));
  vec3 line_up_dir = normalize(cross(line_right_dir, line_dir));

  const vec2 horizontal_vertical_view_signs_array[16] = vec2[](
    // horizontal view (from right)
    vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, 0.0), vec2(0.0, -1.0),
    vec2(0.0, -1.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, 0.0),
    // vertical view (from top)
    vec2(0.0, 1.0), vec2(-1.0, 0.0), vec2(0.0, 0.0), vec2(1.0, 0.0),
    vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(-1.0, 0.0), vec2(0.0, 0.0));

  vec3 endpoint_pos = vertex_id < 4 ? pos_a : pos_b;
  float angle = (vertex_id < 4 ? a_from_turn : a_turn) * 0.0001;

#ifdef FIX_TWISTING
  vec3 to_a = u_camera - pos_a;
  vec3 to_b = u_camera - pos_b;
  vec3 closer_pos = dot(to_a, to_a) < dot(to_b, to_b) ? pos_a : pos_b;
  vec3 camera_view_dir = normalize(closer_pos - u_camera);
#else
  vec3 camera_view_dir = normalize(endpoint_pos - u_camera);
#endif
  vec3 diagonal_dir_border = normalize(height * line_up_dir + width * line_right_dir);
  bool is_vertical_view = abs(dot(camera_view_dir, line_up_dir)) / abs(dot(diagonal_dir_border, line_up_dir)) >
    abs(dot(camera_view_dir, line_right_dir)) / abs(dot(diagonal_dir_border, line_right_dir));

  vec2 signs = horizontal_vertical_view_signs_array[vertex_id + 8 * int(is_vertical_view)];
  float view_right_sign = sign(dot(-camera_view_dir, line_right_dir));
  float view_top_sign = sign(dot(-camera_view_dir, line_up_dir));
  float half_height = 0.5 * height;
  float half_width = 0.5 * width;
  vec3 horizontal_dir = half_width * line_right_dir;
  vec3 vertical_dir = half_height * line_up_dir;
  float horizontal_sign = signs.x * view_right_sign;
  float vertical_sign = signs.y * view_top_sign;
  vec3 pos = endpoint_pos + horizontal_sign * horizontal_dir + vertical_sign * vertical_dir;
  if (vertex_id == 2 || vertex_id == 7) {
    float line_dir_sign = (vertex_id == 2) ? -1.0 : 1.0;
    if (angle == 0.0) {
#ifdef POINTY_CAPS
      // There the path changes or ends: a pointy cap.
      pos += line_dir_sign * line_dir * half_width;
#endif
    } else {
      // A tip that fills the outside of the bend.
      pos += line_dir_sign * line_dir * half_width * sin(abs(angle) * 0.5);
      pos += sign(angle) * horizontal_dir * cos(abs(angle) * 0.5);
    }
  }

  vec3 eye_position = (u_view * vec4(pos, 1.0)).xyz;
  // A wipe is drawn a little nearer, over the line it follows.
  if (kind == 3) eye_position.z += 0.05;
  vec3 eye_normal = (u_view * vec4(normalize(pos - endpoint_pos), 0.0)).xyz;

  // What colour the move is (ViewerImpl.cpp get_vertex_color).
  vec3 rgb;
  if (kind == 1) {
    rgb = u_view_type == 2 ? ranged(a_feedrate) : (u_view_type == 1 ? palette(1, filament) : u_travel_color);
  } else if (kind == 3) {
    rgb = u_view_type == 2 ? ranged(a_feedrate) : u_wipe_color;
  } else if (u_view_type == 0) {
    rgb = palette(0, feature);
  } else if (u_view_type == 1) {
    rgb = palette(1, filament);
  } else if (u_view_type == 2) {
    rgb = ranged(a_feedrate);
  } else if (u_view_type == 3) {
    rgb = ranged(height);
  } else if (u_view_type == 4) {
    rgb = ranged(width);
  } else if (u_view_type == 5) {
    // What a line of this width and height takes for each millimetre of
    // it, times how fast it is laid.
    rgb = ranged(a_feedrate * (height * (width - height) + 0.7853982 * height * height));
  } else if (u_view_type == 6) {
    rgb = ranged(a_what.w);
  } else {
    rgb = ranged(a_temperature);
  }
  vec3 color_base = rgb / 255.0;
  v_color = color_base * (ambient + emission);
  v_color_direct = color_base * direct_lighting(eye_position, eye_normal);
  gl_Position = u_projection * vec4(eye_position, 1.0);
}`,
    fragment: `${head}
in vec3 v_color;
in vec3 v_color_direct;
out vec4 out_color;
void main() {
  out_color = vec4(clamp(v_color + v_color_direct, 0.0, 1.0), 1.0);
}`,
  };

  // The marks of a print (retractions, seams, tool changes): libvgcode's
  // options shader (src/libvgcode/src/Shaders.hpp), a two-ended cone for
  // each, as wide and as high as the line it is on, times a factor.
  S.options = {
    vertex: `${head}
layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec3 a_at;
layout(location = 3) in vec2 a_height_width;
layout(location = 4) in float a_kind;
uniform mat4 u_view;
uniform mat4 u_projection;
uniform vec3 u_origin;
uniform float u_scaling;
// A bit for each kind of mark that is shown, and each kind's colour.
uniform int u_kinds;
uniform vec3 u_colors[8];
out vec3 v_color;
const vec3 light_top_dir = vec3(-0.4574957, 0.4574957, 0.7624929);
const float light_top_diffuse = 0.6 * 0.8;
const float light_top_specular = 0.6 * 0.125;
const float light_top_shininess = 20.0;
const vec3 light_front_dir = vec3(0.6985074, 0.1397015, 0.6985074);
const float light_front_diffuse = 0.6 * 0.3;
const float ambient = 0.3;
const float emission = 0.25;
float lighting(vec3 eye_position, vec3 eye_normal) {
  float top_diffuse = light_top_diffuse * max(dot(eye_normal, light_top_dir), 0.0);
  float front_diffuse = light_front_diffuse * max(dot(eye_normal, light_front_dir), 0.0);
  float top_specular = light_top_specular * pow(max(dot(-normalize(eye_position), reflect(-light_top_dir, eye_normal)), 0.0), light_top_shininess);
  return ambient + top_diffuse + front_diffuse + top_specular + emission;
}
void main() {
  int kind = int(a_kind + 0.5);
  if (((u_kinds >> kind) & 1) == 0) {
    gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
    return;
  }
  vec2 height_width = a_height_width;
  vec3 offset = a_at + u_origin - vec3(0.0, 0.0, 0.5 * height_width.x);
  height_width *= u_scaling;
  vec3 scale = vec3(height_width.y, height_width.y, height_width.x);
  vec3 eye_position = (u_view * vec4(scale * a_position + offset, 1.0)).xyz;
  // Nearer than the line it is on, so that it shows.
  eye_position.z += 0.1;
  vec3 eye_normal = normalize((u_view * vec4(a_normal / scale, 0.0)).xyz);
  v_color = u_colors[kind] * lighting(eye_position, eye_normal);
  gl_Position = u_projection * vec4(eye_position, 1.0);
}`,
    fragment: `${head}
in vec3 v_color;
out vec4 out_color;
void main() { out_color = vec4(clamp(v_color, 0.0, 1.0), 1.0); }`,
  };

  // What is selected, marked: resources/shaders/140/selection_composite.fs,
  // its "Thin" style. The selection is drawn into a mask; where the mask
  // ends, a crisp outline, and over the selection a faint grey.
  S.outline = {
    vertex: `${head}
layout(location = 0) in vec2 a_position;
out vec2 v_uv;
void main() {
  v_uv = a_position * 0.5 + 0.5;
  gl_Position = vec4(a_position, 0.0, 1.0);
}`,
    fragment: `${head}
uniform sampler2D u_mask;
uniform vec3 u_outline_color;
uniform float u_fill_alpha;
uniform float u_outline_width;
uniform float u_rim_width;
uniform float u_rim_alpha;
uniform vec2 u_texel;
const float fillGray = 0.4;
const float diagonal = 0.70710678;
in vec2 v_uv;
out vec4 out_color;
float fullCoverage(vec2 offsetPixels) {
  return texture(u_mask, v_uv + offsetPixels * u_texel).a;
}
// Largest selection coverage on a ring of the given radius in pixels (8 directions).
float ringCoverage(float radius) {
  float d = radius * diagonal;
  float axes = max(max(fullCoverage(vec2(radius, 0.0)), fullCoverage(vec2(-radius, 0.0))),
                   max(fullCoverage(vec2(0.0, radius)), fullCoverage(vec2(0.0, -radius))));
  float diagonals = max(max(fullCoverage(vec2(d, d)), fullCoverage(vec2(-d, d))),
                        max(fullCoverage(vec2(d, -d)), fullCoverage(vec2(-d, -d))));
  return max(axes, diagonals);
}
void main() {
  float coverage = fullCoverage(vec2(0.0));
  float outside = 1.0 - coverage;
  float line = clamp(max(ringCoverage(u_outline_width), ringCoverage(u_outline_width * 0.5)) * outside, 0.0, 1.0);
  float rim = 0.0;
  if (u_rim_alpha > 0.0)
    rim = clamp(ringCoverage(u_outline_width + u_rim_width) * outside * (1.0 - line), 0.0, 1.0) * u_rim_alpha;
  float fillCoverage = coverage * u_fill_alpha;
  float compositeAlpha = clamp(fillCoverage + line + rim, 0.0, 1.0);
  vec3 compositeColor = vec3(fillGray) * fillCoverage + u_outline_color * line;
  // The colour comes multiplied by its alpha already.
  out_color = vec4(compositeColor, compositeAlpha);
}`,
  };

  GL.shaders = S;
})(window.EdgeGL);
