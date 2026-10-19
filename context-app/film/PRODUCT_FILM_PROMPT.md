# Product film prompt

A reusable brief for making a product showcase film the way the Freebyrd film was made
(2026-10-14 and 15). Paste everything below the line into a new session and fill in the
`{…}` placeholders, or leave them for the questions in step 0. The working pipeline it
describes lives in `context-app/film/` (film.html to film6.html, frames.mjs, film_step.py,
eevee_frames.py, post.py, join.py, make.sh, CHAPTERS.md); reuse it rather than starting over.

---

You are making a showcase film for **{PRODUCT}**, a {one-line description}. The film shows
the real product working on a real-looking device, in a calm, premium, documentary style:
one person's day (or journey), told in chapters, where every moment shows the product doing
something useful because of what is on the screen at that moment.

## 0. Ask first, then build

Before writing any code, ask the user these directional questions in one round. Use
multiple-choice questions where you can (put your recommendation first and mark it
"(Recommended)"), and offer a sensible default for everything so they can answer fast.
Do not start building until the story arc is agreed.

1. **Audience and placement.** Who watches this, and where: a website hero, a launch video,
   a sales deck, social? Does it loop?
2. **The person and the day.** Whose story is it (role, made-up name), and over what span
   (one working day, a week, a first hour with the product)?
3. **The throughline.** What one concrete goal or promise runs through every chapter and is
   resolved at the end? (Freebyrd's: "the revised launch plan by Thursday noon", made in
   chapter 1, delivered in chapter 6.)
4. **Features, in order.** Which capabilities must be introduced, which are optional, and in
   what order should a newcomer meet them? One new idea per chapter.
5. **Places.** Which apps or screens each chapter happens in (Mail, Calendar, a browser
   page, an IDE, a call). Real third-party app look, or generic?
6. **Device and setting.** Which device (default: 14-inch MacBook Pro, square-on), light or
   dark mode, wallpaper, locale (date format, currency, spelling).
7. **Look.** Reference film to match (default: the Unifize platform hero film), ground colour,
   grain and glow strength, how much camera movement (default: locked off; see Camera).
8. **Accuracy.** Strictly what the product does today, or are small liberties allowed
   (timing, data)? Which version of the product's UI should be filmed?
9. **Length and deliverables.** Seconds per chapter and in total; master resolution and frame
   rate; formats (master, ProRes for editing, small web cut); captions or audio.
10. **Review.** Who signs off, and at which stage (story, stills, preview reel, final)?

Then write the whole arc as a table (chapter, time, "{PRODUCT}…" idea, place, what happens),
get a yes, and before each chapter, its beats (step 2). Record every decision and its date in
`CHAPTERS.md`.

## 1. Focus

- **The product is the hero; the person's work is the stage.** Every product moment answers
  something visible on screen a moment before.
- **Accuracy over spectacle.** Every word, button, label, count and timing on screen is the
  product's own (see section 5). A beautiful frame that misrepresents the product is a defect.
- **Calm.** No whip pans, no constant drift. Viewers felt motion sick with a moving camera.

## 2. Story and direction

- **Chapters form one consecutive storyline.** The last frame of each chapter is the first frame
  of the next, checked pixel for pixel (render both flat and diff them; allow a 1 to 3 shade
  anti-aliasing difference at most).
- **One new product idea per chapter, in a new place.** Each builds on what the last left on screen
  and in the person's head. A feature appears only once the story has set it up (Freebyrd's night
  shift belongs at the end of the day, not the morning).
- **Each chapter loops: context, product, app, product again.** Something on screen (context),
  the product acts on it, the person goes back into the app and does the thing, the product
  picks up what just happened. No random feature tours.
- **Show the work where it happens.** Example: after code is written, open the page it changed and
  try it, then push; do not push from the chat.
- **Show results where the product keeps them.** Example: what is owed or waiting lives in the
  tasks view, not a side panel.
- **Time passes visibly, without cuts.** Use a time-lapse: the menu-bar clock runs, calendar "now"
  lines slide, the cursor fades out and back in. This is how chapters jump hours (09:34 to 11:30).
- **People and data are made up.** Names, companies, emails, prices. Meeting participants are
  initials, not faces. Pictures (a bookmarked post's image) are drawn, not copied. No real social
  handles.

## 3. Camera

- **Square-on to the screen, 20:9 frame.** Perspective is matched in Blender from `camera.json`
  (screen-pixel keys: centre `cx, cy` and frame width `vw`, monotone cubic through log(vw)).
- **Locked off.** One slow push-in from the whole laptop to the held shot at the start of the
  film, one slow pull-out at the very end. Chapters hand over in the held shot. Never faster than
  0.3 frame widths a second.
- **The held shot (Freebyrd's MAIN):** the screen's full width with the bezel's top corners and
  the notch, screen y -38 to 708 of 982 (`{cx: 756, cy: 335, vw: 1660}`). Text stays readable on
  a 1920 web cut.
- **When something happens out of shot, the app scrolls it into view;** the camera does not go to
  it. Windows that would sit below the shot are made smaller (an unmaximized window) rather than
  moving the camera.
- **Never crop the product's open UI.** Write a check (`filmCheck()`) that lists every frame
  where it leaves the shot. It must be empty.

## 4. Look

- **Ground:** charcoal `#1f2126`, everywhere outside the device (the world colour for camera rays).
- **Device:** a 14-inch MacBook Pro, modelled at 5x scale from measured references (Apple's
  bezel kit used for measurements only, never its pixels). Accurate:
  - screen corner radius and notch (185 x 32 pt)
  - lid and base proportions, keyboard, trackpad, speaker grilles, hinge band
  - black glass bezel
  
  Lid opened about 110 degrees, seen square-on.
- **Light:** a studio of soft area lights: key 420 W front-left-above, rim 520 W behind, fill
  50 W, top 45 W. The aluminium shows a soft gradient. The floor is never seen, but it lights and
  reflects.
- **Screen:** an emission plane carrying the UI frames. Colours are exact (Standard view transform,
  no desaturation, no glare on the screen from the lights).
- **Motion blur:** 180-degree shutter (0.5 frame), centred.
- **Post (`post.py`):** a soft glow of the window's light onto the charcoal (0.3, never on the UI)
  and fine luminance grain at 0.011. That is a third lighter than the Unifize film's 0.016; the
  user asked for less noise.
- **Web cut:** no grain in the video (moving grain bloats it). Add grain in CSS instead: a 128 px
  noise tile, hard-light, 4% opacity, jittered four times every 0.333 s with `steps(4)`.

## 5. The screen: the real product UI, as a pure function of time

- **One page per chapter (`filmN.html`):**
  - it loads the product's real UI markup, CSS and JavaScript
  - the backend is stubbed (every call waits forever)
  - each frame rebuilds the UI's state by calling the app's own functions
  - `window.seek(t)` renders any time `t` exactly
- **Animations are re-applied by time.** Turn off CSS transitions and animations, plus
  `requestAnimationFrame`. Apply each one from its own curve and duration in the app's CSS.
  Simulate the app's springs at 240 Hz from t = 0.
- **Other apps are drawn in HTML to match macOS.** That includes Mail, Calendar's day view,
  Spotlight, the ⌘Tab switcher, Chrome with tabs and address bar, a Notion page, a video call,
  the Claude app, and a localhost page. Get the details right:
  - menu-bar app names and menus change with the front app
  - traffic lights grey out on inactive windows
  - shortcuts flash the menu title (⌘V flashes Edit, ⌘W flashes File)
  - notification banners slide in from the right
- **The cursor moves in eased arcs, presses with a small scale, and rests off the text being read.**
  Buttons it presses show their pressed state.
- **Freeze the product UI as a snapshot for each render** (`--ui=DIR`). Parallel sessions may be
  editing the live UI.
- **Pacing:**
  - Reading time is about 0.25 s a word for the key text, plus a beat after any change.
  - A 50-word answer streams over about 6 s and is held about 2.5 s.
  - Typing runs at about 16 characters a second.
  - Every chapter keeps an `EDIT` table that adds or removes seconds at quiet moments. Use it to
    re-pace without re-authoring.

## 6. Accuracy rules

- **Read the product's code before you script a beat.** Get each wording, label, button, count,
  grouping and timing from the source. Examples from Freebyrd:
  - an event takes Up next from 2 hours before it starts
  - a meeting recap quick ask lasts 4 hours after the meeting
  - "Plan my day" turns into "What's left today?" past the middle of the usual day
  - a calendar alert fires 30 minutes before a meeting and at the start of a planned block
  - an Idea card is weighed on a session's first message
  - a Draft card arrives on the working day before the due date
- **Do not show what the product does not do.** For example, Freebyrd doesn't update a to-do from
  a hand-edited web page, and links in its answers are not clickable. When the story wants
  something the product cannot do, change the story.
- **When a liberty is unavoidable, say so and let the user choose.** Record their answer, for
  example "the Draft card arrives at 17:30, though the app would prepare it in the morning".
- **When the product's UI scrolls or clips, the film shows the same.** A long card scrolls to its
  buttons.

## 7. Render: Blender 5.1, EEVEE

EEVEE replaced Cycles. It is about 2.5x to 5x faster and has no denoiser flicker on smooth
aluminium. Quality is equal here because the screen is an emissive picture and the shell has a
few plain materials in empty, softly lit space.

Settings (`eevee_frames.py`; `--hq` for finals):

| Setting | Value |
|---|---|
| Engine | `BLENDER_EEVEE` |
| `taa_render_samples` | 64, or 128 for HQ |
| `use_raytracing` | True |
| `ray_tracing_method` | `PROBE` (screen-space rays mirror the screen into the aluminium) |
| `ray_tracing_options.resolution_scale` | `"1"` |
| Fast GI | `use_fast_gi` True, `GLOBAL_ILLUMINATION`; HQ: resolution `"1"`, 16 steps, 4 rays, quality 0.5 |
| Shadows | `use_shadows` True, rays 3 (HQ 4), steps 12 (HQ 16), HQ pool `"1024"` |
| `motion_blur_steps` | **1**. More steps sample the screen's image sequence at sub-frames and double any moving UI. |
| `motion_blur_max` | 64 px (HQ) |
| `render.filter_size` | 0.9 px, as sharp as the Cycles render |
| Compositing | off; render only the laptop view layer |

Scene fixes EEVEE needs:

- **Hide the screen from all light probes**, or the deck reflects it.
- **Floor:** `visible_camera` and `visible_shadow` off. Add a sphere light probe just in front of
  the base (influence 4 m), with every laptop mesh hidden from probes. The aluminium then mirrors
  the lit floor, and the front edge reads white as it did with Cycles' shadow catcher.
- **Lights:** `use_shadow_jitter` on, for soft penumbrae. Without it the lid's shadow crosses the
  keys with a hard edge.
- **Lift anything near-coplanar.** The speaker grilles were 0.02 mm proud of the deck, and EEVEE's
  depth buffer streaked them until they were raised 0.4 mm.
- **Paint a soft contact shadow** under the base (an alpha decal).
- **Render the `.blend` in place,** or its relative texture paths break (magenta screen).

Speed on an M3 Pro: about 1.5 s a frame standard and about 3.8 s HQ at 2560 x 1152, with memory
flat at about 3.3 GB. Run finals in 48-frame Blender processes with retries and `caffeinate`.

## 8. Pipeline and delivery

1. `make.sh snapshot`: freeze the product UI.
2. `frames.mjs flat seq:a:b:fps --page=filmN.html --ui=SNAPSHOT --scale=4`: render the screen
   texture sequence (1512 x 982 at 4x, 6048 x 3928 PNG) and `camera.json` in headless Chrome
   over CDP.
3. `film_step.py --save`: build one Blender file per render step (screen plane, laptop, lights,
   camera).
4. `eevee_frames.py --hq`: render the frames.
5. `post.py`: add glow and grain.
6. `join.py`: line the steps up into one sequence.
7. `ffmpeg`: encode.

`CH=N ENGINE=eevee FPS=60 zsh film/make.sh tex|blend|final|reel final|web` does all of this per
chapter.

Deliverables:

- **Master:** 2560 x 1152, 60 fps, H.264 CRF 14, `-preset slow -tune film`, faststart.
- **Edit master:** ProRes 422 HQ.
- **Web cut:** 1920 x 864, AV1 (`libsvtav1 -preset 5 -crf 42 -g 120`) plus an H.264 fallback
  (CRF 26), a poster frame, the grain tile and `embed.html`.

## 9. Working method

- **Storyboard stills first.** Render a handful of 1600 x 720 board frames at the chapter's key
  beats and look at every one before rendering anything else. Check that:
  - text is readable and not covered by the cursor
  - nothing is clipped or overlapping (z-order: the product UI and cursor above all windows)
  - numbers and times are consistent across chapters
- **Then a preview reel:** every 4th frame (7.5 fps) at 1280 x 576 in EEVEE, held to real time,
  textures at 2x, motion blur off. A chapter takes about 5 to 10 minutes. Send it for review.
  Render the final only after it is approved.
- **Check every hand-over:** chapter N's last frame against chapter N+1's first, pixel for pixel.
- **Explain in plain language, with an example, before technical detail.** Report what was
  checked and what was not. Never claim something was verified that wasn't.
- **Keep build output out of iCloud-synced folders,** and check free disk space before long renders.
- **Never use em dashes in anything written for the user or on screen.**

## 10. Lessons already paid for

- A camera that moves most of the time makes people motion sick. Lock it off.
- A story that tours features reads as random. Tie every feature to the throughline and to what
  is on screen.
- Cycles at low samples with per-frame denoising flickers on smooth surfaces. EEVEE does not.
- With EEVEE motion-blur steps above 1, an image-sequence texture shows double text on moving UI.
- A hidden typing caret that takes up width leaves a visible gap. Give carets negative margin.
- `z-index` on a window inside an un-isolated container rises above the product UI and the
  cursor. Isolate the desktop layer.
- Long cards and panels clip inside the product's fixed window height. Scroll them by time to
  what matters.
