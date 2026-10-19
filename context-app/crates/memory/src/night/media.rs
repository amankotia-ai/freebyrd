//! What becomes of rendered frames, with ffmpeg: a clip per shot and a reel
//! that plays at real speed (a preview's every 4th frame at a quarter of the
//! film's rate), a thumbnail per shot, contact sheets for Claude to look at,
//! and plain checks: frames missing, frames black.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// One shot of a film, as Claude Code listed it in `shots.json`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Shot {
    pub name: String,
    pub slug: String,
    pub start: u32,
    pub end: u32,
    pub fps: f64,
}

/// Reads `shots.json`, keeping shots that make sense.
pub fn read_shots(path: &Path) -> Result<Vec<Shot>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let shots: Vec<Shot> =
        serde_json::from_str(&text).context("shots.json is not a list of shots")?;
    let shots: Vec<Shot> = shots
        .into_iter()
        .filter(|shot| {
            !shot.slug.is_empty()
                && shot
                    .slug
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                && shot.start <= shot.end
                && shot.fps > 0.0
                && shot.fps <= 240.0
        })
        .take(40)
        .collect();
    if shots.is_empty() {
        bail!("shots.json lists no usable shot");
    }
    Ok(shots)
}

/// ffmpeg, from Homebrew or the PATH.
pub fn ffmpeg() -> Option<PathBuf> {
    ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"]
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .or_else(|| {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|dir| dir.join("ffmpeg"))
                    .find(|path| path.is_file())
            })
        })
}

/// A shot's frames in `dir` (`<slug>_<number>.png`), in order.
pub fn frames_of(dir: &Path, slug: &str) -> Vec<(u32, PathBuf)> {
    let prefix = format!("{slug}_");
    let mut frames: Vec<(u32, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let number = name
                .strip_prefix(&prefix)?
                .strip_suffix(".png")?
                .parse()
                .ok()?;
            Some((number, entry.path()))
        })
        .collect();
    frames.sort();
    frames
}

/// The frames a shot should have at every `step`th frame.
pub fn expected(shot: &Shot, step: u32) -> Vec<u32> {
    (shot.start..=shot.end)
        .step_by(step.max(1) as usize)
        .collect()
}

/// Expected frames that are not there.
pub fn missing(shot: &Shot, step: u32, found: &[(u32, PathBuf)]) -> Vec<u32> {
    expected(shot, step)
        .into_iter()
        .filter(|number| !found.iter().any(|(n, _)| n == number))
        .collect()
}

/// How many frames all the shots should have.
pub fn total_expected(shots: &[Shot], step: u32) -> usize {
    shots.iter().map(|shot| expected(shot, step).len()).sum()
}

/// How many frames are in `dir` for these shots.
pub fn total_found(dir: &Path, shots: &[Shot]) -> usize {
    shots
        .iter()
        .map(|shot| frames_of(dir, &shot.slug).len())
        .sum()
}

fn run_ffmpeg(args: &[&str]) -> Result<String> {
    let ffmpeg = ffmpeg().context("ffmpeg is not installed")?;
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-v", "error", "-y"])
        .args(args)
        .output()
        .context("running ffmpeg")?;
    if !out.status.success() {
        bail!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("no details")
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A shot's clip from its frames, at `fps / step` so it plays at real speed.
pub fn clip(frames: &Path, shot: &Shot, step: u32, out: &Path) -> Result<()> {
    let pattern = frames.join(format!("{}_*.png", shot.slug));
    let rate = format!("{}/{}", shot.fps, step.max(1));
    run_ffmpeg(&[
        "-framerate",
        &rate,
        "-pattern_type",
        "glob",
        "-i",
        &path_str(&pattern),
        "-vf",
        "scale=trunc(iw/2)*2:trunc(ih/2)*2",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "18",
        "-pix_fmt",
        "yuv420p",
        &path_str(out),
    ])?;
    Ok(())
}

/// The shots' clips one after another.
pub fn reel(clips: &[PathBuf], out: &Path) -> Result<()> {
    if clips.len() == 1 {
        std::fs::copy(&clips[0], out)?;
        return Ok(());
    }
    let list = out.with_extension("txt");
    let lines: Vec<String> = clips
        .iter()
        .map(|clip| format!("file '{}'", path_str(clip).replace('\'', "'\\''")))
        .collect();
    std::fs::write(&list, lines.join("\n"))?;
    run_ffmpeg(&[
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        &path_str(&list),
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "18",
        "-pix_fmt",
        "yuv420p",
        &path_str(out),
    ])?;
    let _ = std::fs::remove_file(list);
    Ok(())
}

/// A small picture of one frame.
pub fn thumbnail(frame: &Path, out: &Path) -> Result<()> {
    run_ffmpeg(&[
        "-i",
        &path_str(frame),
        "-vf",
        "scale=360:-2",
        "-frames:v",
        "1",
        "-q:v",
        "4",
        &path_str(out),
    ])?;
    Ok(())
}

/// Frames spread evenly over a shot: `count` of them, the last repeated if
/// the shot has fewer.
pub fn spread(frames: &[(u32, PathBuf)], count: usize) -> Vec<PathBuf> {
    if frames.is_empty() || count == 0 {
        return Vec::new();
    }
    (0..count)
        .map(|i| {
            let index = if count == 1 {
                frames.len() / 2
            } else {
                (i * (frames.len() - 1)) / (count - 1)
            };
            frames[index.min(frames.len() - 1)].1.clone()
        })
        .collect()
}

/// Contact sheets: one row of `per_row` frames for each shot, at most
/// `rows` shots a sheet, for Claude to look at.
pub fn contact_sheets(
    frames: &Path,
    shots: &[Shot],
    per_row: usize,
    rows: usize,
    out_dir: &Path,
) -> Result<Vec<(PathBuf, Vec<String>)>> {
    let mut sheets = Vec::new();
    for (index, group) in shots.chunks(rows.max(1)).enumerate() {
        let staging = out_dir.join(format!("sheet-{index}"));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;
        let mut n = 0;
        let mut names = Vec::new();
        for shot in group {
            let picked = spread(&frames_of(frames, &shot.slug), per_row);
            if picked.is_empty() {
                continue;
            }
            names.push(shot.name.clone());
            for frame in picked {
                n += 1;
                std::fs::copy(&frame, staging.join(format!("s_{n:04}.png")))?;
            }
        }
        if names.is_empty() {
            continue;
        }
        let sheet = out_dir.join(format!("sheet-{index}.jpg"));
        let tile = format!(
            "scale=480:-2,tile={per_row}x{}:padding=6:margin=6:color=white",
            names.len()
        );
        run_ffmpeg(&[
            "-framerate",
            "1",
            "-i",
            &path_str(&staging.join("s_%04d.png")),
            "-vf",
            &tile,
            "-frames:v",
            "1",
            "-q:v",
            "3",
            &path_str(&sheet),
        ])?;
        let _ = std::fs::remove_dir_all(&staging);
        sheets.push((sheet, names));
    }
    Ok(sheets)
}

/// A frame counts as black below this average brightness (0 to 255, video
/// levels, where black is 16)...
const BLACK_BELOW: f64 = 20.0;
/// ...and as empty when its 10th and 90th brightness percentiles are closer
/// than this: one flat tone with nothing in it (a light switched off, a
/// camera inside a wall). A normal frame of the test film spans about 130,
/// its unlit frame 6.
const EMPTY_SPREAD: f64 = 12.0;

/// One frame's brightness: average, and the 10th and 90th percentiles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub avg: f64,
    pub low: f64,
    pub high: f64,
}

/// A shot's frames that are black, and those that look empty (not black).
pub fn blank_frames(
    frames: &[(u32, PathBuf)],
    dir: &Path,
    slug: &str,
) -> Result<(Vec<u32>, Vec<u32>)> {
    if frames.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let pattern = dir.join(format!("{slug}_*.png"));
    let out = run_ffmpeg(&[
        "-framerate",
        "1",
        "-pattern_type",
        "glob",
        "-i",
        &path_str(&pattern),
        "-vf",
        "scale=160:-2,signalstats,metadata=print:file=-",
        "-f",
        "null",
        "-",
    ])?;
    let mut black = Vec::new();
    let mut empty = Vec::new();
    for ((number, _), level) in frames.iter().zip(levels(&out)) {
        if level.avg < BLACK_BELOW {
            black.push(*number);
        } else if level.high - level.low < EMPTY_SPREAD {
            empty.push(*number);
        }
    }
    Ok((black, empty))
}

/// Each frame's brightness from ffmpeg's `signalstats` metadata.
pub fn levels(printed: &str) -> Vec<Level> {
    let mut levels = Vec::new();
    let mut current: Option<Level> = None;
    for line in printed.lines() {
        let line = line.trim();
        if line.starts_with("frame:") {
            levels.extend(current.take());
            current = Some(Level {
                avg: 0.0,
                low: 0.0,
                high: 255.0,
            });
            continue;
        }
        let Some((key, value)) = line
            .strip_prefix("lavfi.signalstats.")
            .and_then(|rest| rest.split_once('='))
        else {
            continue;
        };
        let (Some(level), Ok(value)) = (current.as_mut(), value.trim().parse::<f64>()) else {
            continue;
        };
        match key {
            "YAVG" => level.avg = value,
            "YLOW" => level.low = value,
            "YHIGH" => level.high = value,
            _ => {}
        }
    }
    levels.extend(current);
    levels
}

/// "212 to 240" or "212, 216 and 240": frame numbers in words.
pub fn ranges(numbers: &[u32], step: u32) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut start: Option<u32> = None;
    let mut last = 0;
    for &n in numbers {
        match start {
            Some(_) if n == last + step => {}
            Some(s) => {
                parts.push(if s == last {
                    s.to_string()
                } else {
                    format!("{s} to {last}")
                });
                start = Some(n);
            }
            None => start = Some(n),
        }
        last = n;
    }
    if let Some(s) = start {
        parts.push(if s == last {
            s.to_string()
        } else {
            format!("{s} to {last}")
        });
    }
    match parts.len() {
        0 => String::new(),
        1 => parts.remove(0),
        _ => {
            let end = parts.pop().unwrap_or_default();
            format!("{} and {end}", parts.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot() -> Shot {
        Shot {
            name: "Scene 4".into(),
            slug: "scene4".into(),
            start: 1,
            end: 13,
            fps: 60.0,
        }
    }

    #[test]
    fn every_4th_frame_is_expected_and_gaps_are_missing() {
        assert_eq!(expected(&shot(), 4), vec![1, 5, 9, 13]);
        let found = vec![(1, PathBuf::from("a")), (9, PathBuf::from("b"))];
        assert_eq!(missing(&shot(), 4, &found), vec![5, 13]);
    }

    #[test]
    fn frame_numbers_read_as_words() {
        assert_eq!(ranges(&[212, 216, 220, 240], 4), "212 to 220 and 240");
        assert_eq!(ranges(&[5], 4), "5");
        assert_eq!(ranges(&[1, 9, 17], 4), "1, 9 and 17");
        assert_eq!(ranges(&[], 4), "");
    }

    #[test]
    fn brightness_comes_from_signalstats_lines() {
        // As printed for the test film: a lit frame, then its unlit one.
        let printed = "frame:0    pts:0       pts_time:0\nlavfi.signalstats.YMIN=54\nlavfi.signalstats.YLOW=66\n\
            lavfi.signalstats.YAVG=143.659\nlavfi.signalstats.YHIGH=197\nframe:1    pts:1\n\
            lavfi.signalstats.YLOW=60\nlavfi.signalstats.YAVG=60.974\nlavfi.signalstats.YHIGH=66\n";
        let levels = levels(printed);
        assert_eq!(levels.len(), 2);
        assert_eq!(
            levels[0],
            Level {
                avg: 143.659,
                low: 66.0,
                high: 197.0
            }
        );
        assert!(
            levels[1].high - levels[1].low < EMPTY_SPREAD,
            "the unlit frame looks empty"
        );
        assert!(levels[0].high - levels[0].low > EMPTY_SPREAD);
    }

    #[test]
    fn spread_picks_from_start_to_end() {
        let frames: Vec<(u32, PathBuf)> = (1..=10)
            .map(|n| (n, PathBuf::from(n.to_string())))
            .collect();
        let picked = spread(&frames, 4);
        assert_eq!(picked.first().unwrap(), &PathBuf::from("1"));
        assert_eq!(picked.last().unwrap(), &PathBuf::from("10"));
        assert_eq!(spread(&frames[..2], 4).len(), 4);
    }

    #[test]
    fn shots_need_a_safe_slug_and_frames() {
        let dir = std::env::temp_dir().join(format!("freebyrd-shots-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shots.json");
        std::fs::write(
            &path,
            r#"[{"name":"Scene 4","slug":"scene4","start":1,"end":150,"fps":60},
               {"name":"Bad","slug":"../x","start":1,"end":2,"fps":60},
               {"name":"Backwards","slug":"b","start":9,"end":2,"fps":60}]"#,
        )
        .unwrap();
        let shots = read_shots(&path).unwrap();
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].slug, "scene4");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
