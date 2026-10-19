//! Command-line access to Freebyrd's memory, for testing and for running a
//! step by hand. The app does the same in the background.

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};
use clap::{Parser, Subcommand};

use memory::routines::{self, jobs};
use memory::{
    Store, cards, connectors, drafts, ingest, moments, plans, promises, recall, recap, screen, time,
};

#[derive(Parser)]
#[command(about = "Freebyrd's memory: read sources, show a day, write recaps")]
struct Cli {
    /// The database (default: ~/Library/Application Support/Freebyrd/memory.db).
    #[arg(long)]
    db: Option<PathBuf>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Read what is new from Claude Code, browser history and saved pages.
    Ingest,
    /// Counts of what is stored.
    Stats,
    /// The text Claude reads for a day's recap (default: yesterday).
    Digest {
        #[arg(long)]
        day: Option<NaiveDate>,
    },
    /// Write a day's recap with Claude and add its card (default: yesterday).
    Recap {
        #[arg(long)]
        day: Option<NaiveDate>,
        /// Write it again even if it exists.
        #[arg(long)]
        force: bool,
    },
    /// What the notch would send to Claude with a question.
    Recall {
        question: String,
        /// The title of the window in front, as a hint.
        #[arg(long)]
        window: Option<String>,
    },
    /// The cards the notch would show now.
    Cards,
    /// What the notch's Up next line shows now. With an action (done, hide)
    /// and a plan, presses that button first.
    UpNext {
        action: Option<String>,
        plan: Option<i64>,
    },
    /// The quick asks the notch would offer over an app and window.
    Quick {
        #[arg(default_value = "Arc")]
        app: String,
        #[arg(long)]
        window: Option<String>,
        #[arg(long)]
        url: Option<String>,
    },
    /// Plan the rest of the working day as blocks, each a card to add, as
    /// the notch's "Plan my day" does (calls Claude). With a note, changes
    /// the latest plan as it asks.
    Plan {
        #[arg(long)]
        note: Option<String>,
        /// Only print what Claude would get about the day, without calling it.
        #[arg(long)]
        facts: bool,
    },
    /// Events on the calendar from before agendas, still to come. With
    /// --preview, prints the notes each would get (calls Claude, not the
    /// calendar); with --apply, adds each again with them and removes the
    /// old event. An event moved in Calendar goes back to the time Freebyrd
    /// has.
    Agendas {
        #[arg(long, conflicts_with = "apply")]
        preview: bool,
        #[arg(long)]
        apply: bool,
    },
    /// Look for events in some text as if it had just appeared on screen,
    /// and add nudges for them (calls Claude). "\n" in the text is a new line.
    Screen {
        text: String,
        #[arg(long, default_value = "Test")]
        app: String,
        #[arg(long, default_value = "")]
        window: String,
    },
    /// Look for promises in the work conversations' new messages (calls Claude).
    Promises,
    /// Read the work threads with new messages for what the person waits on
    /// (calls Claude); the result is on the to-do list's Waiting on.
    Asks,
    /// Move the list's items by what the work did: Claude Code sessions
    /// quiet for 10 minutes and new messages in threads tied to an item
    /// (calls Claude). `--preview` prints what it would send, calling
    /// nothing and marking nothing read.
    Progress {
        #[arg(long)]
        preview: bool,
    },
    /// Drafts before a due date (step 5): list (default; what gets a draft
    /// and when), preview PLAN (what drafting it would send, without
    /// Claude), write PLAN (drafts it now and puts up its card; calls
    /// Claude), run (one pass, as the worker does).
    Drafts {
        #[arg(default_value = "list")]
        action: String,
        plan: Option<i64>,
    },
    /// Press a card's button: add, undo, work, personal; yes, no, run,
    /// pause, resume, remove.
    Act { card: i64, action: String },
    /// Read from the connectors now: fireflies, notion, or both.
    Connectors {
        #[arg(default_value = "both")]
        which: String,
    },
    /// What the detectors find in the memory now: work the person repeats
    /// (asks no model).
    Patterns {
        /// Labels and counts only, without examples and facts.
        #[arg(long)]
        brief: bool,
    },
    /// Look for repeated work and draft fixes for new patterns with Claude,
    /// as suggestion cards.
    Suggest {
        /// At most this many drafts.
        #[arg(long, default_value_t = routines::DRAFTS_A_DAY)]
        max: usize,
        /// Instead, add two clearly named test suggestions: a skill and a
        /// scheduled job.
        #[arg(long)]
        test: bool,
        /// Remove the test suggestions and everything set up from them.
        #[arg(long, conflicts_with = "test")]
        clean_test: bool,
    },
    /// The approved skills and scheduled jobs, with their last runs.
    Jobs,
    /// An approved job: run (now, here, as launchd does), start (through
    /// launchd, as Run now does), pause, resume or remove.
    Job { action: String, name: String },
    /// Jev (TypeSafe) shadows: report USE (a shadow's log beside what
    /// happened); backtest USE (the shadow over past cases the person already
    /// answered; sends them to TypeSafe); usage (today's tokens by use,
    /// beside the budget and the shadows' shares; sends nothing).
    Jev {
        action: String,
        name: Option<String>,
    },
    /// Night shift (`night`): `list` (the default) shows tonight's queue and
    /// what waits from last night; `add TEXT` queues a job (Claude turns it
    /// into a preview or a list); `run [ID]` works through the queue now (or
    /// one job), in this process (the app starts it under caffeinate); `start` starts it in the
    /// background; `act ID ACTION [VALUE]` presses a job's button; `show ID`
    /// prints what a job made; `card` leaves the morning card again; `stop`
    /// picks tonight's work again and offers it on the easy-stop card now,
    /// outside its window (calls Claude); `json` and `page ID` print what
    /// the notch's panel gets.
    Night {
        action: Option<String>,
        args: Vec<String>,
    },
    /// Ideas at the moment they help (`moments`). Without a title: the
    /// moments of the last `--days`. With one, weighs a moment now as if it
    /// were in front: `--url` (and `--ask`, what the person typed, on a chat
    /// site) for a page, `--app` for a window, or `--claude-code` for the
    /// last request typed to Claude Code (or `--ask`, as if typed now).
    /// `--dry` prints the ranked ideas
    /// without recording the moment or asking Claude; without it, a card can
    /// arrive (use a copy of the memory with --db).
    Moments {
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        ask: Option<String>,
        #[arg(long)]
        claude_code: bool,
        #[arg(long)]
        dry: bool,
        #[arg(long, default_value_t = 2)]
        days: i64,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// The to-do list behind the notch's checklist icon. `sync` lists new
    /// meeting action items first; `add TEXT` adds a to-do (a day in it
    /// gives it a date); `act REF ACTION [VALUE]` presses a to-do's button
    /// (`plan:12 done`, `todo:3 date 2026-10-15`); `focus REF` prints what
    /// its Ask would send (no Claude); `json` prints the list as the notch
    /// gets it.
    Todos {
        action: Option<String>,
        args: Vec<String>,
    },
    /// People: with no action, everyone in the People tab; `page NAME|ID`,
    /// a page; `sync`, join what is new; `memory NAME|ID`, what a question
    /// about them sends; `merge FROM INTO`, `split ALIAS`, `claim ALIAS ID`,
    /// `hide ID`, `show ID`, `note ID TEXT`.
    People {
        action: Option<String>,
        args: Vec<String>,
    },
    /// The brief a Claude Code session starting in a folder would get from
    /// the memory (plain code, no Claude), as of now, a past time
    /// (`--as-of "2026-10-13 21:30"`), or a session's start (`--session
    /// ID`). `--index` gives the short index of live threads instead.
    /// Opens the memory read-only.
    Brief {
        #[arg(long)]
        folder: PathBuf,
        #[arg(long)]
        index: bool,
        #[arg(long, conflicts_with = "session")]
        as_of: Option<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// One live thread's details, as a session would get them on asking:
    /// by its key from the index, or `--query WORDS`. The same folder and
    /// times as `brief`; opens the memory read-only.
    Thread {
        key: Option<String>,
        #[arg(long)]
        folder: PathBuf,
        #[arg(long, conflicts_with = "key")]
        query: Option<String>,
        #[arg(long, conflicts_with = "session")]
        as_of: Option<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// The context connector (README "Context connector"): `consents` (the
    /// default: standing answers and asks waiting), `receipts [--day D]
    /// [--item ITEM]` (what went, never its text), `report [--days N]` (the
    /// two weeks' measure, counts only), `revoke PROJECT` (its next ask
    /// waits for the notch again). Allowing is the notch's alone.
    Connector {
        action: Option<String>,
        args: Vec<String>,
        #[arg(long)]
        day: Option<NaiveDate>,
        #[arg(long)]
        item: Option<String>,
        #[arg(long, default_value_t = 14)]
        days: i64,
    },
}

fn connector_command(
    db: Option<&std::path::Path>,
    action: Option<&str>,
    args: &[String],
    day: Option<NaiveDate>,
    item: Option<&str>,
    days: i64,
) -> Result<()> {
    use memory::connector::{Connector, TOOL};
    let connector = Connector::open_default()?;
    let now = Local::now();
    match action.unwrap_or("consents") {
        "consents" => {
            let consents = connector.consents()?;
            if consents.is_empty() {
                println!("No standing answers.");
            }
            for (tool, project, kind, at) in consents {
                println!("{tool}: {project}: {kind} (since {})", time::moment(at));
            }
            for ask in connector.waiting()? {
                println!(
                    "{}: {} waits for the notch (asked {})",
                    ask.tool,
                    ask.project,
                    time::moment(ask.asked_at)
                );
            }
        }
        "receipts" => {
            let receipts = match item {
                Some(item) => connector.with_item(item)?,
                None => {
                    let day = day.unwrap_or_else(|| time::working_day(now));
                    let span = time::day_span(day);
                    connector
                        .receipts(TOOL, span.start_ms, None)?
                        .into_iter()
                        .filter(|receipt| receipt.at < span.end_ms)
                        .collect()
                }
            };
            if receipts.is_empty() {
                println!("No receipts.");
            }
            for receipt in receipts {
                let session = receipt.session_id.as_deref().unwrap_or("-");
                println!(
                    "#{} {} {} {} session {} {} {}: {} items, {} characters, sha256 {}",
                    receipt.id,
                    time::moment(receipt.at),
                    receipt.tool,
                    receipt.project,
                    session.get(..8).unwrap_or(session),
                    receipt.ask,
                    receipt.detail,
                    receipt.items.len(),
                    receipt.chars,
                    receipt.sha256.get(..16).unwrap_or(&receipt.sha256)
                );
            }
        }
        "report" => {
            let path = match db {
                Some(path) => path.to_path_buf(),
                None => Store::default_path().context("HOME is not set")?,
            };
            let store = Store::open_read_only(&path)?;
            for line in memory::connector::report(&store, &connector, days, now)? {
                println!("{line}");
            }
        }
        "revoke" => {
            let project = args.join(" ");
            anyhow::ensure!(!project.is_empty(), "which project: revoke PROJECT");
            let taken = connector.revoke(TOOL, &project, now.timestamp_millis())?;
            println!("{taken} answers taken back for {project}; its next ask waits for the notch.");
        }
        other => {
            anyhow::bail!("unknown connector action {other:?}: consents, receipts, report, revoke")
        }
    }
    Ok(())
}

/// A time given on the command line: "2026-10-13 21:30" (local), with
/// seconds or a "T", RFC 3339, or milliseconds since the Unix epoch.
fn parse_time(text: &str) -> Result<chrono::DateTime<Local>> {
    use chrono::{NaiveDateTime, TimeZone};
    let text = text.trim();
    if let Ok(ms) = text.parse::<i64>() {
        return time::local(ms).context("no such time");
    }
    if let Ok(at) = chrono::DateTime::parse_from_rfc3339(text) {
        return Ok(at.with_timezone(&Local));
    }
    [
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%dT%H:%M:%S",
    ]
    .iter()
    .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
    .and_then(|at| Local.from_local_datetime(&at).earliest())
    .with_context(|| format!("{text:?} is not a time like \"2026-10-13 21:30\""))
}

/// The memory opened read-only, and the time a brief or thread is as of:
/// `--as-of`, a session's start, or now.
fn read_only_as_of(
    db: Option<&std::path::Path>,
    as_of: Option<&str>,
    session: Option<&str>,
) -> Result<(Store, chrono::DateTime<Local>)> {
    let path = match db {
        Some(path) => path.to_path_buf(),
        None => Store::default_path().context("HOME is not set")?,
    };
    let store = Store::open_read_only(&path)?;
    let at = match (as_of, session) {
        (Some(text), _) => parse_time(text)?,
        (None, Some(id)) => {
            let started: Option<i64> = store
                .conn()
                .query_row(
                    "SELECT started_at FROM session WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .with_context(|| format!("no session {id} in the memory"))?;
            time::local(started.context("that session has no start")?).context("no such time")?
        }
        (None, None) => Local::now(),
    };
    Ok((store, at))
}

fn brief_command(
    db: Option<&std::path::Path>,
    folder: &std::path::Path,
    index: bool,
    as_of: Option<&str>,
    session: Option<&str>,
) -> Result<()> {
    let (store, at) = read_only_as_of(db, as_of, session)?;
    let folder = std::path::absolute(folder)?;
    if index {
        let Some(threads) = memory::threads::build(&store, &folder.to_string_lossy(), at)? else {
            println!("{} is no project of yours: no index.", folder.display());
            return Ok(());
        };
        let text = threads.index();
        if text.is_empty() {
            println!(
                "Nothing live in {} as of {}.",
                threads.project,
                time::moment(threads.as_of)
            );
        } else {
            println!("{text}\n\n({} characters)", text.chars().count());
        }
        return Ok(());
    }
    let Some(brief) = memory::brief::build(&store, &folder.to_string_lossy(), at)? else {
        println!("{} is no project of yours: no brief.", folder.display());
        return Ok(());
    };
    let text = brief.text();
    if text.is_empty() {
        println!(
            "Nothing in the memory for {} as of {}.",
            brief.project,
            time::moment(brief.as_of)
        );
    } else {
        println!("{text}\n\n({} characters)", text.chars().count());
    }
    Ok(())
}

fn thread_command(
    db: Option<&std::path::Path>,
    folder: &std::path::Path,
    ask: &str,
    as_of: Option<&str>,
    session: Option<&str>,
) -> Result<()> {
    let (store, at) = read_only_as_of(db, as_of, session)?;
    let folder = std::path::absolute(folder)?;
    let Some(threads) = memory::threads::build(&store, &folder.to_string_lossy(), at)? else {
        println!("{} is no project of yours: no threads.", folder.display());
        return Ok(());
    };
    match threads.find(ask) {
        Some(thread) => {
            let text = threads.details(&thread);
            println!("{text}\n\n({} characters)", text.chars().count());
        }
        None => println!(
            "No thread for {ask:?} in {} as of {}.",
            threads.project,
            time::moment(threads.as_of)
        ),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn moments_command(
    store: &Store,
    title: Option<&str>,
    url: Option<&str>,
    app: Option<&str>,
    ask: Option<&str>,
    claude_code: bool,
    dry: bool,
    days: i64,
    limit: usize,
) -> Result<()> {
    let now = Local::now();
    let moment = if claude_code {
        // `--ask` stands in for a request, as if just typed here.
        let latest = match ask {
            Some(text) => memory::ingest::claude_code::LatestPrompt {
                session_id: "freebyrd-memory".to_owned(),
                cwd: std::env::current_dir()?.to_string_lossy().into_owned(),
                text: text.to_owned(),
                at: now.timestamp_millis(),
            },
            None => memory::ingest::claude_code::latest_prompt(now.timestamp_millis() - 86_400_000)
                .context("no request typed to Claude Code in the last day")?,
        };
        Some(
            moments::Moment::from_prompt("Terminal", &latest, title.unwrap_or_default())
                .context("that request is no moment")?,
        )
    } else if let Some(title) = title {
        let moment = match (url, app) {
            (Some(url), _) => moments::Moment::from_page(app.unwrap_or("Arc"), url, title, ask),
            (None, Some(app)) => moments::Moment::from_window(app, title),
            (None, None) => anyhow::bail!("say where with --url or --app"),
        };
        Some(moment.context("that is no moment: a quiet site or app, or an empty title")?)
    } else {
        None
    };
    let Some(moment) = moment else {
        let lines = moments::report(store, days, now.timestamp_millis())?;
        if lines.is_empty() {
            println!("No moments in the last {days} days.");
        }
        lines.iter().for_each(|line| println!("{line}"));
        return Ok(());
    };
    let helpers = moments::helpers(store);
    let started = std::time::Instant::now();
    if dry {
        for line in moments::preview(store, &moment, now, &helpers, limit)? {
            println!("{line}");
        }
    } else {
        match moments::weigh(store, &moment, now, &helpers)? {
            Some(line) => println!("{line}"),
            None => println!(
                "Skipped: this moment was weighed in the last day, the day's cards are used, or \
                 the place is quiet."
            ),
        }
    }
    println!("({:.1} s)", started.elapsed().as_secs_f64());
    Ok(())
}

fn print_item(item: &memory::todos::Item) {
    let who = item
        .person
        .as_ref()
        .map(|chip| format!(" [{} #{}]", chip.name, chip.id))
        .or_else(|| item.who.as_ref().map(|who| format!(" [{who}]")))
        .unwrap_or_default();
    let when = item.when.as_deref().unwrap_or("no date");
    let buttons: Vec<&str> = item.actions.iter().map(|a| a.id.as_str()).collect();
    println!(
        "  {} {}{who}, {when}\n      from {} {buttons:?}",
        item.id, item.title, item.source
    );
    if let Some(latest) = &item.latest {
        let next = latest
            .next
            .as_deref()
            .map(|next| format!(" Next: {next}"))
            .unwrap_or_default();
        println!(
            "      latest ({}, {}, {}): {}{next}",
            latest.state, latest.when, latest.from, latest.note
        );
    }
}

fn print_list(list: &memory::todos::List) {
    println!(
        "Badge: {} (overdue: {}), {} open",
        list.badge.count, list.badge.overdue, list.badge.open
    );
    for group in &list.mine {
        println!("{}", group.label);
        for item in &group.items {
            print_item(item);
        }
    }
    if !list.waiting.is_empty() {
        println!("Waiting on");
        for item in &list.waiting {
            print_item(item);
        }
    }
    if !list.done.is_empty() {
        println!("Done");
        for item in &list.done {
            print_item(item);
        }
    }
}

fn todos_command(store: &Store, action: Option<&str>, args: &[String]) -> Result<()> {
    use memory::todos;
    let now = Local::now();
    let list = match action {
        None => todos::list(store, now)?,
        Some("sync") => {
            println!("{} new from meetings", todos::sync(store, now)?);
            todos::list(store, now)?
        }
        Some("add") => todos::add(store, &args.join(" "), now)?,
        Some("act") => {
            let reference = args.first().context("which to-do: plan:12, todo:3")?;
            let action = args
                .get(1)
                .context("which button: done, undo, hide, remove, calendar, date")?;
            todos::act(
                store,
                reference,
                action,
                args.get(2).map(String::as_str),
                now,
            )?
        }
        Some("json") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&todos::list(store, now)?)?
            );
            return Ok(());
        }
        Some("focus") => {
            let reference = args
                .first()
                .context("which to-do: listed:12, todo:3, person:4")?;
            let focus = todos::focus(store, reference, now)?;
            println!(
                "{}\n\n(finds related memory by: {})",
                focus.memory, focus.topic
            );
            return Ok(());
        }
        Some(other) => anyhow::bail!("unknown todos action {other:?}: sync, add, act, focus, json"),
    };
    print_list(&list);
    Ok(())
}

/// A person by ID, or by a name or address of theirs.
fn person_id(store: &Store, which: Option<&String>) -> Result<i64> {
    let which = which.context("which person: an ID or a name")?;
    if let Ok(id) = which.parse() {
        return Ok(id);
    }
    memory::people::Directory::load(store)?
        .resolve(which)
        .with_context(|| format!("no one known as {which:?}"))
}

fn people_command(store: &Store, action: Option<&str>, args: &[String]) -> Result<()> {
    use memory::people;
    let now = Local::now();
    match action {
        None | Some("list") => {
            for person in memory::todos::list(store, now)?.people {
                let mark = if person.needs_you { "*" } else { " " };
                println!("{mark} #{} {}: {}", person.id, person.name, person.detail);
            }
        }
        Some("sync") => println!("{} changed", people::sync(store, now.timestamp_millis())?),
        Some("page") => {
            let page = people::page(store, person_id(store, args.first())?, now)?;
            println!("{}", serde_json::to_string_pretty(&page)?);
        }
        Some("memory") => println!(
            "{}",
            people::memory(store, person_id(store, args.first())?, now)?
        ),
        Some("merge") => {
            let from = person_id(store, args.first())?;
            let into = person_id(store, args.get(1))?;
            people::merge(store, from, into)?;
            println!("#{from} is now #{into}");
        }
        Some("split") => {
            let alias = args.first().context("which name or address")?;
            println!(
                "now #{}",
                people::split(store, alias, now.timestamp_millis())?
            );
        }
        Some("claim") => {
            let alias = args.first().context("which name or address")?;
            people::claim(store, alias, person_id(store, args.get(1))?)?;
        }
        Some("hide") => people::hide(store, person_id(store, args.first())?, true)?,
        Some("show") => people::hide(store, person_id(store, args.first())?, false)?,
        Some("note") => people::set_note(
            store,
            person_id(store, args.first())?,
            &args.get(1..).unwrap_or_default().join(" "),
        )?,
        Some(other) => anyhow::bail!(
            "unknown people action {other:?}: list, sync, page, memory, merge, split, claim, hide, show, note"
        ),
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // Read-only, so it opens before anything could migrate the memory.
    match &cli.command {
        Cmd::Brief {
            folder,
            index,
            as_of,
            session,
        } => {
            return brief_command(
                cli.db.as_deref(),
                folder,
                *index,
                as_of.as_deref(),
                session.as_deref(),
            );
        }
        Cmd::Thread {
            key,
            folder,
            query,
            as_of,
            session,
        } => {
            let ask = key
                .as_deref()
                .or(query.as_deref())
                .context("which thread: a KEY from the index, or --query WORDS")?;
            return thread_command(
                cli.db.as_deref(),
                folder,
                ask,
                as_of.as_deref(),
                session.as_deref(),
            );
        }
        Cmd::Connector {
            action,
            args,
            day,
            item,
            days,
        } => {
            return connector_command(
                cli.db.as_deref(),
                action.as_deref(),
                args,
                *day,
                item.as_deref(),
                *days,
            );
        }
        _ => {}
    }
    let store = match &cli.db {
        Some(path) => Store::open(path)?,
        None => Store::open_default()?,
    };
    let yesterday = || {
        time::working_day(Local::now())
            .pred_opt()
            .expect("a day before today")
    };

    match cli.command {
        Cmd::Brief { .. } | Cmd::Thread { .. } | Cmd::Connector { .. } => {}
        Cmd::Todos { action, args } => todos_command(&store, action.as_deref(), &args)?,
        Cmd::People { action, args } => people_command(&store, action.as_deref(), &args)?,
        Cmd::Jev { action, name } => match action.as_str() {
            "report" => {
                let name = name
                    .context("which report: chats, screen, promises, suggest, projects, cards")?;
                for line in memory::jev::report(&store, &name)? {
                    println!("{line}");
                }
            }
            "backtest" => {
                let name = name.context("which backtest: chats, projects")?;
                for line in memory::jev::backtest(&store, &name)? {
                    println!("{line}");
                }
            }
            "usage" => {
                for line in memory::jev::usage_report() {
                    println!("{line}");
                }
            }
            other => anyhow::bail!("unknown jev action {other:?}: report, backtest, usage"),
        },
        Cmd::Night { action, args } => night_command(&store, action.as_deref(), &args)?,
        Cmd::Moments {
            title,
            url,
            app,
            ask,
            claude_code,
            dry,
            days,
            limit,
        } => moments_command(
            &store,
            title.as_deref(),
            url.as_deref(),
            app.as_deref(),
            ask.as_deref(),
            claude_code,
            dry,
            days,
            limit,
        )?,
        Cmd::Ingest => {
            let started = std::time::Instant::now();
            for report in ingest::all(&store) {
                println!("{report}");
                for error in report.errors.iter().skip(1).take(5) {
                    println!("  also: {error}");
                }
            }
            println!("took {:.1} s", started.elapsed().as_secs_f64());
        }
        Cmd::Stats => stats(&store)?,
        Cmd::Digest { day } => {
            let day = day.unwrap_or_else(yesterday);
            match recap::digest_text(&store, day)? {
                Some(text) => println!("{text}"),
                None => println!("Nothing recorded for {}.", time::day_label(day)),
            }
        }
        Cmd::Recap { day, force } => {
            let day = day.unwrap_or_else(yesterday);
            if !force && let Some(saved) = recap::saved(&store, day)? {
                println!("{}", saved.to_markdown());
                println!("(saved earlier; --force writes it again)");
                return Ok(());
            }
            let started = std::time::Instant::now();
            match recap::generate(&store, day)? {
                Some(recap) => println!(
                    "{}\n(written in {:.1} s)",
                    recap.to_markdown(),
                    started.elapsed().as_secs_f64()
                ),
                None => println!("Nothing recorded for {}.", time::day_label(day)),
            }
        }
        Cmd::Recall { question, window } => {
            let text = recall::context(&store, &question, window.as_deref(), Local::now())?;
            println!("{text}\n\n({} characters)", text.chars().count());
        }
        Cmd::Cards => {
            for card in cards::for_overlay(&store, time::now_ms())? {
                let state = if card.seen_at.is_some() {
                    "seen"
                } else {
                    "new"
                };
                let actions: Vec<&str> = card.actions.iter().map(|a| a.id.as_str()).collect();
                println!(
                    "#{} {} [{state}] {} {actions:?}\n{}",
                    card.id, card.kind, card.title, card.body
                );
                if let Some(data) = &card.data {
                    println!("{data}");
                }
                println!();
            }
        }
        Cmd::Screen { text, app, window } => {
            let text = text.replace("\\n", "\n");
            let Some(lines) = screen::candidates(None, &text) else {
                println!("Nothing in it looks like an event.");
                return Ok(());
            };
            println!("Sent to Claude:\n{lines}\n");
            let snippet = screen::Snippet {
                app,
                window,
                text: lines,
            };
            let added = screen::nudge(&store, &[snippet], Local::now())?;
            println!("New nudges: {added:?}");
        }
        Cmd::Promises => println!("{}", promises::find(&store, Local::now())?),
        Cmd::Asks => println!("{}", memory::asks::check(&store, Local::now())?),
        Cmd::Progress { preview: true } => match memory::progress::preview(&store, Local::now())? {
            Some(prompt) => println!("{prompt}"),
            None => println!("Nothing new to read."),
        },
        Cmd::Progress { preview: false } => {
            println!("{}", memory::progress::check(&store, Local::now())?);
        }
        Cmd::Drafts { action, plan } => {
            let now = Local::now();
            let plan = || plan.context("name the plan: drafts list shows them");
            match action.as_str() {
                "list" => {
                    let found = drafts::candidates(&store, now)?;
                    if found.is_empty() {
                        println!(
                            "No promise or task of yours is due from 3 days back to 2 weeks ahead."
                        );
                    }
                    for candidate in found {
                        println!("{candidate}");
                    }
                }
                "preview" => println!("{}", drafts::preview(&store, plan()?, now)?),
                "write" => {
                    let started = std::time::Instant::now();
                    let prepared = drafts::write(&store, plan()?, now)?;
                    println!(
                        "{}\n\n(written in {:.1} s)",
                        serde_json::to_string_pretty(&prepared)?,
                        started.elapsed().as_secs_f64()
                    );
                }
                "run" => match drafts::run_due(&store, now) {
                    Some(line) => println!("{line}"),
                    None => println!("Nothing to draft now."),
                },
                other => anyhow::bail!("drafts has no action {other:?}: list, preview, write, run"),
            }
        }
        Cmd::Connectors { which } => {
            let now = Local::now();
            if which == "fireflies" || which == "both" {
                println!(
                    "{}",
                    connectors::fireflies(&store, now)
                        .unwrap_or_else(|e| format!("fireflies: {e:#}"))
                );
            }
            if which == "notion" || which == "both" {
                println!(
                    "{}",
                    connectors::notion(&store, now).unwrap_or_else(|e| format!("notion: {e:#}"))
                );
            }
        }
        Cmd::Act { card, action } => {
            let card = plans::act(&store, card, &action, None)?;
            println!("#{} {} {}\n{}", card.id, card.kind, card.title, card.body);
        }
        Cmd::UpNext { action, plan } => {
            let item = match (action, plan) {
                (Some(action), Some(plan)) => {
                    plans::set_up_next(&store, plan, &action, Local::now())?
                }
                _ => plans::up_next(&store, Local::now())?,
            };
            match item {
                Some(item) => {
                    let buttons: Vec<&str> =
                        item.actions.iter().map(|a| a.label.as_str()).collect();
                    println!(
                        "#{} {}, {}{} ({} more): {}\n  from {}\n  [{}]",
                        item.id,
                        item.kind,
                        item.when,
                        if item.overdue { ", overdue" } else { "" },
                        item.more,
                        item.title,
                        item.source,
                        buttons.join("] [")
                    );
                }
                None => println!("Nothing up next."),
            }
        }
        Cmd::Quick { app, window, url } => {
            let screen = memory::quick::Screen {
                app: Some(&app),
                title: window.as_deref(),
                url: url.as_deref(),
                read: true,
            };
            for ask in memory::quick::for_screen(&store, &screen, Local::now())? {
                let plans = if ask.plan { " (plans the day)" } else { "" };
                println!("[{}]{plans} {}", ask.label, ask.question);
            }
        }
        Cmd::Agendas { preview, apply } => {
            let mode = match (preview, apply) {
                (true, _) => plans::Rewrite::Preview,
                (_, true) => plans::Rewrite::Apply,
                _ => plans::Rewrite::List,
            };
            let lines = plans::rewrite_notes(&store, Local::now(), mode)?;
            if lines.is_empty() {
                println!("Every event still to come already has its notes.");
            }
            for line in lines {
                println!("{line}");
            }
        }
        Cmd::Plan { note, facts } => {
            if facts {
                println!(
                    "{}",
                    memory::day::facts_now(&store, note.is_some(), Local::now())?
                );
            } else {
                let plan = memory::day::plan(&store, note.as_deref(), Local::now())?;
                println!("{}: {}\n{}\n", plan.day, plan.note, plan.balance);
                for card in plan.cards {
                    let category = card
                        .data
                        .as_ref()
                        .and_then(|data| data["category"].as_str())
                        .unwrap_or_default()
                        .to_owned();
                    let title = card
                        .data
                        .as_ref()
                        .and_then(|data| data["title"].as_str())
                        .unwrap_or_default()
                        .to_owned();
                    println!(
                        "#{} {} [{category}] {title}\n  {}",
                        card.id, card.title, card.body
                    );
                }
            }
        }
        Cmd::Patterns { brief } => patterns(&store, brief)?,
        Cmd::Suggest {
            max,
            test,
            clean_test,
        } => {
            let lines = if test {
                routines::suggest::add_test(&store, time::now_ms())?
            } else if clean_test {
                let places = jobs::Places::standard().context("HOME is not set")?;
                jobs::clean_test(&store, &places)?
            } else {
                routines::suggest_now(&store, Local::now(), max)?
            };
            for line in lines {
                println!("{line}");
            }
        }
        Cmd::Jobs => {
            for (job, last) in jobs::list(&store)? {
                let when = job
                    .schedule
                    .as_ref()
                    .map_or_else(|| "a skill".to_owned(), routines::Schedule::describe);
                println!("{} [{}] {}: {when}", job.name, job.state, job.title);
                if let Some(last) = last {
                    println!(
                        "  last run {}: {}{}",
                        time::moment(last.started_at),
                        last.outcome.as_deref().unwrap_or("running"),
                        last.cost_usd
                            .map(|cost| format!(", ${cost:.3}"))
                            .unwrap_or_default()
                    );
                }
            }
        }
        Cmd::Job { action, name } => {
            let places = jobs::Places::standard().context("HOME is not set")?;
            if action == "run" {
                println!("{}", jobs::run(&store, &places, &name, Local::now())?);
            } else {
                let job = jobs::by_name(&store, &name)?
                    .with_context(|| format!("there is no job named {name}"))?;
                let action = if action == "start" { "run" } else { &action };
                jobs::job_action(&store, &places, &job, action, Local::now())?;
                let job = jobs::by_name(&store, &name)?.context("the job is gone")?;
                println!("{} is {}", job.name, job.state);
            }
        }
    }
    Ok(())
}

/// What the detectors find, with what became of each pattern before.
fn patterns(store: &Store, brief: bool) -> Result<()> {
    let found = routines::detect::all(store, Local::now())?;
    for pattern in &found {
        let state: Option<String> = store
            .conn()
            .query_row(
                "SELECT state FROM pattern WHERE key = ?1",
                [&pattern.key],
                |row| row.get(0),
            )
            .ok();
        println!(
            "{} [{}] {} (score {:.0})",
            pattern.kind.as_str(),
            state.as_deref().unwrap_or("new"),
            pattern.label,
            pattern.score
        );
        println!(
            "  {} times, {} sessions, {}",
            pattern.count,
            pattern.sessions,
            pattern.span()
        );
        if let Some(window) = &pattern.window {
            println!("  {}", window.describe());
        }
        if matches!(
            pattern.kind,
            routines::detect::Kind::Time | routines::detect::Kind::Loop
        ) {
            if pattern.readers.is_empty() {
                println!("  nothing Freebyrd can read it with");
            } else {
                println!("  readable with {}", pattern.readers.join(", "));
            }
        }
        if brief {
            continue;
        }
        for fact in &pattern.facts {
            println!("  - {fact}");
        }
        for example in &pattern.examples {
            println!("  * {}: {}", time::moment(example.at), example.text);
        }
    }
    println!("{} patterns", found.len());
    Ok(())
}

fn stats(store: &Store) -> Result<()> {
    let conn = store.conn();
    let mut statement = conn.prepare(
        "SELECT source, kind, count(*), min(ts), max(ts) FROM activity
         GROUP BY source, kind ORDER BY source, count(*) DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    for row in rows {
        let (source, kind, count, first, last) = row?;
        println!(
            "{source:<8} {kind:<7} {count:>8}  {} to {}",
            time::moment(first),
            time::moment(last)
        );
    }
    let count = |sql: &str| conn.query_row(sql, [], |row| row.get::<_, i64>(0));
    println!(
        "projects {}, sessions {}, page texts {}, recaps {}, cards {}",
        count("SELECT count(*) FROM project")?,
        count("SELECT count(*) FROM session")?,
        count("SELECT count(*) FROM page_text")?,
        count("SELECT count(*) FROM recap")?,
        count("SELECT count(*) FROM card")?,
    );
    if let Some(path) = Store::default_path() {
        let size = std::fs::metadata(&path)
            .map(|m| m.len())
            .unwrap_or_default();
        println!("{}: {:.1} MB", path.display(), size as f64 / 1e6);
    }
    Ok(())
}

fn night_command(store: &Store, action: Option<&str>, args: &[String]) -> Result<()> {
    use memory::night;
    let now = Local::now();
    let id = |args: &[String]| -> Result<i64> {
        Ok(args
            .first()
            .context("give the job's number")?
            .trim_start_matches('#')
            .parse()?)
    };
    match action.unwrap_or("list") {
        "list" => print_night(&night::view(store, now.timestamp_millis())?),
        "add" => {
            let job = night::add(store, &args.join(" "), now)?;
            println!(
                "queued #{} {} ({}) in {}: {}",
                job.id,
                job.title,
                job.kind.as_str(),
                job.project.unwrap_or_default(),
                job.sub
            );
        }
        "run" => {
            match night::run::run_all(store, args.first().map(|id| id.parse()).transpose()?)? {
                Some(summary) => println!(
                    "ran {}, {} need you, {} left in the queue",
                    summary.ran, summary.needs, summary.left
                ),
                None => println!("another night shift is running"),
            }
        }
        "start" => {
            night::run::spawn(None)?;
            println!(
                "started; it writes to {}",
                night::run::log_path()?.display()
            );
        }
        "act" => {
            let job = id(args)?;
            let action = args.get(1).context("act ID ACTION [VALUE]")?;
            let value = args
                .get(2..)
                .map(|rest| rest.join(" "))
                .filter(|value| !value.is_empty());
            print_night(&night::act(store, job, action, value.as_deref(), now)?);
        }
        "show" => {
            let job = night::get(store, id(args)?)?.context("no such job")?;
            println!(
                "#{} {} ({}, {})",
                job.id,
                job.title,
                job.kind.as_str(),
                job.state
            );
            println!("folder: {}", job.dir.unwrap_or_default());
            println!(
                "{}",
                serde_json::to_string_pretty(&job.result.unwrap_or_default())?
            );
        }
        "card" => night::run::morning_card(store, now.timestamp_millis())?,
        "stop" => println!("{} offered on the stop card", night::stop::offer(store, now)?),
        // What the notch's panel gets, as JSON: the view, or a job's page.
        "json" => println!(
            "{}",
            serde_json::to_string(&night::view(store, now.timestamp_millis())?)?
        ),
        "page" => println!("{}", night::detail(store, id(args)?)?),
        other => anyhow::bail!("unknown night action {other}"),
    }
    Ok(())
}

fn print_night(view: &memory::night::View) {
    println!(
        "Tonight ({}):",
        if view.running {
            "running"
        } else {
            "not running"
        }
    );
    for row in &view.tonight {
        println!(
            "  #{} {} [{}] {} · {} · {}",
            row.id, row.title, row.kind, row.state, row.sub, row.end
        );
    }
    println!("While you were away ({} need you):", view.needs);
    for row in &view.waiting {
        let mark = if row.needs { "!" } else { " " };
        println!(
            "{mark} #{} {} [{}] {} · {}",
            row.id, row.title, row.kind, row.sub, row.end
        );
    }
    println!("{}", view.starts);
}
