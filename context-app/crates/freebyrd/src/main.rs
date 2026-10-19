//! Freebyrd: double-tap Option in any app to ask Claude about what is on
//! screen and about your own work, which Freebyrd remembers.
//!
//! The app has no Dock icon. It waits for a double tap (or the menu-bar
//! item), notes which app was in front, shows the overlay at once with any
//! cards waiting (a recap, say) and what is up next, reads that app in the
//! background, offers quick asks for it, and streams Claude's answer into
//! the overlay. In the background, `worker`
//! keeps the memory current; while a card is waiting, the closed island
//! glows at the notch.

mod claude;
mod claude_app;
mod companion;
mod context;
mod frame_rate;
mod hover;
mod live;
mod models;
mod notch;
mod picture;
mod sessions;
mod tap;
mod watcher;
mod whatsapp;
mod worker;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, State, WebviewWindow, WindowEvent};

use claude::{AnswerEvent, Turn};
use context::ScreenContext;
use memory::cards::{self, Card};

const OVERLAY: &str = "overlay";

/// The overlay hides when it loses focus, and a double tap can itself take
/// focus away before it is recognized. A double tap this soon after such a
/// hide is the person closing the overlay, not opening it again.
const CLOSE_GESTURE_WINDOW: Duration = Duration::from_millis(700);

/// How long the page takes to fold the island back into the notch before the
/// window hides: the `FOLD` spring in `ui/app.js` is 99% there by 210 ms.
const FOLD: Duration = Duration::from_millis(220);

/// How often the closed island's indicators are looked at again: while a
/// job runs, its progress; otherwise, to-dos falling due and jobs queued.
const LIVE_MOVING: Duration = Duration::from_secs(3);
const LIVE_STILL: Duration = Duration::from_secs(30);
/// Between those looks, how often the context connector's records are
/// checked for a change: a tool's ask waits on the person's answer.
const CONNECTOR_POLL: Duration = Duration::from_secs(2);

/// Each opening of the overlay is a session. Captures and answers that finish
/// after a newer session started are dropped.
#[derive(Default)]
struct Session {
    current: AtomicU64,
    /// Open, as opposed to hidden or glowing: the glowing island is a
    /// visible window too, but closed.
    open: AtomicBool,
    screen: Mutex<Option<ScreenContext>>,
    /// The questions asked in this opening and their answers: a follow-up
    /// is sent with them.
    thread: Mutex<Thread>,
    hidden_on_blur_at: Mutex<Option<Instant>>,
    /// What the closed island last showed, so a refresh that changes
    /// nothing leaves it alone. None while open or hidden.
    closed: Mutex<Option<Glow>>,
    /// Where the closed island is, so it can be clicked (`hover`).
    island: Mutex<Island>,
    /// Whether the closed window takes clicks: the pointer is over the
    /// island. Otherwise clicks go through it to the menu bar.
    clickable: AtomicBool,
    /// The app in front when the pointer reached the closed island. A click
    /// there opens the notch about it, though Freebyrd is in front by then.
    hovered_from: Mutex<Option<context::Target>>,
}

/// The closed island's place: the window's top left on screen (set when it
/// is placed) and the island's edges inside it (from the page), in points.
#[derive(Default)]
struct Island {
    window: Option<(f64, f64)>,
    edges: Option<(f64, f64, f64)>,
}

#[derive(Default)]
struct Thread {
    session: u64,
    turns: Vec<Turn>,
    /// What was pressed to ask about (`plan:12` on the Up next line or a
    /// draft card, `listed:12` or `todo:3` in the to-do list, `person:4` on
    /// a page), which goes with this opening's questions, follow-ups
    /// included.
    focus: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Capturing {
    session: u64,
    app_name: Option<String>,
}

#[derive(Clone, Serialize)]
struct ContextReady {
    session: u64,
    screen: ScreenContext,
}

#[derive(Clone, Serialize)]
struct CardsReady {
    session: u64,
    cards: Vec<Card>,
}

/// The one plan that most needs the person, for the Up next line.
#[derive(Clone, Serialize)]
struct UpNextReady {
    session: u64,
    item: Option<memory::plans::UpNext>,
}

/// The to-do list behind the checklist icon, with its number.
#[derive(Clone, Serialize)]
struct TodosReady {
    session: u64,
    list: Option<memory::todos::List>,
}

/// Questions to ask with one press, picked for the screen just read.
#[derive(Clone, Serialize)]
struct QuickReady {
    session: u64,
    asks: Vec<memory::quick::QuickAsk>,
}

/// The memory sent with a question, shown under "What was read".
#[derive(Clone, Serialize)]
struct Recalled {
    session: u64,
    text: String,
}

#[derive(Clone, Serialize)]
struct Answer {
    session: u64,
    event: AnswerEvent,
}

fn main() {
    // `freebyrd --ask QUESTION [--picture FILE.jpg | --focus REF] < screen.txt`
    // runs one question through the same Claude path as the overlay, memory
    // included, and prints the events, for testing. `--focus` sends what a
    // button would (`person:4` for Brief me, `listed:12` or `todo:3` for a
    // to-do, `plan:12` for Up next).
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag, question] if flag == "--ask" => {
            std::process::exit(ask_from_terminal(question, None, None));
        }
        [flag, question, picture, path] if flag == "--ask" && picture == "--picture" => {
            std::process::exit(ask_from_terminal(question, Some(path), None));
        }
        [flag, question, focus, reference] if flag == "--ask" && focus == "--focus" => {
            std::process::exit(ask_from_terminal(question, None, Some(reference)));
        }
        _ => {}
    }
    // `freebyrd --read-window PID [OUT]` reads an app the way a double tap
    // does and prints counts and timings; with OUT, it also writes the text
    // in view to OUT.txt and the picture to OUT.jpg, for checking them.
    if let [flag, pid, out @ ..] = args.as_slice()
        && flag == "--read-window"
        && out.len() <= 1
    {
        std::process::exit(read_window(pid, out.first()));
    }
    // `freebyrd --read-whatsapp` reads WhatsApp's window and prints counts
    // only, no names or text, to check the reader against a real window.
    if let [flag] = args.as_slice()
        && flag == "--read-whatsapp"
    {
        std::process::exit(whatsapp::print_counts());
    }
    // `freebyrd --read-claude` reads the Claude app's window as moments do
    // and prints counts only, no text, to check the reader on a real chat.
    if let [flag] = args.as_slice()
        && flag == "--read-claude"
    {
        std::process::exit(claude_app::print_counts());
    }
    // `freebyrd --live` prints what the closed notch would show beside it
    // now (counts and progress only, no titles).
    if let [flag] = args.as_slice()
        && flag == "--live"
    {
        println!(
            "{}",
            serde_json::to_string_pretty(&live::now()).unwrap_or_default()
        );
        return;
    }
    // `freebyrd --sessions` prints the session board as the notch's
    // Sessions tab shows it (titles, folders, states, what Push would push);
    // `freebyrd --sessions ID` prints what Push would hand to Claude Code for
    // that session, opening nothing.
    if let [flag, rest @ ..] = args.as_slice()
        && flag == "--sessions"
    {
        match rest {
            [] => println!(
                "{}",
                serde_json::to_string_pretty(&sessions::board(chrono::Local::now()))
                    .unwrap_or_default()
            ),
            [id] => match sessions::handoff(id) {
                Ok(handoff) => println!("In {}:\n\n{}", handoff.folder, handoff.prompt),
                Err(e) => {
                    eprintln!("freebyrd: {e:#}");
                    std::process::exit(1);
                }
            },
            _ => {
                eprintln!("usage: freebyrd --sessions [ID]");
                std::process::exit(2);
            }
        }
        return;
    }
    // `freebyrd --companion` runs the iPad app's server alone, without the
    // notch, to try it in a browser (http://127.0.0.1:4747/pair to pair).
    if let [flag] = args.as_slice()
        && flag == "--companion"
    {
        std::process::exit(companion::run_foreground());
    }

    tauri::Builder::default()
        .manage(Session::default())
        .invoke_handler(tauri::generate_handler![
            ask,
            dismiss,
            dismiss_card,
            fit,
            page_ready,
            card_action,
            add_event,
            up_next_action,
            plan_day,
            copy_text,
            todos,
            todo_action,
            add_todo,
            person_page,
            person_action,
            night,
            night_job,
            night_add,
            night_action,
            island_shape,
            open_island,
            sessions_board,
            session_open,
            session_push,
            connector_reads,
            connector_verdict
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(focused) = event {
                // On losing focus, name the app that took it: another app's
                // shortcut can grab focus mid-gesture.
                let taker = (!focused)
                    .then(context::frontmost_app)
                    .flatten()
                    .and_then(|app| app.name)
                    .map(|name| format!(", now in front: {name}"))
                    .unwrap_or_default();
                eprintln!("freebyrd: window focused={focused}{taker}");
                let state = window.state::<Session>();
                if !focused && state.open.load(Ordering::SeqCst) {
                    *state
                        .hidden_on_blur_at
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
                    if let Some(overlay) = window.app_handle().get_webview_window(OVERLAY) {
                        close(&overlay);
                    }
                }
            }
        })
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            build_tray(app)?;
            if let Some(overlay) = app.get_webview_window(OVERLAY) {
                let hide_from_capture = std::env::var_os("FREEBYRD_SHOW_IN_SCREEN_SHARE").is_none();
                notch::raise(&overlay, hide_from_capture)?;
                overlay.with_webview(|webview| {
                    if !frame_rate::unlock(webview.inner()) {
                        eprintln!("freebyrd: WebKit's 60 fps setting was not found; animations stay at 60 fps");
                    }
                })?;
            }

            let handle = app.handle().clone();
            match tap::watch(move |gesture| match gesture {
                tap::Gesture::DoubleTap => toggle(&handle),
                tap::Gesture::Tasks => open_tasks(&handle),
            }) {
                Ok(()) => eprintln!("freebyrd: ready; double-tap Option to open, ⌘⌥ for the to-do list"),
                Err(e) => eprintln!("freebyrd: {e} The menu-bar item still opens the overlay."),
            }
            if !capture::ax::is_trusted(false) {
                eprintln!("freebyrd: Accessibility access is not granted, so apps cannot be read");
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("the overlay failed to start")
        .run(|app, event| {
            if let tauri::RunEvent::Ready = event {
                // macOS activates an app as it launches. This one has no
                // window to show, so hand the keyboard back to the app in front.
                #[cfg(target_os = "macos")]
                let _ = app.hide();
                // Started only now, so hiding cannot hide its first glow.
                let handle = app.clone();
                worker::start(move |_| glow_if_closed(&handle));
                watch_live(app.clone());
                watch_pointer(app);
                // The iPad app's server (`companion`), on this Mac only
                // until the person turns on the Wi-Fi or Tailscale.
                companion::start();
                let (open, glow) = (app.clone(), app.clone());
                watcher::start(
                    move || open.state::<Session>().open.load(Ordering::SeqCst),
                    move || glow_if_closed(&glow),
                );
            }
        });
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let ask = MenuItem::with_id(
        app,
        "ask",
        "Ask about this screen (double-tap ⌥)",
        true,
        None::<&str>,
    )?;
    let pair = MenuItem::with_id(app, "pair", "Pair an iPad\u{2026}", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Freebyrd", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&ask, &pair, &quit])?;
    // The falcon as a template image, so macOS tints it for light and dark
    // menu bars (`icons/make_icons.py` draws it).
    let tray = TrayIconBuilder::with_id("main")
        .icon(tauri::include_image!("icons/tray.png"))
        .icon_as_template(true)
        .tooltip("Freebyrd")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "ask" => toggle(app),
            // The pairing page, in the person's browser on this Mac.
            "pair" => {
                let url = format!("http://127.0.0.1:{}/pair", companion::PORT);
                if let Err(e) = std::process::Command::new("/usr/bin/open").arg(url).spawn() {
                    eprintln!("freebyrd: could not open the pairing page: {e}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        });
    tray.build(app)?;
    Ok(())
}

/// Opens the overlay over the current app, or closes it if it is open.
fn toggle(app: &AppHandle) {
    let Some(window) = app.get_webview_window(OVERLAY) else {
        return;
    };
    let state = app.state::<Session>();
    let open = state.open.load(Ordering::SeqCst);
    let just_hidden = state
        .hidden_on_blur_at
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .is_some_and(|at| at.elapsed() < CLOSE_GESTURE_WINDOW);
    eprintln!("freebyrd: toggle (open={open}, just_hidden={just_hidden})");
    if open {
        close(&window);
        return;
    }
    if just_hidden {
        return;
    }
    // Note the app in front before the overlay takes focus.
    open_over(app, context::frontmost_app());
}

/// Opens the overlay about `target`, the app that was in front.
fn open_over(app: &AppHandle, target: Option<context::Target>) {
    let Some(window) = app.get_webview_window(OVERLAY) else {
        return;
    };
    let state = app.state::<Session>();
    let session = state.current.fetch_add(1, Ordering::SeqCst) + 1;
    state.open.store(true, Ordering::SeqCst);
    *state.screen.lock().unwrap_or_else(PoisonError::into_inner) = None;
    *state.thread.lock().unwrap_or_else(PoisonError::into_inner) = Thread {
        session,
        ..Thread::default()
    };
    let _ = app.emit(
        "capturing",
        Capturing {
            session,
            app_name: target.as_ref().and_then(|t| t.name.clone()),
        },
    );
    let _ = app.emit(
        "cards",
        CardsReady {
            session,
            cards: cards_to_show(),
        },
    );
    let _ = app.emit(
        "upnext",
        UpNextReady {
            session,
            item: up_next_now(),
        },
    );
    let _ = app.emit(
        "todos",
        TodosReady {
            session,
            list: todos_now(),
        },
    );
    show(&window);

    let app = app.clone();
    thread::spawn(move || {
        let screen = target.map_or_else(ScreenContext::nothing_in_front, |t| context::capture(&t));
        eprintln!(
            "freebyrd: session {session}: {} via {:?}, {} chars in {} ms{}",
            screen.app_name.as_deref().unwrap_or("?"),
            screen.source,
            screen.text.chars().count(),
            screen.elapsed_ms,
            screen
                .withheld
                .as_deref()
                .map(|r| format!(", withheld: {r}"))
                .unwrap_or_default(),
        );
        let state = app.state::<Session>();
        if state.current.load(Ordering::SeqCst) != session {
            return;
        }
        *state.screen.lock().unwrap_or_else(PoisonError::into_inner) = Some(screen.clone());
        let asks = quick_asks(&screen);
        let _ = app.emit("context", ContextReady { session, screen });
        let _ = app.emit("quick", QuickReady { session, asks });
    });
}

/// Command and Option on their own: the to-do list. Opens the overlay on
/// it, or, already open, turns to it (or closes, if it shows already; the
/// page decides, `tasks` in app.js).
fn open_tasks(app: &AppHandle) {
    let open = app.state::<Session>().open.load(Ordering::SeqCst);
    if !open {
        toggle(app);
        if !app.state::<Session>().open.load(Ordering::SeqCst) {
            return;
        }
    }
    let _ = app.emit("tasks", TasksShortcut { opened: !open });
}

#[derive(Clone, Serialize)]
struct TasksShortcut {
    /// Whether the overlay opened for it, rather than being open already.
    opened: bool,
}

/// What is up next, or nothing if the memory cannot be read.
fn up_next_now() -> Option<memory::plans::UpNext> {
    memory::Store::open_default()
        .and_then(|store| memory::plans::up_next(&store, chrono::Local::now()))
        .unwrap_or_else(|e| {
            eprintln!("freebyrd: memory: could not find what is up next: {e:#}");
            None
        })
}

/// The quick asks for the screen just read, worked out on the Mac.
fn quick_asks(screen: &ScreenContext) -> Vec<memory::quick::QuickAsk> {
    let read = memory::quick::Screen {
        app: screen.app_name.as_deref(),
        title: screen.window_title.as_deref(),
        url: screen.url.as_deref(),
        read: screen.withheld.is_none(),
    };
    memory::Store::open_default()
        .and_then(|store| memory::quick::for_screen(&store, &read, chrono::Local::now()))
        .unwrap_or_else(|e| {
            eprintln!("freebyrd: memory: could not pick quick asks: {e:#}");
            Vec::new()
        })
}

/// The cards the overlay opens with. Showing them counts as seeing them, so
/// the glow stops.
fn cards_to_show() -> Vec<Card> {
    let now = memory::time::now_ms();
    let shown = memory::Store::open_default().and_then(|store| {
        let shown = cards::for_overlay(&store, now)?;
        let ids: Vec<i64> = shown.iter().map(|card| card.id).collect();
        cards::mark_seen(&store, &ids, now)?;
        Ok(shown)
    });
    shown.unwrap_or_else(|e| {
        eprintln!("freebyrd: memory: could not load cards: {e:#}");
        Vec::new()
    })
}

/// Hangs the overlay from the notch of the screen with the pointer and shows
/// it. The page then opens the island out of the notch (`open`).
fn show(window: &WebviewWindow) {
    let overlay = window.clone();
    let _ = window.run_on_main_thread(move || {
        let notch = notch::place(&overlay).unwrap_or_default();
        *overlay
            .state::<Session>()
            .closed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        let _ = overlay.set_ignore_cursor_events(false);
        let _ = overlay.show();
        let _ = overlay.set_focus();
        let _ = overlay.emit("open", notch);
    });
}

/// What the closed island shows: the notch, the kinds of the waiting
/// cards, which pick the icons beside the notch and make it glow, and what
/// is live (`live`), shown beside it without a glow.
#[derive(Clone, PartialEq, Serialize)]
struct Glow {
    notch: notch::Notch,
    kinds: Vec<String>,
    live: live::Live,
}

/// Shows the closed island at the notch, widened for the icons of the
/// waiting cards' kinds (glowing) and what is live, without taking focus.
/// It takes clicks only while the pointer is over it (`watch_pointer`).
/// Opening the overlay shows the cards.
fn glow(window: &WebviewWindow, kinds: Vec<String>, live: live::Live) {
    let overlay = window.clone();
    let _ = window.run_on_main_thread(move || {
        let state = overlay.state::<Session>();
        if state.open.load(Ordering::SeqCst) {
            return;
        }
        let notch = notch::place(&overlay).unwrap_or_default();
        state
            .island
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .window = notch::top_left(&overlay);
        let shown = Glow { notch, kinds, live };
        let mut last = state.closed.lock().unwrap_or_else(PoisonError::into_inner);
        if last.as_ref() == Some(&shown) && overlay.is_visible().unwrap_or(false) {
            return;
        }
        *last = Some(shown.clone());
        drop(last);
        // Clicks go through, except over the island itself (`hover`).
        let _ = overlay.set_ignore_cursor_events(!state.clickable.load(Ordering::SeqCst));
        let _ = overlay.emit("glow", shown);
        notch::show_inactive(&overlay);
    });
}

/// The page is listening. A glow sent before then (the worker's first pass
/// can finish before the page loads) was lost, so it is sent again.
#[tauri::command]
fn page_ready(app: AppHandle) {
    *app.state::<Session>()
        .closed
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = None;
    glow_if_closed(&app);
}

/// Shows the closed island for the cards waiting now and what is live, or
/// hides it if there is neither. The open overlay is left alone.
fn glow_if_closed(app: &AppHandle) {
    show_closed(app, live::now());
}

fn show_closed(app: &AppHandle, live: live::Live) {
    if app.state::<Session>().open.load(Ordering::SeqCst) {
        return;
    }
    let kinds = waiting_kinds();
    if kinds.is_empty() && live.is_empty() {
        stop_glow(app);
    } else if let Some(window) = app.get_webview_window(OVERLAY) {
        glow(&window, kinds, live);
    }
}

/// Puts out a glow whose card is gone (dismissed, or replaced) when nothing
/// is live either. The open overlay is left alone.
fn stop_glow(app: &AppHandle) {
    let state = app.state::<Session>();
    if state.open.load(Ordering::SeqCst) {
        return;
    }
    *state.closed.lock().unwrap_or_else(PoisonError::into_inner) = None;
    state.clickable.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window(OVERLAY)
        && window.is_visible().unwrap_or(false)
    {
        let _ = window.emit("unglow", ());
        let _ = window.hide();
    }
}

/// Looks again at what is live, often while something moves (a job's
/// progress) and now and then otherwise (a to-do falling due, a job queued
/// from the command line), and shows it.
fn watch_live(app: AppHandle) {
    if std::env::var_os("FREEBYRD_NO_MEMORY").is_some() {
        return;
    }
    let spawned = thread::Builder::new()
        .name("live".to_owned())
        .spawn(move || {
            loop {
                let live = live::now();
                let moving = live.is_moving();
                show_closed(&app, live);
                // Sooner when a tool asks or reads (`connector.db` changed),
                // so its consent card or read shows at once.
                let wait = if moving { LIVE_MOVING } else { LIVE_STILL };
                let stamp = memory::connector::stamp();
                let started = Instant::now();
                while started.elapsed() < wait {
                    thread::sleep(CONNECTOR_POLL.min(wait));
                    if memory::connector::stamp() != stamp {
                        break;
                    }
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("freebyrd: the notch's indicators could not start: {e}");
    }
}

/// Lets the closed island take a click while the pointer is over it, and
/// only then (`hover`), noting the app in front as it arrives.
fn watch_pointer(app: &AppHandle) {
    let handle = app.clone();
    let shape = move || {
        let state = handle.state::<Session>();
        let shown = state
            .closed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if !shown || state.open.load(Ordering::SeqCst) {
            return None;
        }
        let island = state.island.lock().unwrap_or_else(PoisonError::into_inner);
        let ((x, y), (left, right, bottom)) = (island.window?, island.edges?);
        Some(hover::Shape {
            left: x + left,
            top: y,
            right: x + right,
            bottom: y + bottom,
        })
    };
    let handle = app.clone();
    let over = move |over: bool| {
        let state = handle.state::<Session>();
        if state.open.load(Ordering::SeqCst) || state.clickable.load(Ordering::SeqCst) == over {
            return;
        }
        if over {
            *state
                .hovered_from
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = context::frontmost_app();
        }
        state.clickable.store(over, Ordering::SeqCst);
        let Some(window) = handle.get_webview_window(OVERLAY) else {
            return;
        };
        let overlay = window.clone();
        // On the main thread, where opening sets it too: never leave the
        // open overlay ignoring clicks.
        let _ = window.run_on_main_thread(move || {
            if overlay.state::<Session>().open.load(Ordering::SeqCst) {
                return;
            }
            let _ = overlay.set_ignore_cursor_events(!over);
            let _ = overlay.emit("hover", over);
        });
    };
    hover::watch(shape, over);
}

/// Where the closed island's edges are in the window, in points, from the
/// page each time it shows, so it can be clicked there (`hover`).
#[tauri::command]
fn island_shape(state: State<Session>, left: f64, right: f64, bottom: f64) {
    state
        .island
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .edges = Some((left, right, bottom));
}

/// The closed island was clicked: opens the notch about the app that was in
/// front, on the to-do list or night shift when one of their indicators
/// was clicked (`panel`).
#[tauri::command]
fn open_island(app: AppHandle, panel: Option<String>) {
    let state = app.state::<Session>();
    if state.open.load(Ordering::SeqCst) {
        return;
    }
    let target = state
        .hovered_from
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    eprintln!(
        "freebyrd: the closed island was clicked ({})",
        panel.as_deref().unwrap_or("notch")
    );
    open_over(&app, target);
    if let Some(panel) =
        panel.filter(|panel| matches!(panel.as_str(), "todos" | "night" | "sessions"))
    {
        let _ = app.emit("island", IslandClicked { panel });
    }
}

#[derive(Clone, Serialize)]
struct IslandClicked {
    /// `todos`, `night` or `sessions`.
    panel: String,
}

fn waiting_kinds() -> Vec<String> {
    memory::Store::open_default()
        .and_then(|store| cards::waiting_kinds(&store))
        .unwrap_or_else(|e| {
            eprintln!("freebyrd: memory: could not check for cards: {e:#}");
            Vec::new()
        })
}

/// Sets the overlay's height in points, keeping its width. macOS keeps the
/// top edge where it is, at the top of the screen, so the overlay grows and
/// shrinks at the bottom.
fn resize(window: &WebviewWindow, height: f64) -> tauri::Result<()> {
    let width = window
        .outer_size()?
        .to_logical::<f64>(window.scale_factor()?)
        .width;
    window.set_size(LogicalSize::new(width, height))
}

/// Before a question about meetings, reads from Fireflies what the answer
/// may need: a call that ended since the last read, a day the question
/// names ("my Sep 7 sync") that is older than the meetings kept, or, when
/// it asks what someone said, the meeting's transcript
/// (`connectors::before_question`). `note` hears when a read starts and
/// when it fails; the question is then answered from what was saved
/// earlier.
fn refresh_meetings(question: &str, note: impl Fn(&str)) {
    let refreshed = memory::Store::open_default().and_then(|store| {
        memory::connectors::before_question(&store, question, chrono::Local::now(), || {
            note("Reading meetings from Fireflies");
        })
    });
    match refreshed {
        Ok(lines) => {
            for line in lines {
                eprintln!("freebyrd: {line}");
            }
        }
        Err(e) => {
            eprintln!("freebyrd: fireflies: {e:#}");
            note("Fireflies did not answer, so this is from the meetings saved earlier");
        }
    }
}

/// What the memory adds to a question. Empty if the memory cannot be read.
fn recall(question: &str, window_title: Option<&str>) -> String {
    memory::Store::open_default()
        .and_then(|store| {
            memory::recall::context(&store, question, window_title, chrono::Local::now())
        })
        .unwrap_or_else(|e| {
            eprintln!("freebyrd: memory: {e:#}");
            String::new()
        })
}

/// The to-do list now, or nothing if the memory cannot be read.
fn todos_now() -> Option<memory::todos::List> {
    memory::Store::open_default()
        .and_then(|store| memory::todos::list(&store, chrono::Local::now()))
        .inspect_err(|e| eprintln!("freebyrd: memory: could not read the to-do list: {e:#}"))
        .ok()
}

/// Starts streaming an answer about the current session's screen and the
/// memory, with the session's earlier questions and answers when this is a
/// follow-up. `focus` is what was pressed to ask about (a plan on the Up
/// next line, a to-do, a person's page; `Thread::focus`), which then goes
/// with this opening's questions. Answers arrive as `answer` events tagged
/// with the session.
#[tauri::command]
fn ask(
    app: AppHandle,
    state: State<'_, Session>,
    question: String,
    focus: Option<String>,
) -> Result<u64, String> {
    let screen = state
        .screen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or("Still reading the screen. Try again in a moment.")?;
    let session = state.current.load(Ordering::SeqCst);
    let (earlier, focus) = {
        let mut thread = state.thread.lock().unwrap_or_else(PoisonError::into_inner);
        if focus.is_some() {
            thread.focus = focus;
        }
        (thread.turns.clone(), thread.focus.clone())
    };

    thread::spawn(move || {
        // What was pressed goes to Claude only inside the memory, as data.
        // Its own words find the work related to it.
        let focus = focus.as_deref().and_then(focus_of);
        // "And tomorrow?" finds little on its own; the question it follows
        // up on says what it is about.
        let mut topic = match earlier.last() {
            Some(last) => format!("{question} {}", last.question),
            None => question.clone(),
        };
        if let Some(focus) = &focus {
            topic = format!("{} {topic}", focus.topic);
        }
        let send = |event| {
            let _ = app.emit("answer", Answer { session, event });
        };
        refresh_meetings(&topic, |message| {
            send(AnswerEvent::Note {
                message: message.to_owned(),
            });
        });
        let mut memory = recall(&topic, screen.window_title.as_deref());
        if let Some(focus) = focus {
            memory = [focus.memory, memory]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
        }
        let _ = app.emit(
            "recalled",
            Recalled {
                session,
                text: memory.clone(),
            },
        );
        let mut answer = String::new();
        let streamed = claude::stream_answer(&screen, &memory, &earlier, &question, |event| {
            if let AnswerEvent::Text { text } = &event {
                answer.push_str(text);
            }
            send(event);
        });
        if let Err(e) = streamed {
            send(AnswerEvent::Error {
                message: format!("{e:#}"),
            });
        }
        // Kept for the next follow-up, unless the overlay was opened afresh.
        let state = app.state::<Session>();
        let mut thread = state.thread.lock().unwrap_or_else(PoisonError::into_inner);
        if thread.session == session && !answer.trim().is_empty() {
            thread.turns.push(Turn { question, answer });
        }
    });
    Ok(session)
}

#[tauri::command]
fn dismiss(window: WebviewWindow) {
    close(&window);
}

/// Puts a draft on the clipboard, for its Copy button. WebKit lets a page
/// write the clipboard only in some cases; the pasteboard always takes it.
#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;

    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    // SAFETY: a constant AppKit defines.
    let kind = unsafe { NSPasteboardTypeString };
    if board.setString_forType(&NSString::from_str(&text), kind) {
        Ok(())
    } else {
        Err("The clipboard did not take the draft.".to_owned())
    }
}

/// A card's button: add a nudge's event (as edited), undo, answer a
/// question. Returns the card as it is now. Calendar work can take a moment,
/// so this runs off the main thread.
#[tauri::command(async)]
fn card_action(
    id: i64,
    action: String,
    template: Option<memory::plans::Template>,
) -> Result<Card, String> {
    memory::Store::open_default()
        .and_then(|store| memory::plans::act(&store, id, &action, template))
        .map_err(|e| format!("{e:#}"))
}

/// Add on an event an answer offered (a ```event block, `claude.rs`): the
/// person's press puts it on the calendar, as they edited it, through a
/// card like a nudge's, which comes back saying so with Undo. Off the main
/// thread, like a card's Add.
#[tauri::command(async)]
fn add_event(offer: memory::plans::Offer) -> Result<Card, String> {
    let added = memory::Store::open_default().and_then(|store| {
        memory::plans::add_offered(&store, &offer, chrono::Local::now().timestamp_millis())
    });
    match added {
        Ok(card) => {
            eprintln!(
                "freebyrd: added an event an answer offered (card {})",
                card.id
            );
            Ok(card)
        }
        Err(e) => {
            eprintln!("freebyrd: adding an offered event: {e:#}");
            Err(format!("{e:#}"))
        }
    }
}

/// "Plan my day": the rest of the working day as blocks, each a card to
/// change, add to the calendar or dismiss (`memory::day`). `note` is what
/// the person typed after a plan, to change it. Claude takes a while, so
/// this runs off the main thread.
#[tauri::command(async)]
fn plan_day(note: Option<String>) -> Result<memory::day::DayPlan, String> {
    let planned = memory::Store::open_default()
        .and_then(|store| memory::day::plan(&store, note.as_deref(), chrono::Local::now()));
    match planned {
        Ok(plan) => {
            eprintln!(
                "freebyrd: planned {} with {} blocks ({})",
                plan.day,
                plan.cards.len(),
                plan.balance
            );
            Ok(plan)
        }
        Err(e) => {
            eprintln!("freebyrd: plan my day: {e:#}");
            Err(format!("Could not plan the day: {e:#}"))
        }
    }
}

/// A button on the Up next line: `done` or `hide` (until tomorrow). Returns
/// what is up next now.
#[tauri::command]
fn up_next_action(id: i64, action: String) -> Result<Option<memory::plans::UpNext>, String> {
    memory::Store::open_default()
        .and_then(|store| memory::plans::set_up_next(&store, id, &action, chrono::Local::now()))
        .map_err(|e| format!("{e:#}"))
}

/// The to-do list, for the checklist icon's panel.
#[tauri::command(async)]
fn todos() -> Result<memory::todos::List, String> {
    memory::Store::open_default()
        .and_then(|store| memory::todos::list(&store, chrono::Local::now()))
        .map_err(|e| format!("{e:#}"))
}

/// A to-do's button (`memory::todos::act`): done, undo, hide, remove,
/// calendar (which can write an agenda with Claude, so this runs off the
/// main thread) or date (`value` is the date). Returns the list as it is now.
#[tauri::command(async)]
fn todo_action(
    id: String,
    action: String,
    value: Option<String>,
) -> Result<memory::todos::List, String> {
    memory::Store::open_default()
        .and_then(|store| {
            memory::todos::act(&store, &id, &action, value.as_deref(), chrono::Local::now())
        })
        .map_err(|e| format!("{e:#}"))
}

/// Night shift's panel (`memory::night`): tonight's queue and what waits
/// from last night.
#[tauri::command(async)]
fn night() -> Result<memory::night::View, String> {
    memory::Store::open_default()
        .and_then(|store| memory::night::view(&store, memory::time::now_ms()))
        .map_err(|e| format!("{e:#}"))
}

/// A night shift job's page: what it made, pictures included.
#[tauri::command(async)]
fn night_job(id: i64) -> Result<serde_json::Value, String> {
    memory::Store::open_default()
        .and_then(|store| memory::night::detail(&store, id))
        .map_err(|e| format!("{e:#}"))
}

/// What the person typed for tonight: Claude turns it into a job, which
/// takes a few seconds. Returns the panel.
#[tauri::command(async)]
fn night_add(text: String) -> Result<memory::night::View, String> {
    memory::Store::open_default()
        .and_then(|store| {
            memory::night::add(&store, &text, chrono::Local::now())?;
            memory::night::view(&store, memory::time::now_ms())
        })
        .map_err(|e| format!("{e:#}"))
}

/// A night shift button (`memory::night::act`): code (Run in Claude Code),
/// approve, redo, keep, discard, remove, open, folder, dismiss, or start.
/// Returns the panel.
#[tauri::command(async)]
fn night_action(
    id: i64,
    action: String,
    value: Option<String>,
) -> Result<memory::night::View, String> {
    memory::Store::open_default()
        .and_then(|store| {
            // Run in Claude Code: the link fills in the prompt; it is then
            // sent for the person, who pressed the button (`claude_app`).
            let prompt = if action == "code" {
                memory::night::get(&store, id)?
                    .map(|job| (memory::night::code_prompt(&job), job.project.clone()))
            } else {
                None
            };
            let view =
                memory::night::act(&store, id, &action, value.as_deref(), chrono::Local::now())?;
            if let Some((prompt, folder)) = prompt {
                thread::spawn(move || {
                    let what = claude_app::send_filled_prompt(&prompt, folder.as_deref());
                    eprintln!("freebyrd: night shift: Claude Code session {what}");
                });
            }
            Ok(view)
        })
        .map_err(|e| format!("{e:#}"))
}

/// The Claude Code sessions of today, for the to-do list's Sessions tab
/// (`sessions::board`).
#[tauri::command(async)]
fn sessions_board() -> sessions::Board {
    sessions::board(chrono::Local::now())
}

/// Who read what today through the context connector, by session, for the
/// Sessions tab (`memory::connector::reads_today`): counts, never text.
#[tauri::command(async)]
fn connector_reads() -> Result<Vec<memory::connector::Reads>, String> {
    let connector = memory::connector::Connector::open_default().map_err(|e| format!("{e:#}"))?;
    let store = memory::Store::open_default().ok();
    memory::connector::reads_today(store.as_ref(), &connector, chrono::Local::now())
        .map_err(|e| format!("{e:#}"))
}

/// The person's word on what a session read: `useful` or `not-useful`.
#[tauri::command(async)]
fn connector_verdict(session: String, verdict: String) -> Result<(), String> {
    memory::connector::Connector::open_default()
        .and_then(|connector| {
            connector.set_verdict(
                memory::connector::TOOL,
                &session,
                &verdict,
                chrono::Local::now().timestamp_millis(),
            )
        })
        .map_err(|e| format!("{e:#}"))
}

/// Opens a session in the Claude app's Code tab.
#[tauri::command(async)]
fn session_open(id: String) -> Result<(), String> {
    sessions::open(&id).map_err(|e| format!("{e:#}"))
}

/// Push: a new Claude Code session in the session's repository commits and
/// pushes what it left. The link fills in the prompt; it is then sent for
/// the person, who pressed the button (`claude_app::send_filled_prompt`).
#[tauri::command(async)]
fn session_push(id: String) -> Result<(), String> {
    let handoff = sessions::push(&id).map_err(|e| format!("{e:#}"))?;
    thread::spawn(move || {
        let what = claude_app::send_filled_prompt(&handoff.prompt, Some(&handoff.folder));
        eprintln!("freebyrd: sessions: push session {what}");
    });
    Ok(())
}

/// What the person typed into the list's Add field. Returns the list.
#[tauri::command(async)]
fn add_todo(text: String) -> Result<memory::todos::List, String> {
    memory::Store::open_default()
        .and_then(|store| memory::todos::add(&store, &text, chrono::Local::now()))
        .map_err(|e| format!("{e:#}"))
}

/// One person's page (`memory::people`).
#[tauri::command(async)]
fn person_page(id: i64) -> Result<memory::people::Page, String> {
    memory::Store::open_default()
        .and_then(|store| memory::people::page(&store, id, chrono::Local::now()))
        .map_err(|e| format!("{e:#}"))
}

/// A correction or note on a person's page: `note` (`value` is the note),
/// `hide` and `show` (not a person), `split` (`value` is a name of theirs
/// that is someone else), `claim` (`value` is an undecided name that is
/// theirs) or `merge` (`value` is the ID of the person they are the same
/// as). Returns the page to show next: theirs, or after a merge, the other.
#[tauri::command(async)]
fn person_action(
    id: i64,
    action: String,
    value: Option<String>,
) -> Result<memory::people::Page, String> {
    use memory::people;
    let run = || -> anyhow::Result<people::Page> {
        let store = memory::Store::open_default()?;
        let now = chrono::Local::now();
        let value = value.as_deref().unwrap_or_default();
        let shown = match action.as_str() {
            "note" => {
                people::set_note(&store, id, value)?;
                id
            }
            "hide" | "show" => {
                people::hide(&store, id, action == "hide")?;
                id
            }
            "split" => {
                people::split(&store, value, now.timestamp_millis())?;
                id
            }
            "claim" => {
                people::claim(&store, value, id)?;
                id
            }
            "merge" => {
                let into: i64 = value.parse()?;
                people::merge(&store, id, into)?;
                into
            }
            other => anyhow::bail!("a page has no action {other:?}"),
        };
        people::page(&store, shown, now)
    };
    run().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn dismiss_card(id: i64) {
    let dismissed = memory::Store::open_default()
        .and_then(|store| cards::dismiss(&store, id, memory::time::now_ms()));
    if let Err(e) = dismissed {
        eprintln!("freebyrd: memory: could not dismiss card {id}: {e:#}");
    }
}

/// Makes the overlay as tall as the island, which the page measures.
#[tauri::command]
fn fit(window: WebviewWindow, height: f64) {
    let _ = resize(&window, height);
}

/// What a button pressed to ask about (`Thread::focus`), for the memory.
fn focus_of(reference: &str) -> Option<memory::plans::Focus> {
    memory::Store::open_default()
        .and_then(|store| memory::todos::focus(&store, reference, chrono::Local::now()))
        .inspect_err(|e| eprintln!("freebyrd: memory: {e:#}"))
        .ok()
}

fn ask_from_terminal(question: &str, picture: Option<&String>, focus: Option<&String>) -> i32 {
    let mut text = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut text) {
        eprintln!("could not read the screen text from stdin: {e}");
        return 1;
    }
    let picture = match picture.map(std::fs::read).transpose() {
        Ok(jpeg) => jpeg.map(|jpeg| picture::Picture {
            width: 0,
            height: 0,
            jpeg,
        }),
        Err(e) => {
            eprintln!("could not read the picture: {e}");
            return 1;
        }
    };
    let screen = ScreenContext {
        app_name: Some("Terminal test".to_owned()),
        window_title: None,
        url: None,
        source: context::Source::Accessibility,
        text,
        truncated: false,
        in_view: None,
        picture,
        no_picture: None,
        elapsed_ms: 0,
        withheld: None,
        ask: None,
    };
    let started = std::time::Instant::now();
    let focus = focus.and_then(|reference| focus_of(reference));
    let topic = match &focus {
        Some(focus) => format!("{} {question}", focus.topic),
        None => question.to_owned(),
    };
    refresh_meetings(&topic, |message| {
        eprintln!("{message} ({} ms)", started.elapsed().as_millis());
    });
    let mut memory = recall(&topic, None);
    if let Some(focus) = focus {
        eprintln!("focus: {} chars", focus.memory.chars().count());
        memory = [focus.memory, memory]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    eprintln!(
        "memory: {} chars in {} ms",
        memory.chars().count(),
        started.elapsed().as_millis()
    );
    let mut first_text = None;
    let result = claude::stream_answer(&screen, &memory, &[], question, |event| {
        if matches!(event, AnswerEvent::Text { .. }) && first_text.is_none() {
            first_text = Some(started.elapsed().as_millis());
        }
        println!("{}", serde_json::to_string(&event).unwrap_or_default());
    });
    eprintln!(
        "first text after {} ms, finished after {} ms",
        first_text.map_or("-".to_owned(), |ms| ms.to_string()),
        started.elapsed().as_millis()
    );
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e:#}");
            1
        }
    }
}

fn read_window(pid: &str, out: Option<&String>) -> i32 {
    let Some(target) = pid.parse().ok().and_then(context::target) else {
        eprintln!("no running app has process ID {pid}");
        return 1;
    };
    let started = Instant::now();
    let screen = context::capture(&target);
    println!(
        "{} via {:?}: {} chars, {} in view, read in {} ms, all done in {} ms",
        screen.app_name.as_deref().unwrap_or("?"),
        screen.source,
        screen.text.chars().count(),
        screen
            .in_view
            .as_ref()
            .map_or(0, |view| view.chars().count()),
        screen.elapsed_ms,
        started.elapsed().as_millis()
    );
    if let Some(reason) = screen.withheld.as_ref().or(screen.no_picture.as_ref()) {
        println!("{reason}");
    }
    if let Some(out) = out {
        let written = std::fs::write(
            format!("{out}.txt"),
            screen.in_view.as_deref().unwrap_or_default(),
        )
        .and_then(|()| match &screen.picture {
            Some(picture) => std::fs::write(format!("{out}.jpg"), &picture.jpeg),
            None => Ok(()),
        });
        if let Err(e) = written {
            eprintln!("could not write {out}: {e}");
            return 1;
        }
    }
    0
}

/// Folds the island back into the notch, then hides the overlay and hands
/// focus back to the app the person came from. Hiding only the window would
/// leave this windowless app active, so typing would go nowhere. If a card
/// is still waiting, the island stays at the notch, glowing.
fn close(window: &WebviewWindow) {
    window
        .state::<Session>()
        .open
        .store(false, Ordering::SeqCst);
    let _ = window.emit("fold", ());
    let window = window.clone();
    thread::spawn(move || {
        thread::sleep(FOLD);
        // A double tap during the fold reopened it.
        if window.state::<Session>().open.load(Ordering::SeqCst) {
            return;
        }
        let _ = window.hide();
        #[cfg(target_os = "macos")]
        let _ = window.app_handle().hide();
        window
            .state::<Session>()
            .clickable
            .store(false, Ordering::SeqCst);
        glow_if_closed(window.app_handle());
    });
}
