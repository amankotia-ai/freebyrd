"""
join.py - lines the per-step frame folders up as one sequence (symlinks, so
nothing is copied), for encode.py to turn into the one film the rail seeks.

  python3 film/join.py OUT_DIR STEP1_DIR STEP2_DIR [...]

Each step is rendered on its own (film_step.py over that step's textures and
camera.json); the steps meet on identical frames, so the joins are cuts
nobody can see. Prints where each step starts, in seconds at 30 fps: those
are the chapter starts for the page.
"""
import os, sys

out, dirs = sys.argv[1], sys.argv[2:]
os.makedirs(out, exist_ok=True)
for f in os.listdir(out):
    os.remove(os.path.join(out, f))
n, starts = 0, []
for d in dirs:
    starts.append(round(n / 30, 4))
    for f in sorted(x for x in os.listdir(d) if x.endswith(".png")):
        n += 1
        os.symlink(os.path.abspath(os.path.join(d, f)), os.path.join(out, f"j{n:05d}.png"))
print(f"{n} frames, {n / 30:.3f} s; chapter starts {starts}")
