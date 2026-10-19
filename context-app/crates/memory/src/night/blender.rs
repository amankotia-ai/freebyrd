//! Blender films at night: a preview reel at every 4th frame, and the full
//! render of a preview the person approved.
//!
//! A preview has three parts. Claude Code reads the project and writes
//! `render.sh` and `shots.json` in the job's folder (it may not change the
//! project). Freebyrd runs `render.sh` in the sandbox, counting frames for
//! the notch. Then plain code makes the reel, thumbnails and contact sheets
//! and finds missing and black frames, and Claude (Sonnet) looks at the
//! sheets for what code cannot see: parts cutting into each other, noise,
//! missing textures.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::media::{self, Shot};
use super::{Job, STEP, sandbox};
use crate::{Store, time};

/// Blender, where the Mac keeps it.
pub fn blender() -> Option<PathBuf> {
    [
        "/Applications/Blender.app/Contents/MacOS/Blender",
        "/Applications/Blender.app/Contents/MacOS/blender",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

const PREP_BUDGET_USD: f64 = 4.0;
const PREP_TIMEOUT: Duration = Duration::from_secs(40 * 60);
const RENDER_TIMEOUT: Duration = Duration::from_secs(8 * 3600);

pub(crate) const RULES: &str = "You work for Freebyrd's night shift while the person sleeps, so \
nobody can answer a question: decide sensibly and say what you assumed. You may write only inside \
the current folder; their project is for reading. There is no network, so nothing can be \
installed. Never use an em dash.";

/// What a job's run came to.
pub(crate) struct Outcome {
    pub ok: bool,
    pub result: Value,
}

fn failed(summary: &str, detail: &str) -> Outcome {
    Outcome {
        ok: false,
        result: json!({
            "summary": summary,
            "needs": true,
            "notes": [["error", detail]],
        }),
    }
}

/// A preview reel of what the person asked for, every 4th frame.
pub(crate) fn preview(store: &Store, job: &Job, dir: &Path) -> Result<Outcome> {
    let Some(blender) = blender() else {
        return Ok(failed(
            "Blender was not found",
            "Install Blender in Applications.",
        ));
    };
    let project = job.project.clone().unwrap_or_default();
    let what = job.spec["what"]
        .as_str()
        .or(job.spec["text"].as_str())
        .unwrap_or_default();
    let note = job.spec["note"]
        .as_str()
        .map(|note| format!("\nTheir note on the last preview: \"{note}\""))
        .unwrap_or_default();
    let prompt = format!(
        "The person queued this for tonight: \"{what}\"{note}\n\
         Their project folder: {project}\n\
         Blender: {blender}\n\n\
         Prepare a preview render in the current folder. Do not render the whole thing: Freebyrd \
         runs your script after you finish, in a sandbox like yours.\n\
         1. Find how this film is made: its .blend files, its scene scripts (Python with bpy) and \
         how earlier renders were run. Read; do not change the project. If a script must change \
         for the preview, copy it here and change the copy.\n\
         2. Write render.sh (bash) that renders the shots they asked for with \"$BLENDER\" -b. It \
         must render every $STEP-th frame (STEP comes from the environment, default 4: Blender's \
         -s, -e and -j, or scene.frame_step in a script); write PNG frames into $OUT (from the \
         environment, default ./frames) named <slug>_<frame number, 4 digits>.png with one slug \
         per shot; keep the film's own look (engine, samples, resolution) unless they asked \
         otherwise; write nothing outside this folder and $OUT.\n\
         3. Write shots.json: [{{\"name\": \"Scene 4\", \"slug\": \"scene4\", \"start\": 1, \
         \"end\": 150, \"fps\": 60}}], the shots in order, frame numbers as in the film.\n\
         4. Check it: run STEP=100000 OUT=./test bash render.sh (one frame a shot), look at a \
         frame, then delete ./test. Blender often cannot start in your sandbox; that is \
         expected (Freebyrd's sandbox runs it), so check the script by reading it instead and \
         end with READY.\n\
         End with one line: READY, or BLOCKED: what only the person can answer (a missing \
         file, which shots they meant).",
        blender = blender.display(),
    );
    let run = claude_cli::agent(
        &claude_cli::Agent {
            dir,
            model: super::WORK_MODEL,
            effort: "medium",
            budget_usd: PREP_BUDGET_USD,
            resume: None,
            timeout: PREP_TIMEOUT,
            rules: RULES,
            read: &[Path::new(&project)],
        },
        &prompt,
    );
    let run = match run {
        Ok(run) => run,
        Err(e) => return Ok(failed("Preparing the render failed", &format!("{e:#}"))),
    };
    if let Some(session) = &run.session_id {
        super::set_session(store, job.id, session)?;
    }
    let last_line = run
        .text
        .trim()
        .lines()
        .last()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let blocked = last_line
        .strip_prefix("BLOCKED:")
        .map(|why| super::no_dashes(why.trim()));
    let script = dir.join("render.sh");
    // With its files written, it renders even if Claude Code had doubts:
    // Blender rarely starts in Claude Code's sandbox, and does in Freebyrd's.
    let shots = match media::read_shots(&dir.join("shots.json")) {
        Ok(shots) if script.is_file() => shots,
        _ => {
            let mut outcome = match blocked {
                Some(why) => failed("It needs you before it can render", &why),
                None => failed(
                    "No render script was written",
                    run.stopped.as_deref().unwrap_or(&last_line),
                ),
            };
            outcome.result["cost_usd"] = json!(run.cost_usd);
            return Ok(outcome);
        }
    };
    let frames = dir.join("frames");
    let rendered = render(
        store,
        job,
        dir,
        &blender,
        &frames,
        &shots,
        STEP,
        &[dir.to_path_buf()],
    )?;
    let mut outcome = finish_preview(job, dir, &frames, &shots, rendered)?;
    if let Some(why) = blocked
        && let Some(notes) = outcome.result["notes"].as_array_mut()
    {
        notes.push(json!(["warn", format!("While preparing: {why}")]));
    }
    outcome.result["cost_usd"] = json!(run.cost_usd);
    Ok(outcome)
}

/// How a render went.
pub(crate) struct Rendered {
    pub ok: bool,
    pub minutes: i64,
    pub tail: String,
    pub timed_out: bool,
}

/// Runs a job's `render.sh` in the sandbox at every `step`th frame into
/// `frames`, reporting progress to the notch.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    store: &Store,
    job: &Job,
    script_dir: &Path,
    blender: &Path,
    frames: &Path,
    shots: &[Shot],
    step: u32,
    writable: &[PathBuf],
) -> Result<Rendered> {
    std::fs::create_dir_all(frames)?;
    let total = media::total_expected(shots, step);
    let started = time::now_ms();
    let mut writable = writable.to_vec();
    writable.push(frames.to_path_buf());
    let ended = sandbox::run(
        "bash ./render.sh",
        script_dir,
        &writable,
        &[
            ("STEP", step.to_string()),
            ("OUT", frames.to_string_lossy().into_owned()),
            ("BLENDER", blender.to_string_lossy().into_owned()),
        ],
        &frames.with_file_name(format!("render-{step}.log")),
        RENDER_TIMEOUT,
        || {
            let done = media::total_found(frames, shots);
            let _ = super::set_progress(
                store,
                job.id,
                &json!({ "text": format!("Frame {done} of {total}"), "done": done, "of": total }),
            );
        },
    )?;
    Ok(Rendered {
        ok: ended.ok,
        minutes: (time::now_ms() - started) / 60_000,
        tail: ended.tail,
        timed_out: ended.timed_out,
    })
}

/// Per shot: its frames checked, its clip and thumbnail made.
struct ShotResult {
    shot: Shot,
    frames: usize,
    missing: Vec<u32>,
    black: Vec<u32>,
    empty: Vec<u32>,
    clip: Option<PathBuf>,
    thumb: Option<PathBuf>,
}

fn check_shots(dir: &Path, frames: &Path, shots: &[Shot], step: u32) -> Vec<ShotResult> {
    shots
        .iter()
        .map(|shot| {
            let found = media::frames_of(frames, &shot.slug);
            let missing = media::missing(shot, step, &found);
            let (black, empty) =
                media::blank_frames(&found, frames, &shot.slug).unwrap_or_default();
            let clip = dir.join(format!("{}.mp4", shot.slug));
            let clip = (!found.is_empty() && media::clip(frames, shot, step, &clip).is_ok())
                .then_some(clip);
            let thumb = dir.join(format!("{}.jpg", shot.slug));
            let middle = media::spread(&found, 1);
            let thumb = middle
                .first()
                .and_then(|frame| media::thumbnail(frame, &thumb).ok().map(|()| thumb));
            ShotResult {
                shot: shot.clone(),
                frames: found.len(),
                missing,
                black,
                empty,
                clip,
                thumb,
            }
        })
        .collect()
}

fn finish_preview(
    job: &Job,
    dir: &Path,
    frames: &Path,
    shots: &[Shot],
    rendered: Rendered,
) -> Result<Outcome> {
    let checked = check_shots(dir, frames, shots, STEP);
    let made: usize = checked.iter().map(|shot| shot.frames).sum();
    if made == 0 {
        let why = if rendered.timed_out {
            "It ran out of time."
        } else {
            "Blender wrote no frames."
        };
        return Ok(failed(
            "The render made no frames",
            &format!("{why} {}", last_lines(&rendered.tail, 3)),
        ));
    }
    let clips: Vec<PathBuf> = checked
        .iter()
        .filter_map(|shot| shot.clip.clone())
        .collect();
    let reel = dir.join("reel.mp4");
    let reel = (!clips.is_empty() && media::reel(&clips, &reel).is_ok()).then_some(reel);
    // What Claude sees in the frames, by shot name.
    let looked = look(dir, frames, shots).unwrap_or_default();
    let mut notes: Vec<Value> = Vec::new();
    let mut shot_values = Vec::new();
    let mut problems = 0;
    for shot in &checked {
        let (mut state, mut note) = ("ok".to_owned(), String::new());
        if let Some((seen_state, seen_note)) = looked
            .iter()
            .find(|(name, _, _)| *name == shot.shot.name)
            .map(|(_, s, n)| (s, n))
        {
            state.clone_from(seen_state);
            note.clone_from(seen_note);
        }
        if !shot.black.is_empty() {
            state = "error".into();
            note = format!("{} black.", frames_phrase(&shot.black, STEP));
        } else if !shot.missing.is_empty() {
            state = "error".into();
            note = format!("{} missing.", frames_phrase(&shot.missing, STEP));
        } else if !shot.empty.is_empty() {
            // The frames code found say it exactly; Claude's words about the
            // same shot would mostly repeat them.
            if state == "ok" {
                state = "warn".into();
            }
            note = format!(
                "{} empty: one flat tone, nothing in view.",
                frames_phrase(&shot.empty, STEP)
            );
        }
        if state != "ok" {
            problems += 1;
            notes.push(json!([state, format!("{}: {}", shot.shot.name, note)]));
        }
        shot_values.push(json!({
            "name": shot.shot.name,
            "state": state,
            "note": note,
            "frames": shot.frames,
            "thumb": shot.thumb.as_ref().map(|path| path.to_string_lossy()),
            "clip": shot.clip.as_ref().map(|path| path.to_string_lossy()),
        }));
    }
    let clear: Vec<&str> = checked
        .iter()
        .zip(&shot_values)
        .filter(|(_, value)| value["state"] == "ok")
        .map(|(shot, _)| shot.shot.name.as_str())
        .collect();
    if !clear.is_empty() {
        notes.push(json!([
            "ok",
            format!(
                "{}: no black, empty or missing frames{}.",
                clear.join(", "),
                if looked.is_empty() {
                    ""
                } else {
                    ", and nothing wrong to the eye"
                }
            )
        ]));
    }
    if !rendered.ok {
        notes.push(json!([
            "warn",
            format!(
                "The render stopped early: {}",
                last_lines(&rendered.tail, 2)
            )
        ]));
    }
    let summary = match problems {
        0 if rendered.ok => format!("{} shots, all clear", checked.len()),
        0 => "Partly rendered".to_owned(),
        _ => notes
            .first()
            .and_then(|note| note[1].as_str())
            .unwrap_or("Problems found")
            .to_owned(),
    };
    Ok(Outcome {
        ok: true,
        result: json!({
            "summary": summary,
            "needs": problems > 0 || !rendered.ok,
            "shots": shot_values,
            "notes": notes,
            "reel": reel.map(|path| path.to_string_lossy().into_owned()),
            "frames_dir": frames.to_string_lossy(),
            "render_minutes": rendered.minutes,
            "step": STEP,
            "frames": made,
            "title": job.title,
        }),
    })
}

/// "Frame 9 is" or "Frames 212 to 240 are".
fn frames_phrase(numbers: &[u32], step: u32) -> String {
    if numbers.len() == 1 {
        format!("Frame {} is", numbers[0])
    } else {
        format!("Frames {} are", media::ranges(numbers, step))
    }
}

fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.trim().lines().collect();
    lines[lines.len().saturating_sub(count)..].join(" ")
}

const LOOK_SYSTEM: &str = "You check preview renders of a person's film for them. Each contact \
sheet shows one shot per row, frames left to right in time. Look for what plain code cannot \
catch: parts cutting into or through each other, odd cut-outs or gaps, noise or grain, missing \
textures (bright magenta), blown highlights, empty or broken framing. A preview may be rough; \
only report clear mistakes that would still be wrong in the final, never ordinary things such as \
an object passing in front of another or shadows overlapping. Reply with JSON only: \
{\"shots\": [{\"name\": \
the shot's name as given, \"state\": \"ok\", \"warn\" or \"error\", \"note\": one short sentence, \
empty when ok}]}. Never use an em dash.";

/// Claude (Sonnet) looks at contact sheets of the preview: (shot, state,
/// note) for each shot it judged.
fn look(dir: &Path, frames: &Path, shots: &[Shot]) -> Result<Vec<(String, String, String)>> {
    let sheets = media::contact_sheets(frames, shots, 4, 6, dir)?;
    let mut verdicts = Vec::new();
    for (sheet, names) in sheets {
        let bytes = std::fs::read(&sheet)?;
        let prompt = format!("Shots on this sheet, top to bottom: {}.", names.join("; "));
        let mut answer = String::new();
        claude_cli::stream(
            &claude_cli::Request {
                system_prompt: LOOK_SYSTEM,
                model: super::LOOK_MODEL,
                effort: "low",
            },
            &prompt,
            &[claude_cli::Image {
                media_type: "image/jpeg",
                data: &bytes,
            }],
            claude_cli::Tools::None,
            |event| {
                if let claude_cli::Event::Text(text) = event {
                    answer.push_str(&text);
                }
            },
        )
        .context("looking at the frames")?;
        verdicts.extend(verdicts_in(&answer));
    }
    Ok(verdicts)
}

pub(crate) fn verdicts_in(answer: &str) -> Vec<(String, String, String)> {
    let Some(value) = super::json_in(answer) else {
        return Vec::new();
    };
    value["shots"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|shot| {
            let state = match shot["state"].as_str()? {
                "error" => "error",
                "warn" => "warn",
                _ => "ok",
            };
            Some((
                shot["name"].as_str()?.to_owned(),
                state.to_owned(),
                super::no_dashes(shot["note"].as_str().unwrap_or_default().trim()),
            ))
        })
        .collect()
}

/// The full render of an approved preview: its script at every frame.
pub(crate) fn final_(store: &Store, job: &Job, dir: &Path) -> Result<Outcome> {
    let Some(blender) = blender() else {
        return Ok(failed(
            "Blender was not found",
            "Install Blender in Applications.",
        ));
    };
    let preview_dir = PathBuf::from(job.spec["preview_dir"].as_str().unwrap_or_default());
    let shots = match media::read_shots(&preview_dir.join("shots.json")) {
        Ok(shots) if preview_dir.join("render.sh").is_file() => shots,
        _ => {
            return Ok(failed(
                "The approved preview's script is gone",
                &preview_dir.to_string_lossy(),
            ));
        }
    };
    let frames = dir.join("frames");
    let rendered = render(
        store,
        job,
        &preview_dir,
        &blender,
        &frames,
        &shots,
        1,
        &[dir.to_path_buf(), preview_dir.clone()],
    )?;
    let checked = check_shots(dir, &frames, &shots, 1);
    let made: usize = checked.iter().map(|shot| shot.frames).sum();
    if made == 0 {
        return Ok(failed(
            "The final made no frames",
            &last_lines(&rendered.tail, 3),
        ));
    }
    let clips: Vec<PathBuf> = checked
        .iter()
        .filter_map(|shot| shot.clip.clone())
        .collect();
    let reel = dir.join("final.mp4");
    let reel = (!clips.is_empty() && media::reel(&clips, &reel).is_ok()).then_some(reel);
    let mut notes = Vec::new();
    let mut needs = !rendered.ok;
    for shot in &checked {
        if !shot.missing.is_empty() {
            needs = true;
            notes.push(json!([
                "error",
                format!(
                    "{}: {} missing.",
                    shot.shot.name,
                    frames_phrase(&shot.missing, 1).to_lowercase()
                )
            ]));
        }
        if !shot.black.is_empty() {
            needs = true;
            notes.push(json!([
                "error",
                format!(
                    "{}: {} black.",
                    shot.shot.name,
                    frames_phrase(&shot.black, 1).to_lowercase()
                )
            ]));
        }
        if !shot.empty.is_empty() {
            needs = true;
            notes.push(json!([
                "warn",
                format!(
                    "{}: {} empty.",
                    shot.shot.name,
                    frames_phrase(&shot.empty, 1).to_lowercase()
                )
            ]));
        }
    }
    if !needs {
        notes.push(json!([
            "ok",
            format!("{made} frames, none missing, black or empty.")
        ]));
    }
    Ok(Outcome {
        ok: true,
        result: json!({
            "summary": if needs { "The final has problems".to_owned() } else { format!("Rendered and checked · {made} frames") },
            "needs": needs,
            "shots": checked.iter().map(|shot| json!({
                "name": shot.shot.name,
                "state": if !shot.missing.is_empty() || !shot.black.is_empty() { "error" } else if shot.empty.is_empty() { "ok" } else { "warn" },
                "frames": shot.frames,
                "thumb": shot.thumb.as_ref().map(|path| path.to_string_lossy()),
            })).collect::<Vec<_>>(),
            "notes": notes,
            "reel": reel.map(|path| path.to_string_lossy().into_owned()),
            "frames_dir": frames.to_string_lossy(),
            "render_minutes": rendered.minutes,
            "frames": made,
        }),
    })
}
