//! What the closed notch keeps in view, the way the Dynamic Island keeps a
//! Live Activity: right of the notch, how much on the to-do list is overdue
//! or due today; left of it, work in progress: a job started with Run now,
//! and night shift (what waits for tonight, or how far tonight's run has
//! got); and the person's Claude Code sessions, how many work (left) and
//! how many wait for them (right, `sessions`); and, for a few minutes after
//! Claude Code reads the memory through the context connector, how many
//! items it read today (left, `reads`). They stay while they are true, calm
//! and black; only waiting cards make the island glow. Plain code over the
//! memory and the transcripts, nothing sent anywhere. A tool's ask to read a
//! project becomes its consent card here (`memory::connector::card_asks`).

use std::sync::atomic::{AtomicBool, Ordering};

use memory::night::{self, Job, Kind};
use memory::todos::Badge;
use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Live {
    /// What is overdue or due today on the to-do list.
    pub todo: Option<Todo>,
    /// A job started with Run now, while it runs.
    pub task: Option<Progress>,
    /// Night shift: jobs waiting for tonight, or tonight's run.
    pub night: Option<Night>,
    /// Claude Code sessions working, and waiting for the person.
    pub sessions: Option<crate::sessions::Summary>,
    /// Claude Code read the memory in the last few minutes: today's count.
    pub reads: Option<memory::connector::Reading>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub count: usize,
    pub overdue: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// How far, from 0 to 1; none while it starts.
    pub done: Option<f64>,
    /// Beside the pie: "2/5", the item a list is on, or "40%" of the frames.
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Night {
    /// Jobs waiting for the night to start (or Start now).
    pub queued: usize,
    /// The job tonight's run is on, if it is running.
    pub running: Option<Progress>,
}

impl Live {
    pub fn is_empty(&self) -> bool {
        self.todo.is_none()
            && self.task.is_none()
            && self.night.is_none()
            && self.sessions.is_none()
            && self.reads.is_none()
    }

    /// Whether something moves, so it is looked at again soon: a job's
    /// progress, or a session that will finish.
    pub fn is_moving(&self) -> bool {
        self.task.is_some()
            || self.night.as_ref().is_some_and(|n| n.running.is_some())
            || self.sessions.as_ref().is_some_and(|s| s.working > 0)
    }
}

/// What is live now. Nothing, if the memory cannot be read (said once).
pub fn now() -> Live {
    static SAID: AtomicBool = AtomicBool::new(false);
    let say = |e: anyhow::Error| {
        if !SAID.swap(true, Ordering::Relaxed) {
            eprintln!("freebyrd: memory: the notch's indicators: {e:#}");
        }
    };
    let store = match memory::Store::open_default() {
        Ok(store) => store,
        Err(e) => {
            say(e);
            return Live::default();
        }
    };
    let badge = memory::todos::badge(&store, chrono::Local::now())
        .map_err(say)
        .ok();
    let jobs = night::tonight(&store).map_err(say).unwrap_or_default();
    // Only ask whether the runner is alive when there is something to run.
    let runner = !jobs.is_empty() && night::run::is_running();
    Live {
        sessions: crate::sessions::summary(chrono::Local::now()),
        reads: connector(&store),
        ..of(badge.as_ref(), &jobs, runner)
    }
}

/// The context connector: a tool's new asks become consent cards, and a
/// read in the last few minutes shows with today's count. Errors are said
/// once.
fn connector(store: &memory::Store) -> Option<memory::connector::Reading> {
    static SAID: AtomicBool = AtomicBool::new(false);
    let say = |e: anyhow::Error| {
        if !SAID.swap(true, Ordering::Relaxed) {
            eprintln!("freebyrd: the context connector's records: {e:#}");
        }
    };
    let connector = memory::connector::Connector::open_default()
        .map_err(say)
        .ok()?;
    let now = chrono::Local::now();
    match memory::connector::card_asks(store, &connector, now.timestamp_millis()) {
        Ok(0) => {}
        Ok(asks) => eprintln!("freebyrd: the context connector: {asks} new ask(s) for the notch"),
        Err(e) => say(e),
    }
    memory::connector::reading(&connector, now)
        .map_err(say)
        .ok()?
}

/// What is live, from the to-do list's badge and the night jobs still to
/// run (queued or running). `runner` says whether the runner is working:
/// a job left "running" by a runner that stopped is not shown at all.
fn of(badge: Option<&Badge>, jobs: &[Job], runner: bool) -> Live {
    let todo = badge.filter(|b| b.count > 0).map(|b| Todo {
        count: b.count,
        overdue: b.overdue,
    });
    let run_now = |job: &Job| job.spec["now"].as_bool().unwrap_or(false);
    // Run now: running, or just pressed and starting. One queued with the
    // runner gone waits for the night like the rest.
    let task = jobs
        .iter()
        .find(|job| run_now(job) && runner && matches!(job.state.as_str(), "running" | "queued"))
        .map(progress);
    let running = jobs
        .iter()
        .find(|job| !run_now(job) && runner && job.state == "running")
        .map(progress);
    let queued = jobs
        .iter()
        .filter(|job| job.state == "queued" && !(run_now(job) && runner))
        .count();
    let night = (queued > 0 || running.is_some()).then_some(Night { queued, running });
    Live {
        todo,
        task,
        night,
        sessions: None,
        reads: None,
    }
}

/// How far a running job has got, from what its runner last wrote.
fn progress(job: &Job) -> Progress {
    let counts = job.progress.as_ref().and_then(|p| {
        let done = p["done"].as_u64()?;
        let of = p["of"].as_u64().filter(|&of| of > 0)?;
        Some((done.min(of), of))
    });
    let Some((done, of)) = counts.filter(|_| job.state == "running") else {
        return Progress {
            done: None,
            label: None,
        };
    };
    let fraction = done as f64 / of as f64;
    let label = match job.kind {
        // The item it is on, as "Item 2 of 5" says.
        Kind::List => format!("{}/{of}", (done + 1).min(of)),
        Kind::Preview | Kind::Final => format!("{}%", (fraction * 100.0).floor()),
    };
    Progress {
        done: Some(fraction),
        label: Some(label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn job(id: i64, kind: Kind, state: &str, now: bool, progress: Option<Value>) -> Job {
        Job {
            id,
            kind,
            title: format!("Job {id}"),
            sub: String::new(),
            project: None,
            spec: if now {
                json!({ "now": true })
            } else {
                json!({})
            },
            state: state.to_owned(),
            position: id,
            parent_id: None,
            dir: None,
            session_id: None,
            progress,
            result: None,
            created_at: 0,
            started_at: None,
            finished_at: None,
            acted_at: None,
            action: None,
        }
    }

    fn badge(count: usize, overdue: bool) -> Badge {
        Badge {
            count,
            overdue,
            open: count + 3,
        }
    }

    #[test]
    fn nothing_due_and_nothing_queued_shows_nothing() {
        let live = of(Some(&badge(0, false)), &[], false);
        assert!(live.is_empty());
        assert!(!live.is_moving());
        assert!(of(None, &[], true).is_empty());
    }

    #[test]
    fn the_to_do_count_is_what_is_overdue_or_due_today() {
        let live = of(Some(&badge(2, true)), &[], false);
        assert_eq!(
            live.todo,
            Some(Todo {
                count: 2,
                overdue: true
            })
        );
        assert!(!live.is_moving());
    }

    #[test]
    fn jobs_queued_for_tonight_show_the_moon_with_their_count() {
        let jobs = [
            job(1, Kind::Preview, "queued", false, None),
            job(2, Kind::List, "queued", false, None),
        ];
        let live = of(None, &jobs, false);
        assert_eq!(
            live.night,
            Some(Night {
                queued: 2,
                running: None
            })
        );
        assert_eq!(live.task, None);
    }

    #[test]
    fn tonight_running_shows_how_far_with_the_rest_still_queued() {
        let jobs = [
            job(
                1,
                Kind::Preview,
                "running",
                false,
                Some(json!({ "text": "Frame 120 of 300", "done": 120, "of": 300 })),
            ),
            job(2, Kind::List, "queued", false, None),
        ];
        let live = of(None, &jobs, true);
        let night = live.night.clone().unwrap();
        assert_eq!(night.queued, 1);
        let running = night.running.unwrap();
        assert_eq!(running.label.as_deref(), Some("40%"));
        assert!((running.done.unwrap() - 0.4).abs() < 1e-9);
        assert!(live.is_moving());
    }

    #[test]
    fn run_now_is_its_own_indicator_and_names_the_item_it_is_on() {
        let jobs = [job(
            7,
            Kind::List,
            "running",
            true,
            Some(json!({ "text": "Item 2 of 5", "done": 1, "of": 5 })),
        )];
        let live = of(None, &jobs, true);
        let task = live.task.clone().unwrap();
        assert_eq!(task.label.as_deref(), Some("2/5"));
        assert!((task.done.unwrap() - 0.2).abs() < 1e-9);
        assert_eq!(live.night, None);
        assert!(live.is_moving());
    }

    #[test]
    fn run_now_just_pressed_is_starting() {
        let jobs = [job(7, Kind::List, "queued", true, None)];
        let task = of(None, &jobs, true).task.unwrap();
        assert_eq!(
            task,
            Progress {
                done: None,
                label: None
            }
        );
    }

    #[test]
    fn a_run_now_job_without_a_runner_waits_for_the_night() {
        let jobs = [job(7, Kind::List, "queued", true, None)];
        let live = of(None, &jobs, false);
        assert_eq!(live.task, None);
        assert_eq!(live.night.unwrap().queued, 1);
    }

    #[test]
    fn a_job_left_running_by_a_stopped_runner_is_not_shown() {
        let jobs = [
            job(1, Kind::Preview, "running", false, None),
            job(2, Kind::List, "running", true, None),
        ];
        assert!(of(None, &jobs, false).is_empty());
    }

    #[test]
    fn progress_without_counts_shows_as_starting() {
        // Before `done` and `of` were written, a list's progress was text
        // only; a runner from then shows as starting, not as wrong numbers.
        let jobs = [job(
            7,
            Kind::List,
            "running",
            true,
            Some(json!({ "text": "Item 2 of 5" })),
        )];
        let task = of(None, &jobs, true).task.unwrap();
        assert_eq!(task.done, None);
        assert_eq!(task.label, None);
    }
}
