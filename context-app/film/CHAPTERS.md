# Freebyrd film: a day, in chapters

One long film, cut into chapters that can also play on their own. The last
frame of each chapter is the first frame of the next, so they join with no
cut, and chapter 6 ends where chapter 1 begins, so the day loops.

Every chapter follows the same loop:

1. **Context**: something real happens in an app on screen.
2. **Notch**: Freebyrd acts on that context (a glow, a card, an answer, a draft).
3. **App**: back in the app, the person does the thing (paste, send, add, open).
4. **Notch again**: Freebyrd picks up what just happened.

The Unifize platform film's look: a 14-inch MacBook Pro, square-on camera,
180-degree motion blur, charcoal ground, glow and grain (a third lighter than
Unifize's). Rendered in EEVEE. The notch is Freebyrd's real UI; the camera always
keeps the whole notch window in frame while it is open.

Camera (2026-10-15, after viewers of the first cut felt motion sick): locked
off. The film pushes in from the laptop once, at the start of chapter 1, and
pulls out once, at the end of chapter 6; in between it holds a shot of the
screen while the UI and the cursor do all the moving, and chapters hand over
in that held shot. Never faster than 0.3 frame widths a second (the first cut
peaked at 1.4 and moved two thirds of the time). When something happens out
of the shot, the app scrolls it into view; the camera does not go to it.

People, company and emails are made up (Sam Ellis, Nina Park at Halden, Leo
Grant, Asha Rao).

Agreed 2026-10-15 (the first chapter 2, built around night shift, had no setup and
was dropped): one consecutive storyline. Its spine is the promise Sam makes in
chapter 1, "the revised launch plan by Thursday noon, with the new pricing tier on
page 4", carried through one Wednesday until it is kept. Each chapter shows
Freebyrd doing one new thing, in a new place, following from what the last one
left; the people in chapter 1's inbox (Nina, Asha, Leo, Dev, Tom) come back.

| # | Time | Freebyrd... | Where | What happens |
|---|---|---|---|---|
| 1 | 09:12 | **sees what you're looking at** (built) | Mail | Nina's email; a drafted reply; the promise to the calendar; the kickoff booked |
| 2 | 09:20 | **knows what you owe** (built) | the tasks view, then Calendar | the promise just made is on the list beside Asha's checklist (Notion) and Leo's SOW (Waiting on); Plan my day fits them into today, agendas written; Calendar shows the day; at 09:30 the first block alerts and Up next offers Draft it |
| 3 | 09:30 | **helps you do the work** (built) | Chrome, the launch plan in Notion | Draft it: page 4 drafted from Dev's tier sheet and Tom's review notes; Copy, ⌘Tab, pasted in; "What's left here?" catches the pilot date still saying the 19th; fixed to the 26th |
| 4 | 11:30 to 12:52 | **gets you ready, and catches what's said** (built) | Chrome: the launch plan, then the call | the call's alert 30 minutes before; Up next, Prep me; on the call Nina asks Sam to confirm the yearly price (the guess from chapter 3) and promises the pilot team list; after it the promise is a Due card with an agenda, it is due today on the list, and Nina's list is under Waiting on |
| 5 | 14:30 | **brings the right thing at the right moment** (built) | Claude, a Claude Code session on the launch page | the session's first message (the pricing section) brings an Idea card: a bookmarked pricing table and a page on yearly toggles, with a prompt; copied and pasted into the running session; it works to 14:44, tests pass; Sessions: 4 files not pushed, Push |
| 6 | 17:05 to 17:32 | **follows through before you ask** (built) | Mail | Dev's answer lands and the promise to him is kept by itself (the "1 due today" goes); at 17:30 the Draft card holds the email to Nina with the plan, nothing missing; copied, Reply, sent; Up next moves on; the list's Done holds both promises; Waiting on, Leo's SOW: Draft a check-in |
| 7 | 21:30 | **keeps going while you're away** | the notch at night | Stop for tonight hands the unfinished launch page work to night shift; the camera pulls out, the display sleeps: chapter 1's first frame |

The Halden call is at 12:00 so that at 09:30 Up next is the promise, not the call
(an event takes Up next from 2 hours before it starts).

## Chapter 1, as built

`film.html`, 51.2 s in seven render steps (starts 0, 6.6, 13, 23.2, 29.2, 34.7, 44). A 4 s
push-in to one shot (MAIN: the screen's full width from the notch down to the reply
window), held to the end. The beats are authored in story time and the edit
(`EDIT` in film.html) adds or takes out seconds at quiet moments; re-pacing is a
one-line change there.

1. 0 to 6.6: asleep, the display wakes on Leo's email; the push-in; Nina's email lands (a notification banner, her row at the top of the inbox); it is clicked open.
2. 6.6 to 13: her email is read (her two asks: the plan by Thursday, a pilot date); a double tap of Option opens the notch on it; "Draft a reply".
3. 13 to 23.2: the reply is drafted in 6 s (her email, plus the pilot date from Monday's call), held to be read; Copy.
4. 23.2 to 29.2: Reply (the click on Mail closes the notch), paste, Send; the reply joins the thread.
5. 29.2 to 34.7: the promise lands as a Due card: glow, open, "On your calendar for Thu 15 Oct, 12:00".
6. 34.7 to 44: Nina answers with a kickoff time (the thread scrolls it into the shot, 3 s to read); the event card, filled in, read; Add to calendar.
7. 44 to 51.2: Reply, "Booked for the 26th at 10:00" typed in 3 s and sent; the thread scrolls; a beat.

First frame: the laptop, display asleep. Last frame: the held shot, Mail showing the four-message thread at 09:19, notch closed, no cursor (chapter 2's first frame).

The first cut (41.2 s, a 45-key camera that moved two thirds of the time) is rendered in
`~/Downloads/freebyrd-film-v1-blender/ch1/`; this one goes to `ch1-v2/`.

## Chapter 2, as built

`film2.html`, 39.5 s in five render steps (starts 0, 9, 16, 25, 31.5). It starts on chapter 1's
last frame (checked pixel for pixel) and holds the same shot throughout.

1. 0 to 9: 09:20; the notch over Mail: Up next is the promise just made ("Due tomorrow, 12:00", 2 more); the checklist icon opens the list: To do (the promise, Asha's pilot checklist and the pricing section from Notion, Tom's fixes from yesterday's review), then Waiting on (Leo's countersigned SOW, Tom's hero image).
2. 9 to 16: the list closes; "Plan my day": the list laid into today around the Halden call at 12:00, with the balance line; Add to calendar on the launch plan: "Writing its agenda…", then its four steps.
3. 16 to 25: down the plan: Asha's checklist and the pricing section added with agendas, lunch and a walk added.
4. 25 to 31.5: ⌘Space, "Cal", Calendar: the day in the Freebyrd calendar; the launch plan clicked: its notes hold the agenda.
5. 31.5 to 39.5: the minutes run to 09:30 (the clock and the red line); the block starts and Calendar alerts; the notch opens over Calendar: Up next, the promise, Draft it.

Last frame: the notch open over Calendar at 09:30, Up next with Draft it, the cursor resting under it (chapter 3 presses it).

## Chapter 3, as built

`film3.html`, 31.5 s in five render steps (starts 0, 8, 14, 20, 26). It starts on chapter 2's
last frame (checked pixel for pixel) and holds the same shot throughout.

1. 0 to 8: 09:30; Draft it on Up next: "Pages 1 to 3 of the launch plan are done in Notion; page 4, the new tier, is missing", then page 4 as a draft, from Dev's tier sheet and Tom's review notes.
2. 8 to 14: Copy ("Copied"); ⌘Tab, Tab past Mail to Google Chrome: the Halden launch plan in Notion, 4. Pricing empty.
3. 14 to 20: the empty line clicked, ⌘V: page 4 is in; the notch again, over the page (What's left here?, Summarize this, Plan my day; Up next still the promise).
4. 20 to 26: "What's left here?": 2. Pilot still says after the 19th though Nina was told the 26th and the kickoff is booked; Dev's yearly Growth price is a guess.
5. 26 to 31.5: "19th" double-clicked (the notch folds), "26th" typed.

Last frame: Chrome on the launch plan at 09:34, "26th" just typed, notch closed, the cursor beside it (chapter 4's first frame).

Not shown, because Freebyrd does not do it: a to-do's "Latest" line moving on a Notion page edited by hand (`progress.rs` reads Claude Code sessions and work threads). Links in answers are text, not clickable, so Sam switches to Chrome with ⌘Tab.

## Chapter 4, as built

`film4.html`, 41.5 s in five render steps (starts 0, 7, 16.6, 27.6, 35.6). It starts on chapter 3's
last frame (checked: no pixel differs by more than 3 shades) and holds the same shot.

1. 0 to 7: the clock runs from 09:34 to 11:30 (the cursor away); Calendar's alert, 30 minutes before the call (`TIMED_ALERT_AFTER_S`); the notch: Up next is the call, "Starts in 30 min", Prep me and Hide (an event takes Up next from 2 hours before it).
2. 7 to 16.6: Prep me: who (Nina and Tom), what Monday's call left, what is still open, Dev's yearly price a guess.
3. 16.6 to 27.6: to 11:59, the call's tab opened meanwhile; Join now; the call at 12:00, four tiles as initials (Fireflies' notetaker among them) and captions: Nina asks Sam to confirm the yearly price before tomorrow, Sam will with Dev today, Nina will send the pilot team list by Friday; the clock to 12:45; ⌘W.
4. 27.6 to 35.6: to 12:52, Fireflies' notes read (`connectors.rs`): Sam's promise is a Due card ("From the meeting “Halden call”", with its agenda); the notch now offers Recap Halden call; Up next is the promise, due today.
5. 35.6 to 41.5: the list: Today, the promise; Waiting on, Nina's pilot team list (due Friday) above Leo's SOW and Tom's hero image.

Last frame: the notch open on Waiting on over the launch plan at 12:52, the cursor on the tab (chapter 5's first frame).

## Chapter 5, as built

`film5.html`, 42 s in six render steps (starts 0, 6, 11.6, 19.3, 26.6, 33.8). It starts on chapter 4's
last frame (checked: no pixel differs by more than 3 shades) and holds the same shot.

1. 0 to 6: a click on the page closes the notch (the closed notch keeps "1" due today on its right, `live.rs`); the hours pass to 14:30 and Claude comes to the front; Sam finishes a new Claude Code session's first message (the pricing section: three tiers, Growth highlighted, a monthly and yearly switch) and sends it; the session runs, its working dots beside the notch.
2. 6 to 11.6: Claude reads the page and the pricing doc; the first message is weighed as a moment (`moments.rs`) and an Idea card lands: the notch glows.
3. 11.6 to 19.3: the card (quick asks for Claude, Up next the promise due at 17:00): "Pricing section: Three-tier table with a highlighted plan and 1 more", the bookmarked post's picture, why, a bookmark and a page; the cards scroll down to its buttons; Copy prompt; into the prompt box, ⌘V, Return.
4. 19.3 to 26.6: the session works to 14:44 (the cards, the switch, the page, 24 tests pass); done, with a link to the page on localhost: the dots give way to "1" waiting on Sam.
5. 26.6 to 33.8: the link opens the launch page in Chrome: the new pricing section, Growth raised as Most teams, the same rows in each card; Yearly tried: £490 and £890 a seat a year.
6. 33.8 to 42: the notch over the page (quick asks for something built on this Mac: Ideas from my bookmarks, What do I know about this?), the list, Sessions: "Launch page pricing section · Done 1 min ago · 4 files not pushed"; Push: "Opening a session to push it…", and a new session to commit and push it appears in Claude.

The user's note (2026-10-15): look at the work before pushing it, on the page it changed, not from the chat.

Last frame: the notch open on Sessions over the launch page at 14:45 (chapter 6's first frame).

## Chapter 6, as built

`film6.html`, 35.5 s in five render steps (starts 0, 6.6, 14.4, 20, 27). It starts on chapter 5's
last frame (checked: one cursor-edge pixel differs, by 18 of 765) and holds the same shot.

1. 0 to 6.6: a click closes the notch; the hours pass to 17:05 with Mail in front; Dev's answer lands ("Confirmed: £490 a seat a year"), a banner and a row; the "1 due today" beside the notch goes (`progress.rs`: a reply in a tied thread settles the promise); to 17:30.
2. 6.6 to 14.4: the notch glows with the Draft card (`drafts.rs`): "Due Thu 15 Oct, 12:00", "Send Nina the revised launch plan", "A draft for Nina, by email", the email itself (the plan's link, the Growth price, the pilot date), What's done; its Copy.
3. 14.4 to 20: Reply in Mail (the notch folds), ⌘V, Send: the thread's fifth message, at 17:31.
4. 20 to 27: the notch: Up next has moved on to Asha's checklist; the list: To do, then Done (both promises, with their Latest lines: sent with the plan; Dev confirmed £490 a year), then Waiting on.
5. 27 to 35.5: Leo's SOW opened (From Mail, 08:47), Draft a check-in: a short note to Leo, drafted.

Agreed with the user (2026-10-15): the Draft card arrives at 17:30 here, though the app would prepare it on the morning the promise is made (from the working day before it is due, `LEAD_DAYS`).

Last frame: the notch open on the check-in to Leo at 17:32 (chapter 7's first frame).

