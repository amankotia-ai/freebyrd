"""
eevee_frames.py - renders frames of a saved step file (film_step.py --save)
with EEVEE instead of Cycles, to compare speed and look.

  /Applications/Blender.app/Contents/MacOS/Blender -b STEP.blend -P film/eevee_frames.py -- OUT_DIR PREFIX FRAMES [FRAMES ...]
      FRAMES          a frame (73) or a run of them (1:24); frames already in OUT_DIR are skipped
      [--hq]          the final's quality: 128 samples, full-resolution shadows
                      and bounce light (about three times as long)

EEVEE has no shadow catcher, so the floor is kept out of the camera's view
but left in a reflection probe (the aluminium mirrors it, as in Cycles) and
a soft contact shadow is painted under the base. The screen stays emission,
so it comes out exactly as in Cycles. Three EEVEE-only fixes: one motion-blur
step, jittered soft shadows, and the speaker grilles lifted off the deck.
"""
import bpy, math, os, sys, time
import numpy as np
from mathutils import Matrix

argv = sys.argv[sys.argv.index("--") + 1:]
args = [a for a in argv if not a.startswith("--")]
HQ = "--hq" in argv
OUT, PREFIX = args[0], args[1]
FRAMES = []
for a in args[2:]:
    lo, _, hi = a.partition(":")
    FRAMES += range(int(lo), int(hi or lo) + 1)
os.makedirs(OUT, exist_ok=True)

s = bpy.context.scene
s.render.engine = "BLENDER_EEVEE"
ee = s.eevee
ee.taa_render_samples = 64
ee.use_raytracing = True
ee.ray_tracing_method = "PROBE"                 # screen-space rays would mirror the screen in the deck, which the Cycles render hides
ee.ray_tracing_options.resolution_scale = "1"
ee.use_fast_gi = True
ee.fast_gi_method = "GLOBAL_ILLUMINATION"
ee.use_shadows = True
ee.shadow_ray_count = 3
ee.shadow_step_count = 12
ee.motion_blur_steps = 1                          # one scene time a frame: more steps sample the screen's
                                                  # texture sequence at sub-frames and double the moving UI
s.render.filter_size = s.cycles.filter_width       # as sharp as the Cycles render (0.9 px, not EEVEE's 1.5)
if HQ:
    ee.taa_render_samples = 128
    ee.motion_blur_max = 64                       # px; the dollies never blur further than this
    ee.shadow_ray_count = 4
    ee.shadow_step_count = 16
    ee.shadow_pool_size = "1024"
    ee.fast_gi_resolution = "1"
    ee.fast_gi_step_count = 16
    ee.fast_gi_ray_count = 4
    ee.fast_gi_quality = 0.5
s.render.use_compositing = False                  # Cycles' denoise passes do not exist in EEVEE
for vl in s.view_layers:
    vl.use = vl.name == "laptop"                  # it holds the screen too; nothing covers it

# the screen lights the keys but is not seen in the aluminium (Cycles: visible_glossy off)
screen = bpy.data.objects["screen"]
for k in ("hide_probe_sphere", "hide_probe_volume", "hide_probe_plane"):
    if hasattr(screen, k):
        setattr(screen, k, True)

# the floor as Cycles' shadow catcher left it for every ray but the camera's:
# a lit grey plane that the base's front edge mirrors white. A reflection
# probe just in front of the base sees it and the studio, not the laptop.
floor = bpy.data.objects["floor"]
floor.visible_camera = False
floor.visible_shadow = False
for ob in bpy.data.objects:
    if ob.type == "MESH" and ob is not floor:
        ob.hide_probe_sphere = True
pd = bpy.data.lightprobes.new("floor reflections", "SPHERE")
pd.influence_distance = 4.0
pd.clip_start = 0.01
probe = bpy.data.objects.new("floor reflections", pd)
probe.location = floor.matrix_world @ Matrix.Translation((0, -1.5, 0.05)).translation
for coll in floor.users_collection:
    coll.objects.link(probe)

# the speaker grilles sit 0.02 mm proud of the deck, too close for EEVEE's
# depth buffer (they z-fight in streaks); lift them 0.4 mm, out of sight
up = (floor.matrix_world.to_3x3() @ Matrix.Identity(3).col[2]).normalized()
for ob in bpy.data.objects:
    if ob.name.startswith("grille"):
        ob.matrix_world = Matrix.Translation(up * 0.0004) @ ob.matrix_world

# big area lights: Cycles' penumbrae are wide and soft; EEVEE needs jittered
# shadow maps (accumulated over the samples) to match, or the lid's shadow
# crosses the keys with a hard edge
for ob in bpy.data.objects:
    if ob.type == "LIGHT":
        ob.data.use_shadow_jitter = True

# a soft contact shadow under the base, as Cycles' shadow catcher drew it
W_M, D_M = 1.9, 1.43                              # the decal, metres
BW, BD = 1.567, 1.105                             # the base
res = (380, 286)
ys, xs = np.mgrid[0:res[1], 0:res[0]]
px = (xs + 0.5) / res[0] * W_M - W_M / 2
py = (ys + 0.5) / res[1] * D_M - D_M / 2
dx = np.maximum(np.abs(px) - (BW / 2 - 0.05), 0)
dy = np.maximum(np.abs(py) - (BD / 2 - 0.05), 0)
d = np.hypot(dx, dy) - 0.05                       # outside the footprint, metres
alpha = np.where(d <= 0, 0.62, 0.62 * np.exp(-np.maximum(d, 0) / 0.035))
img = bpy.data.images.new("contact shadow", res[0], res[1], alpha=True)
px_ = np.zeros((res[1], res[0], 4), dtype=np.float32)
px_[..., 3] = alpha[::-1]
img.pixels.foreach_set(px_.ravel())
img.pack()
mat = bpy.data.materials.new("contact shadow")
mat.use_nodes = True
nt = mat.node_tree
for n in list(nt.nodes):
    nt.nodes.remove(n)
tex = nt.nodes.new("ShaderNodeTexImage"); tex.image = img; tex.extension = "CLIP"
tr = nt.nodes.new("ShaderNodeBsdfTransparent")
em = nt.nodes.new("ShaderNodeEmission"); em.inputs["Color"].default_value = (0, 0, 0, 1)
mix = nt.nodes.new("ShaderNodeMixShader")
out = nt.nodes.new("ShaderNodeOutputMaterial")
nt.links.new(tex.outputs["Alpha"], mix.inputs["Fac"])
nt.links.new(tr.outputs["BSDF"], mix.inputs[1])
nt.links.new(em.outputs["Emission"], mix.inputs[2])
nt.links.new(mix.outputs["Shader"], out.inputs["Surface"])
mat.surface_render_method = "BLENDED"
bpy.ops.mesh.primitive_plane_add(size=1)
decal = bpy.context.active_object
decal.name = "contact shadow"
decal.data.materials.append(mat)
decal.matrix_world = floor.matrix_world @ Matrix.Translation((0, -BD / 2, 0.0008)) @ Matrix.Diagonal((W_M, D_M, 1, 1))
for coll in floor.users_collection:
    if decal.name not in coll.objects:
        coll.objects.link(decal)
for coll in list(decal.users_collection):
    if coll not in floor.users_collection:
        coll.objects.unlink(decal)
decal.visible_shadow = False

for f in FRAMES:
    path = os.path.join(OUT, f"{PREFIX}-r{f:04d}")
    if os.path.exists(path + ".png"):
        continue
    s.frame_set(f)
    s.render.filepath = path
    t = time.time()
    bpy.ops.render.render(write_still=True)
    print(f"EEVEE {PREFIX} {f}: {time.time() - t:.2f} s")
