"""
post_v3.py - the finishing pass for a v3 step, per frame: a soft spill of the
window's light onto the charcoal (never onto the UI), then fine luminance
film grain over everything, the UI included.

  python3 film/post.py IN_DIR OUT_DIR [--grain 0.011] [--glow 0.3]

No edge fade: on the page the film box masks the video into the hero
charcoal (platform-rails.css, .pf-film), on any crop.
Grain in the file is for review cuts. For the page, --grain 0 and the same
grain laid over the video in CSS keeps the MP4 small (animated grain defeats
H.264).
"""
import os, sys
from multiprocessing import Pool
import numpy as np
from PIL import Image, ImageFilter

argv = sys.argv[1:]
opt = lambda k, d: float(argv[argv.index(k) + 1]) if k in argv else d
GRAIN = opt("--grain", 0.011)     # sigma on the whites, 0-1 scale; the platform film had 0.016, a third less here (2026-10-15)
GLOW = opt("--glow", 0.3)
LUMA = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)


def blur(arr, r):
    im = Image.fromarray((np.clip(arr, 0, 1) * 255).astype(np.uint8))
    return np.asarray(im.filter(ImageFilter.GaussianBlur(r)), dtype=np.float32) / 255


def finish(job):
    src, dst, seed = job
    img = np.asarray(Image.open(src).convert("RGB"), dtype=np.float32) / 255
    h, w, _ = img.shape
    s = min(w, h)

    # glow: the bright UI blurred at two radii, screened on only where the
    # frame is dark, so light spills onto the charcoal and the type stays clean
    lum = img @ LUMA
    src_glow = img * np.clip((lum - 0.6) / 0.4, 0, 1)[..., None]
    g = blur(src_glow, s * 0.02) * 0.55 + blur(src_glow, s * 0.07) * 0.45
    dark = np.clip(1 - lum / 0.45, 0, 1)[..., None] ** 2
    out = 1 - (1 - img) * (1 - np.clip(g * GLOW * dark, 0, 1))

    # grain: luminance only, a touch softer than a pixel so it survives the
    # encoder; full on the UI, lighter on the charcoal (at the same amplitude
    # the dark ground read as sandpaper)
    if GRAIN > 0:
        rng = np.random.default_rng(seed)
        n = rng.standard_normal((h, w)).astype(np.float32)
        n = np.asarray(Image.fromarray(np.clip(n * 40 + 128, 0, 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(0.55)), dtype=np.float32)
        n = (n - 128) / 40
        n /= max(n.std(), 1e-6)
        lum = out @ LUMA
        out = out + (n * GRAIN * (0.55 + 0.45 * lum))[..., None]

    Image.fromarray((np.clip(out, 0, 1) * 255 + 0.5).astype(np.uint8)).save(dst)
    return dst


if __name__ == "__main__":
    a, b = argv[0], argv[1]
    os.makedirs(b, exist_ok=True)
    files = sorted(f for f in os.listdir(a) if f.endswith(".png"))
    jobs = [(os.path.join(a, f), os.path.join(b, f), i + 7) for i, f in enumerate(files)]
    with Pool() as p:
        for i, _ in enumerate(p.imap(finish, jobs)):
            if i % 30 == 0:
                print("post", i, "/", len(jobs), flush=True)
