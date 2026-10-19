use super::*;
use crate::cards;
use chrono::TimeZone;

fn at(hour: u32) -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 10, 14, hour, 30, 0).unwrap()
}

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

fn project() -> String {
    std::env::temp_dir().to_string_lossy().into_owned()
}

#[test]
fn a_request_becomes_a_preview_in_a_listed_folder() {
    let folders = vec![project()];
    let plan = plan_with("preview Sahi scenes 4 to 9", &folders, |prompt| {
        assert!(prompt.contains("<request"), "the request goes as data");
        Ok(format!(
            "```json\n{{\"kind\": \"preview\", \"project\": \"{}\", \"title\": \"Preview Sahi scenes 4 to 9\", \
             \"sub\": \"Sahi film \u{2014} every 4th frame\", \"what\": \"scenes 4 to 9\", \"minutes\": 70}}\n```",
            folders[0]
        ))
    })
    .unwrap();
    assert_eq!(plan.kind, Kind::Preview);
    assert_eq!(plan.what, "scenes 4 to 9");
    assert_eq!(plan.minutes, Some(70));
    assert!(!plan.sub.contains('\u{2014}'), "no em dashes");
}

#[test]
fn a_list_needs_items_and_every_job_a_folder() {
    let folders = vec![project()];
    let empty = plan_with("fix things", &folders, |_| {
        Ok(format!(
            "{{\"kind\": \"list\", \"project\": \"{}\", \"items\": []}}",
            folders[0]
        ))
    });
    assert!(empty.unwrap_err().to_string().contains("tasks"));
    let nowhere = plan_with("fix things", &folders, |_| {
        Ok("{\"kind\": \"list\", \"project\": \"/no/such/folder\", \"items\": [\"a\"]}".to_owned())
    });
    assert!(nowhere.unwrap_err().to_string().contains("project"));
    let finals = plan_with("render it", &folders, |_| {
        Ok(format!(
            "{{\"kind\": \"final\", \"project\": \"{}\"}}",
            folders[0]
        ))
    });
    assert!(
        finals.is_err(),
        "a final comes only from an approved preview"
    );
}

fn queued_preview(store: &Store) -> Job {
    let plan = Plan {
        kind: Kind::Preview,
        project: project(),
        title: "Preview Sahi scenes 4 to 9".into(),
        sub: "Sahi film".into(),
        what: "scenes 4 to 9".into(),
        items: Vec::new(),
        minutes: Some(70),
    };
    queue_plan(
        store,
        "preview Sahi scenes 4 to 9",
        &plan,
        at(2).timestamp_millis(),
    )
    .unwrap()
}

#[test]
fn tonight_lists_the_queue_and_the_morning_what_needs_the_person_first() {
    let store = store();
    let job = queued_preview(&store);
    let first = view(&store, at(3).timestamp_millis()).unwrap();
    assert_eq!(first.tonight.len(), 1);
    assert_eq!(first.tonight[0].end, "1 h 10 m");
    assert!(first.waiting.is_empty());

    let dir = std::env::temp_dir();
    set_running(&store, job.id, &dir, at(3).timestamp_millis()).unwrap();
    finish(
        &store,
        job.id,
        "done",
        &json!({ "summary": "Scene 7: frames 212 to 240 are black.", "needs": true }),
        at(4).timestamp_millis(),
    )
    .unwrap();
    let other = queued_preview(&store);
    set_running(&store, other.id, &dir, at(4).timestamp_millis()).unwrap();
    finish(
        &store,
        other.id,
        "done",
        &json!({ "summary": "6 shots, all clear" }),
        at(5).timestamp_millis(),
    )
    .unwrap();

    let later = view(&store, at(14).timestamp_millis()).unwrap();
    assert!(later.tonight.is_empty());
    assert_eq!(later.needs, 1);
    assert!(
        later.waiting[0].needs,
        "the one that needs them comes first"
    );
    assert_eq!(
        later.waiting[0].sub,
        "Scene 7: frames 212 to 240 are black."
    );
    assert_eq!(later.waiting[0].end, "1 h");
    assert_eq!(later.ran.as_deref(), Some("Ran from 03:30 to 05:30"));
}

#[test]
fn approving_a_preview_queues_its_final_and_redo_queues_it_again() {
    let store = store();
    let job = queued_preview(&store);
    let dir = std::env::temp_dir().join("night-preview-test");
    set_running(&store, job.id, &dir, at(3).timestamp_millis()).unwrap();
    finish(
        &store,
        job.id,
        "done",
        &json!({ "summary": "clear", "render_minutes": 38 }),
        at(4).timestamp_millis(),
    )
    .unwrap();

    let view = act(&store, job.id, "approve", None, at(14)).unwrap();
    assert_eq!(view.tonight.len(), 1);
    let final_job = get(&store, view.tonight[0].id).unwrap().unwrap();
    assert_eq!(final_job.kind, Kind::Final);
    assert_eq!(final_job.parent_id, Some(job.id));
    assert_eq!(final_job.title, "Final: Sahi scenes 4 to 9");
    assert_eq!(
        final_job.spec["preview_dir"],
        dir.to_string_lossy().as_ref()
    );
    assert_eq!(final_job.spec["minutes"], 152);
    assert!(view.waiting.is_empty(), "an approved preview is put away");

    let again = queued_preview(&store);
    set_running(&store, again.id, &dir, at(3).timestamp_millis()).unwrap();
    finish(
        &store,
        again.id,
        "done",
        &json!({}),
        at(4).timestamp_millis(),
    )
    .unwrap();
    let view = act(&store, again.id, "redo", Some("less blue haze"), at(14)).unwrap();
    let redo = get(&store, view.tonight.last().unwrap().id)
        .unwrap()
        .unwrap();
    assert_eq!(redo.kind, Kind::Preview);
    assert_eq!(redo.spec["note"], "less blue haze");
    assert_eq!(redo.sub, "less blue haze");
}

#[test]
fn a_queued_job_can_be_removed_and_a_wrong_button_is_refused() {
    let store = store();
    let job = queued_preview(&store);
    assert!(
        act(&store, job.id, "keep", None, at(3)).is_err(),
        "keep is for lists"
    );
    let view = act(&store, job.id, "remove", None, at(3)).unwrap();
    assert!(view.tonight.is_empty());
}

#[test]
fn it_starts_only_in_the_night_away_and_on_power() {
    let store = store();
    let away = Some(AWAY_SECONDS);
    assert!(
        !run::due(&store, at(3), away, true).unwrap(),
        "nothing queued"
    );
    queued_preview(&store);
    assert!(run::due(&store, at(3), away, true).unwrap() || run::is_running());
    assert!(
        !run::due(&store, at(15), away, true).unwrap(),
        "the afternoon"
    );
    assert!(
        !run::due(&store, at(3), Some(60), true).unwrap(),
        "they are at the keyboard"
    );
    assert!(!run::due(&store, at(3), away, false).unwrap(), "on battery");
}

const AWAY_SECONDS: u64 = run::AWAY_MIN * 60 + 1;

#[test]
fn durations_read_short() {
    assert_eq!(duration_label(25), "25 m");
    assert_eq!(duration_label(60), "1 h");
    assert_eq!(duration_label(70), "1 h 10 m");
    assert_eq!(duration_label(0), "1 m");
}

#[test]
fn claude_s_look_is_read_by_shot() {
    let verdicts = blender::verdicts_in(
        "Here: {\"shots\": [{\"name\": \"Scene 7\", \"state\": \"error\", \"note\": \"Two ribbons cut each other \u{2014} frames 212 on.\"}, \
         {\"name\": \"Scene 4\", \"state\": \"fine\", \"note\": \"\"}]}",
    );
    assert_eq!(verdicts.len(), 2);
    assert_eq!(verdicts[0].1, "error");
    assert!(!verdicts[0].2.contains('\u{2014}'));
    assert_eq!(verdicts[1].1, "ok", "anything unknown counts as ok");
}

#[test]
fn a_project_s_check_comes_from_its_package_scripts() {
    let dir = std::env::temp_dir().join(format!("night-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("package.json"),
        r#"{"scripts": {"build": "next build", "typecheck": "tsc --noEmit"}}"#,
    )
    .unwrap();
    assert_eq!(
        list::check_command(&dir).as_deref(),
        Some("npm run -s typecheck")
    );
    std::fs::write(
        dir.join("package.json"),
        r#"{"scripts": {"build": "next build"}}"#,
    )
    .unwrap();
    assert_eq!(
        list::check_command(&dir),
        None,
        "no tsc installed, no check"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_morning_card_says_what_needs_the_person() {
    let store = store();
    let job = queued_preview(&store);
    set_running(
        &store,
        job.id,
        &std::env::temp_dir(),
        at(3).timestamp_millis(),
    )
    .unwrap();
    finish(
        &store,
        job.id,
        "done",
        &json!({ "summary": "Scene 7 has black frames", "needs": true }),
        at(4).timestamp_millis(),
    )
    .unwrap();
    run::morning_card(&store, at(5).timestamp_millis()).unwrap();
    let card = cards::for_overlay(&store, at(5).timestamp_millis())
        .unwrap()
        .into_iter()
        .find(|card| card.kind == "night")
        .unwrap();
    assert_eq!(card.title, "Night shift: 1 ready, 1 need you");
    assert!(card.body.contains("Scene 7 has black frames"));
    assert!(!card.body.contains('\u{2014}'));
}

fn context() -> suggest::Context {
    suggest::Context {
        due: vec![suggest::Due {
            title: "Create persona pages".into(),
            when: None,
            who: Some("Ben Merton".into()),
            source: "the \u{201c}Website review\u{201d} meeting, Thu 8 Oct".into(),
            already: None,
        }],
        recent: vec![
            suggest::Recent {
                folder: "/work/unifize".into(),
                project: "Unifize Monorepo 2".into(),
                threads: vec![suggest::Thread {
                    title: "Platform page".into(),
                    asks: vec!["make the persona section responsive".into()],
                    left: "The hero is responsive. Not done: the persona rail still overflows \
                           at 768 px. Want me to fix that next?"
                        .into(),
                    waiting: true,
                    last_at: at(1).timestamp_millis(),
                }],
                last_at: at(1).timestamp_millis(),
                blender: false,
                git: true,
                code: true,
            },
            suggest::Recent {
                folder: "/films/sahi".into(),
                project: "Sahi Homepage Film".into(),
                threads: vec![suggest::Thread {
                    title: "Seller path scenes".into(),
                    asks: vec!["move on to scene 9".into()],
                    ..Default::default()
                }],
                last_at: at(0).timestamp_millis(),
                blender: true,
                git: false,
                code: false,
            },
        ],
        taken: vec!["Already queued".into()],
        repos: Default::default(),
    }
}

#[test]
fn picks_must_fit_their_kind_and_a_listed_folder() {
    let picks = suggest::pick_with(&context(), at(23), |system, prompt| {
        assert!(system.contains("none"), "none is a fine answer");
        assert!(prompt.contains("for Ben Merton") && prompt.contains("<recent_folders"));
        assert!(prompt.contains("code, git") && prompt.contains("Blender"));
        assert!(prompt.contains("ended waiting on them"));
        assert!(prompt.contains("Not done: the persona rail still overflows at 768 px"));
        assert!(prompt.contains("they asked: \u{201c}make the persona section responsive"));
        Ok(r#"{"jobs": [
            {"kind": "list", "folder": "/work/unifize", "title": "Persona pages for Ben", "why": "For Ben: persona pages, from Thursday's review", "items": ["Create the persona pages"], "minutes": 90},
            {"kind": "preview", "folder": "/work/unifize", "title": "No film here", "why": "x"},
            {"kind": "list", "folder": "/films/sahi", "title": "Not a repository", "items": ["a"]},
            {"kind": "preview", "folder": "/elsewhere", "title": "Not listed"},
            {"kind": "list", "folder": "/work/unifize", "title": "Already queued", "items": ["a"]},
            {"kind": "list", "folder": "/work/unifize", "title": "No items", "items": []},
            {"kind": "preview", "folder": "/films/sahi", "title": "Sahi scenes 9 to 12", "why": "You were moving through the scenes until 00:30", "what": "scenes 9 to 12"},
            {"kind": "preview", "folder": "/films/sahi", "title": "Third pick", "what": "a"},
            {"kind": "preview", "folder": "/films/sahi", "title": "Fourth pick", "what": "b"}
        ]}"#
        .to_owned())
    })
    .unwrap();
    let titles: Vec<&str> = picks.iter().map(|pick| pick.plan.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Persona pages for Ben", "Sahi scenes 9 to 12", "Third pick"]
    );
    assert_eq!(picks[0].plan.kind, Kind::List);
    assert_eq!(
        picks[0].why,
        "For Ben: persona pages, from Thursday's review"
    );
    assert_eq!(picks[1].plan.project, "/films/sahi");
}

#[test]
fn without_recent_work_nothing_is_picked_or_asked() {
    let empty = suggest::Context::default();
    let picks = suggest::pick_with(&empty, at(23), |_, _| panic!("Claude was asked")).unwrap();
    assert!(picks.is_empty());
}

#[test]
fn suggestions_wait_for_add_and_skip_puts_them_away() {
    let store = store();
    assert!(!view(&store, at(23).timestamp_millis()).unwrap().picks_fresh);
    let picks = suggest::pick_with(&context(), at(23), |_, _| {
        Ok(r#"{"jobs": [
            {"kind": "list", "folder": "/work/unifize", "title": "Persona pages for Ben", "why": "For Ben", "items": ["Create the persona pages"], "minutes": 90},
            {"kind": "preview", "folder": "/films/sahi", "title": "Sahi scenes 9 to 12", "why": "Where you left off", "what": "scenes 9 to 12"}
        ]}"#
        .to_owned())
    })
    .unwrap();
    suggest::save(&store, &picks, at(23).timestamp_millis()).unwrap();
    let shown = view(&store, at(23).timestamp_millis()).unwrap();
    assert!(shown.picks_fresh);
    assert!(shown.tonight.is_empty(), "nothing is queued without Add");
    assert_eq!(shown.suggested.len(), 2);
    assert_eq!(shown.suggested[0].sub, "For Ben");
    assert_eq!(shown.suggested[0].end, "1 h 30 m");

    let added = act(&store, shown.suggested[0].id, "add", None, at(23)).unwrap();
    assert_eq!(added.tonight.len(), 1);
    assert_eq!(added.tonight[0].title, "Persona pages for Ben");
    assert_eq!(added.suggested.len(), 1);
    let skipped = act(&store, added.suggested[0].id, "skip", None, at(23)).unwrap();
    assert!(skipped.suggested.is_empty());
    let context = suggest::gather(&store, at(23)).unwrap();
    assert!(
        context.taken.contains(&"Sahi scenes 9 to 12".to_owned()),
        "a skipped pick is not picked again today"
    );
    assert!(context.taken.contains(&"Persona pages for Ben".to_owned()));

    let quiet = Store::open_in_memory().unwrap();
    suggest::save(&quiet, &[], at(23).timestamp_millis()).unwrap();
    let shown = view(&quiet, at(23).timestamp_millis()).unwrap();
    assert!(
        shown.picks_fresh,
        "a day with no picks still counts as done"
    );
    assert!(shown.suggested.is_empty() && shown.tonight.is_empty());
}

#[test]
fn run_now_puts_a_pick_first_and_on_the_stack() {
    let store = store();
    let queued = queued_preview(&store);
    let picks = suggest::pick_with(&context(), at(15), |_, _| {
        Ok(r#"{"jobs": [{"kind": "list", "folder": "/work/unifize", "title": "Audit Factors integration", "why": "For Ben: Factors tracking", "items": ["Find where Factors loads"], "minutes": 40}]}"#.to_owned())
    })
    .unwrap();
    suggest::save(&store, &picks, at(15).timestamp_millis()).unwrap();
    let shown = view(&store, at(15).timestamp_millis()).unwrap();
    assert!(shown.picks_fresh);
    assert!(shown.now.is_empty());
    let pick = shown.suggested[0].id;

    let after = act(&store, pick, "run", None, at(15)).unwrap();
    assert!(after.suggested.is_empty());
    assert_eq!(after.now.len(), 1, "it shows on the to-do list's stack");
    assert_eq!(after.now[0].title, "Audit Factors integration");
    assert_eq!(
        after.tonight[0].id, pick,
        "it runs before what was queued for tonight"
    );
    assert_eq!(after.tonight[1].id, queued.id);
    assert_eq!(get(&store, pick).unwrap().unwrap().spec["now"], true);

    // Done and not looked at, it stays on the stack; put away, it leaves.
    set_running(
        &store,
        pick,
        &std::env::temp_dir(),
        at(15).timestamp_millis(),
    )
    .unwrap();
    finish(
        &store,
        pick,
        "done",
        &json!({ "summary": "Status written" }),
        at(16).timestamp_millis(),
    )
    .unwrap();
    let done = view(&store, at(17).timestamp_millis()).unwrap();
    assert_eq!(done.now[0].sub, "Status written");
    let away = act(&store, pick, "dismiss", None, at(17)).unwrap();
    assert!(away.now.is_empty());
}

#[test]
fn picks_go_stale_after_a_few_hours() {
    let store = store();
    suggest::save(&store, &[], at(10).timestamp_millis()).unwrap();
    assert!(view(&store, at(12).timestamp_millis()).unwrap().picks_fresh);
    assert!(!view(&store, at(15).timestamp_millis()).unwrap().picks_fresh);
}

#[test]
fn a_pick_opens_as_a_claude_code_session_and_leaves_the_stack() {
    let store = store();
    let picks = suggest::pick_with(&context(), at(15), |_, _| {
        Ok(r#"{"jobs": [{"kind": "list", "folder": "/work/unifize", "title": "Audit Factors integration", "why": "For Ben: Factors tracking — from the review", "items": ["Find where Factors loads", "Write FACTORS_STATUS.md for Ben"], "minutes": 40}]}"#.to_owned())
    })
    .unwrap();
    suggest::save(&store, &picks, at(15).timestamp_millis()).unwrap();
    let pick = view(&store, at(15).timestamp_millis()).unwrap().suggested[0].id;
    let job = get(&store, pick).unwrap().unwrap();

    let prompt = code_prompt(&job);
    assert!(prompt.starts_with("Audit Factors integration\nFor: For Ben: Factors tracking"));
    assert!(prompt.contains("1. Find where Factors loads\n2. Write FACTORS_STATUS.md for Ben"));
    assert!(!prompt.contains('\u{2014}'), "no em dashes");
    let link = code_link(&job).unwrap();
    assert!(
        link.starts_with("claude://code/new?folder=%2Fwork%2Funifize&q=Audit%20Factors")
            || link.starts_with("claude-cli://open?cwd=%2Fwork%2Funifize&q=")
    );
    assert!(!link.contains(' ') && !link.contains('\n'));

    let after = act(&store, pick, "code", None, at(15)).unwrap();
    assert!(
        after.suggested.is_empty(),
        "handed to Claude Code, it leaves the stack"
    );
    assert!(after.tonight.is_empty());
    let taken = suggest::gather(&store, at(15)).unwrap().taken;
    assert!(
        taken.contains(&"Audit Factors integration".to_owned()),
        "and is not picked again today"
    );
}

fn unifize_repo() -> suggest::Repo {
    suggest::Repo {
        project: "Unifize Monorepo 2".into(),
        paths: vec![
            "src/app/explorations/personas/quality-manager/page.tsx".into(),
            "src/app/explorations/personas/_shared/PersonaRailsPage.tsx".into(),
            "src/app/pricing/page.tsx".into(),
        ],
        dirty: vec!["src/app/explorations/personas/_shared/types.ts".into()],
        commits: vec!["2026-10-09 Persona rails: quality manager page".into()],
        sessions: vec!["Persona pages for QMs".into(), "Pricing table".into()],
    }
}

#[test]
fn terms_name_what_work_is_about() {
    assert_eq!(suggest::terms("Scaffold persona pages"), ["persona"]);
    assert_eq!(
        suggest::terms("Audit Factors integration status"),
        ["factor", "integration"]
    );
    assert_eq!(
        suggest::terms("Write tests for the case studies"),
        ["case", "study"]
    );
    assert!(suggest::terms("Create the page").is_empty());
}

#[test]
fn already_finds_what_a_repository_has_for_some_work() {
    let repo = unifize_repo();
    let line = suggest::already(&repo, &suggest::terms("Create persona pages")).unwrap();
    assert!(
        line.starts_with("2 files such as src/app/explorations/personas/quality-manager/page.tsx")
    );
    assert!(line.contains("1 changed and not committed"));
    assert!(line.contains("commits: 2026-10-09 Persona rails"));
    assert!(line.contains("sessions: Persona pages for QMs"));
    assert!(suggest::already(&repo, &suggest::terms("Contact form states")).is_none());
    assert!(suggest::already(&repo, &[]).is_none());
}

#[test]
fn owed_work_already_there_is_marked_for_the_pick() {
    let mut context = context();
    context.due[0].already =
        Some("Unifize Monorepo 2: 2 files such as src/app/explorations/personas/x.tsx".into());
    suggest::pick_with(&context, at(23), |system, prompt| {
        assert!(system.contains("never plan building it again"));
        assert!(prompt.contains("| already there: Unifize Monorepo 2: 2 files"));
        Ok(r#"{"jobs": []}"#.to_owned())
    })
    .unwrap();
}

#[test]
fn a_pick_that_would_redo_built_work_is_dropped_with_why() {
    let pick = |title: &str, items: &[&str]| suggest::Pick {
        plan: Plan {
            kind: Kind::List,
            project: "/work/unifize".into(),
            title: title.into(),
            sub: String::new(),
            what: String::new(),
            items: items.iter().map(|item| (*item).to_owned()).collect(),
            minutes: Some(60),
        },
        why: "For Ben".into(),
    };
    let repos = std::collections::HashMap::from([("/work/unifize".to_owned(), unifize_repo())]);
    let mut asked = Vec::new();
    let (kept, dropped) = suggest::check_with(
        vec![
            pick("Scaffold persona pages", &["Create a page per persona"]),
            pick("Contact form states", &["Add the error state"]),
            pick("Pricing table copy", &["Tighten the plan names"]),
        ],
        &repos,
        |system, prompt| {
            assert!(system.contains("redo"));
            asked.push(prompt.to_owned());
            Ok(if prompt.contains("persona") {
                r#"{"redo": true, "why": "Persona pages are built under explorations/personas"}"#
            } else {
                r#"{"redo": false, "why": "only the copy is missing"}"#
            }
            .to_owned())
        },
    );
    // Nothing in the repository names the contact form: kept, not asked.
    assert_eq!(asked.len(), 2);
    assert!(asked[0].contains("<already_there") && asked[0].contains("quality-manager/page.tsx"));
    let kept: Vec<&str> = kept.iter().map(|pick| pick.plan.title.as_str()).collect();
    assert_eq!(kept, ["Contact form states", "Pricing table copy"]);
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].0.plan.title, "Scaffold persona pages");
    assert_eq!(
        dropped[0].1,
        "Persona pages are built under explorations/personas"
    );
}

#[test]
fn a_failed_check_keeps_the_pick() {
    let repos = std::collections::HashMap::from([("/work/unifize".to_owned(), unifize_repo())]);
    let pick = suggest::Pick {
        plan: Plan {
            kind: Kind::List,
            project: "/work/unifize".into(),
            title: "Persona tests".into(),
            sub: String::new(),
            what: String::new(),
            items: vec!["Test the persona rails".into()],
            minutes: None,
        },
        why: "x".into(),
    };
    let (kept, dropped) = suggest::check_with(vec![pick], &repos, |_, _| anyhow::bail!("offline"));
    assert_eq!(kept.len(), 1);
    assert!(dropped.is_empty());
}

#[test]
fn a_session_s_end_says_what_was_asked_and_what_is_left() {
    let lines = [
        r#"{"partial": "cut off"#.to_owned(),
        json!({"type": "user", "cwd": "/work/unifize", "message": {"content": "build the pricing table"}}).to_string(),
        json!({"type": "assistant", "cwd": "/work/unifize", "message": {"content": [{"type": "text", "text": "Started."}]}}).to_string(),
        json!({"type": "user", "cwd": "/work/unifize", "message": {"content": [{"type": "tool_result", "content": "ok"}]}}).to_string(),
        json!({"type": "user", "cwd": "/work/unifize", "isMeta": true, "message": {"content": "<command-name>/clear</command-name>"}}).to_string(),
        json!({"type": "user", "cwd": "/work/unifize/apps/web", "message": {"content": [{"type": "text", "text": "now   the FAQ\nsection"}, {"type": "image"}]}}).to_string(),
        json!({"type": "assistant", "cwd": "/work/unifize/apps/web", "message": {"content": [{"type": "text", "text": "FAQ is in. Not done: the mobile accordion. Want me to do it?"}]}}).to_string(),
    ];
    let facts = suggest::read_tail(&lines.join("\n")).unwrap();
    assert_eq!(facts.folder, "/work/unifize/apps/web");
    assert_eq!(
        facts.asks,
        ["build the pricing table", "now the FAQ section"]
    );
    assert_eq!(
        facts.left,
        "FAQ is in. Not done: the mobile accordion. Want me to do it?"
    );
    assert!(facts.waiting, "it ended with a question");

    let answered = format!(
        "{}\n{}",
        lines.join("\n"),
        json!({"type": "user", "cwd": "/w", "message": {"content": "yes"}})
    );
    assert!(!suggest::read_tail(&answered).unwrap().waiting);
    assert!(
        suggest::read_tail("not json").is_none(),
        "no folder, no facts"
    );
}

#[test]
fn films_are_found_by_the_blend_files_a_session_opened() {
    let home = std::env::temp_dir().join(format!("night-films-{}", std::process::id()));
    let chapter = home.join("Downloads/freebyrd-film/ch1");
    for step in ["step6-kickoff", "step7-booked"] {
        std::fs::create_dir_all(chapter.join(step)).unwrap();
        std::fs::write(chapter.join(step).join(format!("{step}.blend")), "x").unwrap();
    }
    std::fs::create_dir_all(home.join("Downloads/platform-film/step4")).unwrap();
    std::fs::write(home.join("Downloads/platform-film/step4/seal.blend"), "x").unwrap();
    std::fs::write(home.join("Downloads/loose.blend"), "x").unwrap();
    let h = home.to_string_lossy();
    let transcript = format!(
        r#"{{"text": "open {h}/Downloads/platform-film/step4/seal.blend"}}
{{"command": "Blender -b {h}/Downloads/freebyrd-film/ch1/step6-kickoff/step6-kickoff.blend -P x.py"}}
{{"text": "saved {h}/Downloads/freebyrd-film/ch1/step7-booked/step7-booked.blend and {h}/Downloads/loose.blend and {h}/Downloads/gone/x.blend"}}"#
    );
    let blends = suggest::blend_paths(transcript.as_bytes());
    assert_eq!(blends.len(), 4, "only files that exist: {blends:?}");
    assert!(blends[0].ends_with("loose.blend"), "newest mention first");
    let films = suggest::films_in(&blends, &home);
    assert_eq!(
        films,
        [
            chapter.to_string_lossy().into_owned(),
            home.join("Downloads/platform-film/step4")
                .to_string_lossy()
                .into_owned(),
        ]
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_session_deep_in_a_project_counts_as_the_project() {
    let root = std::env::temp_dir().join(format!("night-root-{}", std::process::id()));
    let ui = root.join("context-app/crates/freebyrd/ui");
    std::fs::create_dir_all(&ui).unwrap();
    std::fs::write(root.join("context-app/Cargo.toml"), "[workspace]").unwrap();
    std::fs::write(
        root.join("context-app/crates/freebyrd/Cargo.toml"),
        "[package]",
    )
    .unwrap();
    assert_eq!(
        suggest::project_root(&ui.to_string_lossy()),
        root.join("context-app").to_string_lossy()
    );
    let notes = root.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    assert_eq!(
        suggest::project_root(&notes.to_string_lossy()),
        notes.to_string_lossy(),
        "no manifest: the folder itself"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn one_shared_word_is_not_work_already_there() {
    let pulsedeck = suggest::Repo {
        project: "pulsedeck".into(),
        paths: vec![
            "src/tracks/TrackList.tsx".into(),
            "src/audio/analyser.ts".into(),
        ],
        ..Default::default()
    };
    let words = suggest::terms("Track Factors integration");
    assert!(suggest::already(&pulsedeck, &words).is_none());
    let unifize = suggest::Repo {
        project: "Unifize".into(),
        paths: vec!["src/lib/factors/integration.ts".into()],
        ..Default::default()
    };
    assert!(suggest::already(&unifize, &words).is_some());
}

#[test]
fn the_panel_says_why_nothing_was_picked() {
    let store = store();
    let picked = suggest::picked_with(&context(), at(23), |_, _| {
        Ok(
            r#"{"jobs": [], "none": "Every session finished or waits on a design choice"}"#
                .to_owned(),
        )
    })
    .unwrap();
    assert!(picked.picks.is_empty());
    let note = suggest::Note::Nothing(picked.none.unwrap());
    suggest::save_noted(&store, &[], Some(&note), at(23).timestamp_millis()).unwrap();
    let shown = view(&store, at(23).timestamp_millis()).unwrap();
    assert_eq!(
        shown.picks_note.as_deref(),
        Some("Every session finished or waits on a design choice")
    );

    // A failure says so, and leaves what was suggested before in place.
    let picks = suggest::pick_with(&context(), at(23), |_, _| {
        Ok(r#"{"jobs": [{"kind": "preview", "folder": "/films/sahi", "title": "Sahi scenes", "what": "9 to 12"}]}"#.to_owned())
    })
    .unwrap();
    suggest::save(&store, &picks, at(23).timestamp_millis()).unwrap();
    let failed = suggest::Note::Failed("Claude did not answer".into());
    suggest::save_noted(&store, &[], Some(&failed), at(23).timestamp_millis() + 1).unwrap();
    let shown = view(&store, at(23).timestamp_millis() + 2).unwrap();
    assert_eq!(shown.suggested.len(), 1);
    assert_eq!(shown.picks_note, None, "a waiting pick needs no note");
    store
        .conn()
        .execute("DELETE FROM night_job WHERE state = 'suggested'", [])
        .unwrap();
    let shown = view(&store, at(23).timestamp_millis() + 2).unwrap();
    assert_eq!(
        shown.picks_note.as_deref(),
        Some("Picking failed: Claude did not answer")
    );
}

fn git_in(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_project_without_git_is_worked_on_a_copy_and_kept_all_or_nothing() {
    let root = std::env::temp_dir().join(format!("night-copy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(project.join("target/debug")).unwrap();
    std::fs::write(project.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(project.join("src/lib.rs"), "fn a() {}\n").unwrap();
    std::fs::write(project.join("target/debug/big"), "build output").unwrap();
    let tree = root.join("job/repo");
    std::fs::create_dir_all(root.join("job")).unwrap();
    list::start_copy(&project, &tree).unwrap();
    assert!(tree.join("src/lib.rs").is_file());
    assert!(!tree.join("target").exists(), "build output is not copied");
    let base = String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&tree)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    // The night's item.
    std::fs::write(tree.join("src/lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    std::fs::write(tree.join("src/new.rs"), "fn c() {}\n").unwrap();
    git_in(&tree, &["add", "-A"]);
    git_in(&tree, &["commit", "-q", "-m", "item"]);
    let result = json!({
        "base": base.trim(),
        "source": "copy",
        "repo": project.to_string_lossy(),
        "tree": tree.to_string_lossy(),
    });

    // The person changed the same file since: nothing is applied.
    std::fs::write(project.join("src/lib.rs"), "fn changed() {}\n").unwrap();
    let error = list::keep_result(&result).unwrap_err().to_string();
    assert!(error.starts_with("Nothing was changed"), "{error}");
    assert_eq!(
        std::fs::read_to_string(project.join("src/lib.rs")).unwrap(),
        "fn changed() {}\n"
    );
    assert!(!project.join("src/new.rs").exists());
    assert!(tree.is_dir(), "the night's work stays");

    std::fs::write(project.join("src/lib.rs"), "fn a() {}\n").unwrap();
    list::keep_result(&result).unwrap();
    assert_eq!(
        std::fs::read_to_string(project.join("src/lib.rs")).unwrap(),
        "fn a() {}\nfn b() {}\n"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("src/new.rs")).unwrap(),
        "fn c() {}\n"
    );
    assert!(!tree.exists(), "the copy is gone once kept");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_git_project_s_night_starts_from_uncommitted_work_and_keeps_it_uncommitted() {
    let root = std::env::temp_dir().join(format!("night-git-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git_in(&repo, &["init", "-q"]);
    git_in(&repo, &["config", "user.name", "Test"]);
    git_in(&repo, &["config", "user.email", "test@example.invalid"]);
    std::fs::write(repo.join("page.tsx"), "one\n").unwrap();
    git_in(&repo, &["add", "-A"]);
    git_in(&repo, &["commit", "-q", "-m", "first"]);
    // Yesterday's work, never committed.
    std::fs::write(repo.join("page.tsx"), "one\ntwo\n").unwrap();
    std::fs::write(repo.join("rail.tsx"), "rail\n").unwrap();

    let job_dir = root.join("job");
    std::fs::create_dir_all(&job_dir).unwrap();
    let tree = job_dir.join("repo");
    let note = list::start_worktree(&repo, &tree, "night/test-1", &job_dir).unwrap();
    assert_eq!(note, None);
    assert_eq!(
        std::fs::read_to_string(tree.join("page.tsx")).unwrap(),
        "one\ntwo\n"
    );
    assert_eq!(
        std::fs::read_to_string(tree.join("rail.tsx")).unwrap(),
        "rail\n"
    );
    let head = |dir: &std::path::Path| {
        String::from_utf8(
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned()
    };
    let base = head(&tree);
    std::fs::write(tree.join("rail.tsx"), "rail\nresponsive\n").unwrap();
    git_in(&tree, &["commit", "-q", "-am", "item"]);

    list::keep_result(&json!({
        "base": base,
        "source": "git",
        "branch": "night/test-1",
        "repo": repo.to_string_lossy(),
        "tree": tree.to_string_lossy(),
    }))
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.join("rail.tsx")).unwrap(),
        "rail\nresponsive\n"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("page.tsx")).unwrap(),
        "one\ntwo\n"
    );
    let status = String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["log", "--oneline"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(
        status.lines().count(),
        1,
        "nothing was committed on their branch"
    );
    assert!(!tree.exists(), "the worktree is gone");
    let branches = String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["branch"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(!branches.contains("night/test-1"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_session_still_running_is_marked_and_steps_must_stand_alone() {
    let mut context = context();
    context.recent[0].threads[0].last_at = at(23).timestamp_millis() - 5 * 60_000;
    suggest::pick_with(&context, at(23), |system, prompt| {
        assert!(prompt.contains("\u{201c}Platform page\u{201d}, last worked"));
        assert!(prompt.contains(", still running"));
        assert!(
            !prompt.contains("ended waiting on them"),
            "running, not waiting"
        );
        assert!(system.contains("without the sessions"));
        Ok(r#"{"jobs": [], "none": "x"}"#.to_owned())
    })
    .unwrap();
}
