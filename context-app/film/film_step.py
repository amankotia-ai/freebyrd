"""
film_step.py - films one step of the Freebyrd film, as the platform film's
film_step.py does: the screen texture sequence (frames.mjs flat mode, one PNG
per frame at 4x) on a flat plane, a camera that faces it square on and
dollies along the path the film itself defines (camera.json), real motion
blur on the moves, the hero charcoal all round. New here: a 14-inch laptop
built round the screen, lit like a product shot and standing on an unseen
floor that only takes its shadow.

  /Applications/Blender.app/Contents/MacOS/Blender -b -P film/film_step.py -- \
      TEX_DIR OUT_DIR [--frames 1:150] [--samples 64] [--res 2560x1152] [--filter 0.9]
      [--step 4] [--fps 60] [--shell-samples 32] [--save FILE.blend]

  --step N   render every Nth frame (a preview, as night shift does)
  --save     build the scene and save it instead of rendering (texture and
             output paths relative to the file)

Scale: 1 screen px (a macOS point) = 1 mm, so the screen is a 1.512 x 0.982 m
plane and the laptop is a 14-inch one at five times its size (the display is
302.4 mm wide). The screen is UNLIT emission under the Standard view
transform, so the UI's whites and the notch's blacks come out exactly as
drawn; the world is exactly #1f2126 to the camera. Everything else is lit.
The laptop is denoised in the compositor; the screen's own pixels are not
(a denoiser would smear the type), by its object index.

Axes: the screen lies in XY, facing +Z (the camera); +Y is up the screen.
The lid leans back 20 degrees and the base opens 110 degrees from it, so
the floor's up is (0, cos 20, sin 20) here.
"""
import bpy, bmesh, json, math, os, sys
from mathutils import Matrix, Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
TEX, OUT = os.path.abspath(argv[0]), os.path.abspath(argv[1])
opt = lambda k, d: argv[argv.index(k) + 1] if k in argv else d
F0, F1 = map(int, opt("--frames", "1:0").split(":"))
SAMPLES = int(opt("--samples", "48"))              # the screen's, as the platform film
SHELL_SAMPLES = int(opt("--shell-samples", "16"))  # the laptop's, denoised
RES_X, RES_Y = map(int, opt("--res", "2560x1152").split("x"))
FILTER = float(opt("--filter", "0.9"))
STEP = int(opt("--step", "1"))
SAVE = opt("--save", None)
BLUR = opt("--blur", "1") != "0"              # 0: no motion blur, for stills
FPS = int(opt("--fps", "30"))                 # the texture sequence's rate (frames.mjs seq:a:b:FPS)
PERSIST = opt("--persist", "1") != "0"        # keep the scene between frames (fast; see the handler below)

PX = 0.001                      # metres per screen px
SENSOR = 36.0                   # mm, fitted to the frame width
FOCAL = 85.0
HERO = (0x1F, 0x21, 0x26)

meta = json.load(open(os.path.join(TEX, "camera.json")))
WIN_W, WIN_H = meta["window"]
CAMS = meta["cam"]
frames = sorted(f for f in os.listdir(TEX) if f.endswith(".png"))
assert len(frames) == len(CAMS), (len(frames), len(CAMS))
if F1 == 0:
    F1 = len(frames)


def srgb_to_linear(c):
    c = c / 255
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


MM = 0.001
UP = Vector((0, math.cos(math.radians(20)), math.sin(math.radians(20))))     # the floor's up
FWD = Vector((0, -math.sin(math.radians(20)), math.cos(math.radians(20))))   # towards the viewer, level
RIGHT = Vector((1, 0, 0))

# the laptop, in screen mm (a 14-inch one, times five), its front measured
# from Apple's 14-inch bezel (bezel.json): a 27.5 mm border of silver edge,
# dark gasket and black glass round the screen, Apple's continuous corner
# curves at the top of the lid and the screen, square at the bottom, a 17.5
# mm black chin and a 42 mm dark hinge band under it
BZ = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "bezel.json")))
LID = BZ["lid"]
SCREEN_TOP = WIN_H / 2                       # 491
LID_W, LID_T = LID["w"], 24                  # 4.8 mm
LID_TOP = SCREEN_TOP + LID["border_top"]     # 518.5
CHIN_BOTTOM = -SCREEN_TOP - LID["chin"]      # -508.5
LID_BOTTOM = CHIN_BOTTOM - LID["hinge_band"] # -550.5
Q_LID = [tuple(p) for p in LID["top_corner"]]
BASE_W, BASE_D, BASE_T = LID_W, 1105, 51     # 312.6 x 221 x 10.2 mm
HINGE = Vector((0, (LID_BOTTOM + 4) * MM, -26 * MM))   # the base's back top edge


# ------------------------------------------------------------------ scene
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.render.engine = "CYCLES"
try:
    prefs = bpy.context.preferences.addons["cycles"].preferences
    prefs.compute_device_type = "METAL"
    # no background kernel specialisation: its compile threads crashed
    # Blender mid-render (NSURL from a nil path, 2026-10-14)
    if hasattr(prefs, "kernel_optimization_level"):
        prefs.kernel_optimization_level = "OFF"
    prefs.get_devices()
    for d in prefs.devices:
        d.use = True
    scene.cycles.device = "GPU"
except Exception:
    scene.cycles.device = "CPU"
scene.cycles.samples = SAMPLES
scene.cycles.use_adaptive_sampling = False
scene.cycles.use_denoising = False          # the compositor denoises the laptop only
scene.cycles.max_bounces = 6
scene.cycles.diffuse_bounces = 1
scene.cycles.glossy_bounces = 3
scene.cycles.transmission_bounces = 2
scene.cycles.transparent_max_bounces = 4
scene.cycles.caustics_reflective = False
scene.cycles.caustics_refractive = False
scene.cycles.blur_glossy = 1.0
scene.cycles.sample_clamp_indirect = 4.0
scene.cycles.pixel_filter_type = "BLACKMAN_HARRIS"
scene.cycles.filter_width = FILTER
scene.render.resolution_x, scene.render.resolution_y = RES_X, RES_Y
scene.render.resolution_percentage = 100
scene.render.fps = FPS                       # the 180-degree shutter is half of one frame at this rate
scene.render.use_overwrite = False          # a stopped render picks up where it left off
scene.render.dither_intensity = 0.0         # grain is added in post, on purpose
scene.render.image_settings.file_format = "PNG"
scene.render.image_settings.color_mode = "RGB"
scene.view_settings.view_transform = "Standard"
scene.view_settings.look = "None"
scene.view_settings.exposure = 0.0
scene.view_settings.gamma = 1.0
scene.render.use_motion_blur = BLUR
scene.render.use_persistent_data = PERSIST  # nothing moves but the camera: keep the BVH
scene.render.motion_blur_shutter = 0.5
scene.render.motion_blur_position = "CENTER"
scene.render.film_transparent = False
scene.frame_step = STEP

# the world: exactly the hero charcoal to the camera; to everything else a
# dim studio, lighter overhead, so the aluminium has something to reflect
world = bpy.data.worlds.new("hero")
world.use_nodes = True
wn = world.node_tree
for n in list(wn.nodes):
    wn.nodes.remove(n)
lp = wn.nodes.new("ShaderNodeLightPath")
geo = wn.nodes.new("ShaderNodeTexCoord")
dot = wn.nodes.new("ShaderNodeVectorMath"); dot.operation = "DOT_PRODUCT"
dot.inputs[1].default_value = UP
ramp = wn.nodes.new("ShaderNodeValToRGB")
ramp.color_ramp.elements[0].position = 0.72   # black at and below the horizon: the glass stays black
ramp.color_ramp.elements[0].color = (0.004, 0.004, 0.005, 1)
ramp.color_ramp.elements[1].position = 0.95
ramp.color_ramp.elements[1].color = (0.11, 0.115, 0.125, 1)
mapr = wn.nodes.new("ShaderNodeMapRange")
mapr.inputs["From Min"].default_value = -1
mapr.inputs["From Max"].default_value = 1
studio = wn.nodes.new("ShaderNodeBackground")
cam_bg = wn.nodes.new("ShaderNodeBackground")
cam_bg.inputs["Color"].default_value = (*[srgb_to_linear(c) for c in HERO], 1)
mix = wn.nodes.new("ShaderNodeMixShader")
wout = wn.nodes.new("ShaderNodeOutputWorld")
wn.links.new(geo.outputs["Generated"], dot.inputs[0])
wn.links.new(dot.outputs["Value"], mapr.inputs["Value"])
wn.links.new(mapr.outputs["Result"], ramp.inputs["Fac"])
wn.links.new(ramp.outputs["Color"], studio.inputs["Color"])
wn.links.new(lp.outputs["Is Camera Ray"], mix.inputs["Fac"])
wn.links.new(studio.outputs["Background"], mix.inputs[1])
wn.links.new(cam_bg.outputs["Background"], mix.inputs[2])
wn.links.new(mix.outputs["Shader"], wout.inputs["Surface"])
scene.world = world


# ------------------------------------------------------------------ materials
def principled(name, color, metallic=0.0, rough=0.5, spec=0.5, coat=0.0, aniso=0.0):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    b = m.node_tree.nodes["Principled BSDF"]
    b.inputs["Base Color"].default_value = (*color, 1)
    b.inputs["Metallic"].default_value = metallic
    b.inputs["Roughness"].default_value = rough
    b.inputs["Specular IOR Level"].default_value = spec
    b.inputs["Coat Weight"].default_value = coat
    b.inputs["Anisotropic"].default_value = aniso
    return m


ALU = principled("aluminium", (0.52, 0.53, 0.55), metallic=1.0, rough=0.36, aniso=0.2)
DECK = principled("deck", (0.46, 0.47, 0.49), metallic=1.0, rough=0.42, aniso=0.25)
HINGE_BAND = principled("hinge band", (0.005, 0.005, 0.0055), rough=0.75, spec=0.06)
GASKET = principled("gasket", (srgb_to_linear(35),) * 3, rough=0.6, spec=0.3)
GLASS = principled("bezel glass", (0.0, 0.0, 0.0), rough=0.12, spec=0.35, coat=0.25)
KEY = principled("keys", (0.009, 0.009, 0.01), rough=0.68, spec=0.12)
WELL = principled("keyboard well", (0.006, 0.006, 0.007), rough=0.8, spec=0.2)
PAD = principled("trackpad", (0.47, 0.48, 0.5), metallic=1.0, rough=0.3, aniso=0.1)
FOOT = principled("feet", (0.01, 0.01, 0.01), rough=0.7)

# speaker grilles: read as a darker strip of the aluminium (a hole pattern
# this fine would only shimmer at film size)
GRILLE = principled("grille", (0.085, 0.087, 0.092), metallic=0.6, rough=0.55)


# ------------------------------------------------------------------ geometry helpers
def rounded_rect(w, h, r, seg=14):
    """the outline of a w x h rectangle with corner radius r, centred, mm"""
    pts = []
    for cx, cy, a0 in ((w / 2 - r, h / 2 - r, 0), (-w / 2 + r, h / 2 - r, 90), (-w / 2 + r, -h / 2 + r, 180), (w / 2 - r, -h / 2 + r, 270)):
        for i in range(seg + 1):
            a = math.radians(a0 + 90 * i / seg)
            pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return pts


def corner_offset(q, d):
    """a quarter corner (x in from the side, y in from the top/bottom edge)
    moved d mm inwards along its normals"""
    q = [p for i, p in enumerate(q) if i == 0 or math.hypot(p[0] - q[i - 1][0], p[1] - q[i - 1][1]) > 1e-6]
    out = []
    for i, (x, y) in enumerate(q):
        ax, ay = q[max(i - 1, 0)]
        bx, by = q[min(i + 1, len(q) - 1)]
        tx, ty = bx - ax, by - ay
        L = math.hypot(tx, ty) or 1.0
        nx, ny = -ty / L, tx / L
        if nx + ny < 0:
            nx, ny = -nx, -ny
        out.append((x + d * nx, y + d * ny))
    return out


def outline(w, top, bottom, q_top, d=0.0, q_bottom=None):
    """a counter-clockwise outline (mm): top corners on q_top, bottom corners
    on q_bottom or square, all moved d inwards from the w-wide outer edge"""
    hw = w / 2
    qt = corner_offset(q_top, d) if d else list(q_top)
    pts = []
    if q_bottom:
        qb = corner_offset(q_bottom, d) if d else list(q_bottom)
        pts += [(-hw + x, bottom + y) for x, y in reversed(qb)]
        pts += [(hw - x, bottom + y) for x, y in qb]
    else:
        pts += [(-hw + d, bottom), (hw - d, bottom)]
    pts += [(hw - x, top - y) for x, y in reversed(qt)]
    pts += [(-hw + x, top - y) for x, y in qt]
    clean = []                                # ~1.2 mm apart: dense points choke the bevel
    for p in pts:
        if not clean or math.hypot(p[0] - clean[-1][0], p[1] - clean[-1][1]) > 1.2:
            clean.append(p)
    if math.hypot(clean[0][0] - clean[-1][0], clean[0][1] - clean[-1][1]) < 0.05:
        clean.pop()
    return clean


def slab_pts(name, pts, depth, bevel, mat, bevel_segs=4, z_top=0.0):
    """a slab on an outline, top face at z_top (mm), its rims bevelled"""
    me = bpy.data.meshes.new(name)
    bm = bmesh.new()
    top = [bm.verts.new((x * MM, y * MM, z_top * MM)) for x, y in pts]
    bot = [bm.verts.new((x * MM, y * MM, (z_top - depth) * MM)) for x, y in pts]
    bm.faces.new(top)
    bm.faces.new(list(reversed(bot)))
    n = len(top)
    for i in range(n):
        j = (i + 1) % n
        bm.faces.new((bot[i], bot[j], top[j], top[i]))
    bm.normal_update()
    bm.to_mesh(me)
    bm.free()
    ob = bpy.data.objects.new(name, me)
    scene.collection.objects.link(ob)
    me.materials.append(mat)
    if bevel > 0:
        bv = ob.modifiers.new("rims", "BEVEL")
        bv.width = bevel * MM
        bv.segments = bevel_segs
        bv.limit_method = "ANGLE"
        bv.angle_limit = math.radians(60)
        bv.harden_normals = True
        bv.use_clamp_overlap = False
    for p in me.polygons:
        p.use_smooth = True
    me.set_sharp_from_angle(angle=math.radians(35))
    return ob


def slab(name, w, h, depth, r, bevel, mat, seg=14, bevel_segs=4, z_top=0.0):
    """a rounded-corner slab (circular corners), for keys and the like"""
    return slab_pts(name, rounded_rect(w, h, r, seg), depth, bevel, mat, bevel_segs, z_top)


def resample(poly, n):
    """n points spread evenly along an open polyline"""
    acc = [0.0]
    for (ax, ay), (bx, by) in zip(poly, poly[1:]):
        acc.append(acc[-1] + math.hypot(bx - ax, by - ay))
    out, j = [], 0
    for k in range(n):
        at = acc[-1] * k / (n - 1)
        while j < len(acc) - 2 and acc[j + 1] < at:
            j += 1
        u = (at - acc[j]) / max(acc[j + 1] - acc[j], 1e-9)
        (ax, ay), (bx, by) = poly[j], poly[j + 1]
        out.append((ax + (bx - ax) * u, ay + (by - ay) * u))
    return out


def ring(w, top, bottom, q, d, n=32):
    """a counter-clockwise outline with all four corners on q, moved d in"""
    c = resample(corner_offset(q, d), n)
    hw = w / 2
    return ([(-hw + x, bottom + y) for x, y in reversed(c)] + [(hw - x, bottom + y) for x, y in c]
            + [(hw - x, top - y) for x, y in reversed(c)] + [(-hw + x, top - y) for x, y in c])


def rounded_body(name, w, depth, thick, q, r_top, r_bottom, mat, steps=(6, 9)):
    """a body on the outline (corners on q), top face at z = 0: its top edge
    rounded with radius r_top, its bottom edge with r_bottom (mm), and a
    support ring just inside each face so the faces stay flat"""
    levels = [(0.0, r_top + 1.5)]
    for i in range(steps[0] + 1):
        t = math.pi / 2 * i / steps[0]
        levels.append((-(r_top - r_top * math.cos(t)), r_top - r_top * math.sin(t)))
    for i in range(steps[1] + 1):
        t = math.pi / 2 * i / steps[1]
        levels.append((-(thick - r_bottom) - r_bottom * math.sin(t), r_bottom - r_bottom * math.cos(t)))
    levels.append((-thick, r_bottom + 1.5))
    me = bpy.data.meshes.new(name)
    bm = bmesh.new()
    rings = [[bm.verts.new((x * MM, y * MM, z * MM)) for x, y in ring(w, depth / 2, -depth / 2, q, d)] for z, d in levels]
    bm.faces.new(rings[0])
    bm.faces.new(list(reversed(rings[-1])))
    for up, dn in zip(rings, rings[1:]):
        n = len(up)
        for i in range(n):
            j = (i + 1) % n
            bm.faces.new((dn[i], dn[j], up[j], up[i]))
    bm.normal_update()
    bm.to_mesh(me)
    bm.free()
    for p in me.polygons:
        p.use_smooth = True
    me.set_sharp_from_angle(angle=math.radians(40))
    ob = bpy.data.objects.new(name, me)
    scene.collection.objects.link(ob)
    me.materials.append(mat)
    return ob


def place(ob, matrix):
    ob.matrix_world = matrix @ Matrix.LocRotScale(ob.location, ob.rotation_euler, ob.scale)


# ------------------------------------------------------------------ the lid
# from the outside in: the aluminium (its bevelled front edge is the silver
# line), the dark gasket 3.5 mm in, the black glass 8.5 mm in, the screen
lid = slab_pts("lid", outline(LID_W, LID_TOP, LID_BOTTOM, Q_LID), LID_T - 1.2, 2.2, ALU, z_top=-1.2)
gasket = slab_pts("gasket", outline(LID_W, LID_TOP, CHIN_BOTTOM, Q_LID, d=LID["silver"]), 1.0, 0.5, GASKET, bevel_segs=2, z_top=-0.6)
glass = slab_pts("bezel", outline(LID_W, LID_TOP, CHIN_BOTTOM, Q_LID, d=LID["silver"] + LID["gasket"]), 0.5, 0.25, GLASS, bevel_segs=2, z_top=-0.25)
# the hinge band under the chin: dark, between the lid's silver sides, its
# edges softened so the light rolls off them
band = LID["hinge_band"]
inner = LID_W / 2 - LID["silver"]
hinge = slab_pts("hinge band", [(-inner, LID_BOTTOM + 1.0), (inner, LID_BOTTOM + 1.0), (inner, CHIN_BOTTOM), (-inner, CHIN_BOTTOM)],
                 2.0, 1.0, HINGE_BAND, bevel_segs=3, z_top=-0.4)

# ------------------------------------------------------------------ the base, built flat (top at z = 0, front towards -y), then opened
# A 14-inch MacBook Pro's base, times five: 312.6 x 221.2 mm, about 10.2 mm
# thick, flat top and bottom, a tight round on the deck's edge and a softer
# one underneath, the lid's corner curve in plan. The keyboard sits in its
# black well 8 mm from the back: 14.5 keys wide on an 18.9 mm pitch, 16.4 mm
# keycaps, six full-height rows; speaker grilles either side; the trackpad
# (129.5 x 78.7 mm) centred 10 mm from the front.
base_parts = []
base = rounded_body("base", BASE_W, BASE_D, BASE_T, Q_LID, 5.0, 17.0, DECK)
base.location = (0, -BASE_D / 2 * MM, 0)
base_parts.append(base)

P, GAP, KEY_T = 94.5, 12.5, 3.0
rows = [
    [1.5] + [1.0] * 12 + [1.0],                               # esc, F1-F12, Touch ID
    [1.0] * 13 + [1.5],                                       # ` 1 ... = delete
    [1.5] + [1.0] * 13,                                       # tab q ... \
    [1.8] + [1.0] * 11 + [1.7],                               # caps a ... ' return
    [2.3] + [1.0] * 10 + [2.2],                               # shift z ... / shift
    [1.0, 1.0, 1.0, 1.25, 5.0, 1.25, 1.0, "arrows"],          # fn ctrl opt cmd space cmd opt arrows
]
KB_W = 14.5 * P
KB_TOP = -45.0
KB_H = len(rows) * P
well = slab("keyboard well", KB_W + 10, KB_H + 10, 3.0, 10, 0.0, WELL, seg=6, z_top=0.2)
well.location = (0, (KB_TOP - KB_H / 2) * MM, 0)
base_parts.append(well)
keymesh = {}
def key(w_units, h_units, x, y):
    w, h = w_units * P - GAP, h_units * P - GAP
    k = (round(w), round(h))
    if k not in keymesh:
        ob = slab(f"key {k}", w, h, KEY_T + 1.0, 6, 0.8, KEY, seg=5, bevel_segs=2, z_top=KEY_T)
        keymesh[k] = ob
    else:
        ob = bpy.data.objects.new("key", keymesh[k].data)
        scene.collection.objects.link(ob)
        bv = ob.modifiers.new("rims", "BEVEL")
        src = keymesh[k].modifiers["rims"]
        bv.width, bv.segments, bv.limit_method, bv.angle_limit, bv.harden_normals, bv.use_clamp_overlap = \
            src.width, src.segments, src.limit_method, src.angle_limit, src.harden_normals, src.use_clamp_overlap
    ob.location = (x * MM, y * MM, 0)
    base_parts.append(ob)
for r, row in enumerate(rows):
    x = -KB_W / 2
    yc = KB_TOP - (r + 0.5) * P
    for u in row:
        if u == "arrows":                                     # an inverted T of half-height keys
            key(1.0, 0.5, x + 0.5 * P, yc - P / 4)
            key(1.0, 0.5, x + 1.5 * P, yc + P / 4)
            key(1.0, 0.5, x + 1.5 * P, yc - P / 4)
            key(1.0, 0.5, x + 2.5 * P, yc - P / 4)
            x += 3 * P
            continue
        key(u, 1.0, x + u * P / 2, yc)
        x += u * P

PAD_W, PAD_H, PAD_FRONT = 647.5, 393.5, 50.0
pad = slab("trackpad", PAD_W, PAD_H, 0.6, 20, 1.0, PAD, seg=8, z_top=0.05, bevel_segs=2)
pad.location = (0, (-BASE_D + PAD_FRONT + PAD_H / 2) * MM, 0)
base_parts.append(pad)
GRILLE_W = 70.0
for side in (-1, 1):
    g = slab("grille", GRILLE_W, KB_H, 0.2, 8, 0.0, GRILLE, seg=6, z_top=0.02)
    g.location = (side * (KB_W / 2 + (BASE_W - KB_W) / 4) * MM, well.location.y, 0)
    base_parts.append(g)

# opened: the base's top faces the floor's up, its front towards the viewer
opened = Matrix.Translation(HINGE) @ Matrix.Rotation(math.radians(-70), 4, "X")
for ob in base_parts:
    place(ob, opened)

# the floor: only its shadow, under the base
bpy.ops.mesh.primitive_plane_add(size=12)
floor = bpy.context.active_object
floor.name = "floor"
floor.location = (0, 0, (-BASE_T - 3.0) * MM)   # on its feet
place(floor, opened)
floor.is_shadow_catcher = True

# ------------------------------------------------------------------ the screen
bpy.ops.mesh.primitive_plane_add(size=1)
win = bpy.context.active_object
win.name = "screen"
win.scale = (WIN_W * PX, WIN_H * PX, 1)
win.pass_index = 1
img = bpy.data.images.load(os.path.join(TEX, frames[0]))
img.source = "SEQUENCE"
img.colorspace_settings.name = "sRGB"
mat = bpy.data.materials.new("ui")
mat.use_nodes = True
nt = mat.node_tree
for n in list(nt.nodes):
    nt.nodes.remove(n)
tex = nt.nodes.new("ShaderNodeTexImage")
tex.image = img
tex.interpolation = "Linear"
tex.extension = "CLIP"
tex.image_user.frame_duration = len(frames)
tex.image_user.frame_start = 1
tex.image_user.frame_offset = -1            # scene frame 1 -> f0000.png
tex.image_user.use_auto_refresh = True
emit = nt.nodes.new("ShaderNodeEmission")
emit.inputs["Strength"].default_value = 1.0
outn = nt.nodes.new("ShaderNodeOutputMaterial")
nt.links.new(tex.outputs["Color"], emit.inputs["Color"])
nt.links.new(emit.outputs["Emission"], outn.inputs["Surface"])
win.data.materials.append(mat)
win.visible_shadow = False                  # it lights the keys, and casts nothing
win.visible_glossy = False                  # the deck mirroring it at a graze read as flat white


# Each frame's screen is a new 24-megapixel image. Kept, they pile up (a
# long render reached 78 GB and stalled the Mac), so each frame's is let go
# once it is rendered.
def _let_go(_scene, *_):
    for im in bpy.data.images:
        if im.source == "SEQUENCE":
            im.buffers_free()


bpy.app.handlers.render_post.append(_let_go)

# ------------------------------------------------------------------ lights: a soft key overhead left, a rim from behind right, a low fill
CENTRE = Vector((0, -0.35, 0.35))
def area(name, size, power, pos, aim=CENTRE, color=(1, 1, 1), spread=180):
    ld = bpy.data.lights.new(name, "AREA")
    ld.shape = "RECTANGLE"
    ld.size, ld.size_y = size
    ld.energy = power
    ld.color = color
    ld.spread = math.radians(spread)
    ob = bpy.data.objects.new(name, ld)
    scene.collection.objects.link(ob)
    ob.location = pos
    ob.rotation_euler = (aim - pos).to_track_quat("-Z", "Y").to_euler()
    ob.visible_camera = False
    ob.visible_glossy = True
    return ob
area("key", (3.2, 2.0), 420, CENTRE + UP * 2.7 + FWD * 0.8 - RIGHT * 2.2, color=(1.0, 0.985, 0.97))
area("rim", (2.4, 0.5), 520, CENTRE + UP * 1.6 - FWD * 2.6 + RIGHT * 2.6, color=(0.96, 0.98, 1.0))
area("fill", (2.0, 1.4), 50, CENTRE + UP * 0.4 + FWD * 2.4 + RIGHT * 2.8)
area("top", (4.0, 0.35), 45, CENTRE + UP * 3.2 - FWD * 0.6, color=(1.0, 1.0, 1.0))

# ------------------------------------------------------------------ the camera, as the platform film's
cam_data = bpy.data.cameras.new("lens")
cam_data.lens = FOCAL
cam_data.sensor_width = SENSOR
cam_data.sensor_fit = "HORIZONTAL"
cam_data.clip_start = 0.05
cam_data.clip_end = 100
cam = bpy.data.objects.new("lens", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
cam.rotation_euler = (0, 0, 0)              # square on: looking straight down -Z


def to_world(cx, cy):
    """screen px (x right, y down, origin top-left) -> plane coordinates"""
    return ((cx - WIN_W / 2) * PX, (WIN_H / 2 - cy) * PX)


bpy.context.preferences.edit.keyframe_new_interpolation_type = "LINEAR"
for i, c in enumerate(CAMS):
    x, y = to_world(c["cx"], c["cy"])
    dist = c["vw"] * PX * FOCAL / SENSOR
    cam.location = (x, y, dist)
    cam.keyframe_insert("location", frame=i + 1)

# ------------------------------------------------------------------ two layers: the screen as the platform film rendered it, the laptop lit
# The screen alone (emission, no bounces) at the film's samples; the laptop
# at a few samples, denoised. Square on, nothing ever covers the screen, so
# its layer needs nothing else. The compositor puts the screen's own pixels
# over the laptop's by the screen's object index.
screen_col = bpy.data.collections.new("screen")
laptop_col = bpy.data.collections.new("laptop")
scene.collection.children.link(screen_col)
scene.collection.children.link(laptop_col)
for ob in list(scene.collection.objects):
    if ob.type == "CAMERA":
        continue
    (screen_col if ob is win else laptop_col).objects.link(ob)
    scene.collection.objects.unlink(ob)
shell = scene.view_layers[0]
shell.name = "laptop"
shell.samples = SHELL_SAMPLES
shell.use_pass_object_index = True
shell.cycles.denoising_store_passes = True
screen_vl = scene.view_layers.new("screen")
screen_vl.samples = SAMPLES
screen_vl.use_pass_object_index = True
screen_vl.layer_collection.children["laptop"].exclude = True

tree = bpy.data.node_groups.new("finish", "CompositorNodeTree")
tree.interface.new_socket("Image", in_out="OUTPUT", socket_type="NodeSocketColor")
rl = tree.nodes.new("CompositorNodeRLayers")
rl.layer = "laptop"
rs = tree.nodes.new("CompositorNodeRLayers")
rs.layer = "screen"
dn = tree.nodes.new("CompositorNodeDenoise")
idm = tree.nodes.new("CompositorNodeIDMask")
idm.inputs["Index"].default_value = 1
idm.inputs["Anti-Alias"].default_value = True
mixc = tree.nodes.new("ShaderNodeMix")
mixc.data_type = "RGBA"
go = tree.nodes.new("NodeGroupOutput")
tree.links.new(rl.outputs["Image"], dn.inputs["Image"])
tree.links.new(rl.outputs["Denoising Albedo"], dn.inputs["Albedo"])
tree.links.new(rl.outputs["Denoising Normal"], dn.inputs["Normal"])
tree.links.new(rs.outputs["Object Index"], idm.inputs["ID value"])
tree.links.new(idm.outputs[0], mixc.inputs["Factor"])
tree.links.new(dn.outputs[0], mixc.inputs["A"])
tree.links.new(rs.outputs["Image"], mixc.inputs["B"])
tree.links.new(mixc.outputs["Result"], go.inputs[0])
scene.compositing_node_group = tree
scene.render.use_compositing = True
scene.cycles.denoising_quality = "HIGH"     # the denoise node follows the scene
if hasattr(scene.render, "compositor_device"):
    scene.render.compositor_device = "GPU"
    scene.render.compositor_precision = "FULL"

scene.frame_start, scene.frame_end = F0, F1
if SAVE:
    scene.render.filepath = "//render/r"
    bpy.ops.wm.save_as_mainfile(filepath=os.path.abspath(SAVE), relative_remap=True)
    img.filepath = bpy.path.relpath(img.filepath)
    bpy.ops.wm.save_mainfile()
    print("saved", SAVE)
    sys.exit(0)
os.makedirs(OUT, exist_ok=True)
scene.render.filepath = os.path.join(OUT, "r")
scene.render.use_file_extension = True
bpy.ops.render.render(animation=True)
print("rendered", F0, "to", F1, "step", STEP, "into", OUT)
