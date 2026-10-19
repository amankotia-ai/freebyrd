# Your X bookmarks as context for your AI tools: scope

Status: a scope, with your answers (2026-10-19: the recommended answer to all eight decisions, below). Nothing is built yet. Written on 2026-10-19, after you asked to bring X context into your agentic tools through the context connector. It builds on two documents:

- `twitter-suggestions-rework.md`: how bookmarks are captured and noted, and the four ways you look for one.
- README "Context connector": consent, receipts and the cap, live since 2026-10-18.

It changes one thing in the rework: bookmarks reach you through the tool you are working in when it asks, not through cards pushed into the notch. Build none of it without your go.

## The short version

When you bookmark something on X, you mean "this matters". Today none of it reaches the tools you build with: Freebyrd stopped reading X on 2026-10-16, and the connector serves only to-dos, recaps, meetings, sessions and saved pages.

The proposal: your bookmarks become one more thing Claude Code, Codex and your other tools can ask Freebyrd for, with the same consent, receipts and cap. The tool asks when its task needs a reference. Nothing pops up while you browse.

**An example.** You start a Claude Code session in the Freebyrd folder: "make the notch cards stack and swap when a new one arrives". Claude asks Freebyrd's `references` tool for "card stack swap animation". It gets your bookmarked stacked card swap:

- what it is
- how the motion is built (what moves, in what order, roughly how fast)
- the post's link
- its picture

Claude builds from it straight away. Later you open Codex in Panorama for a pricing section, and Codex asks the same tool for "pricing section". It gets the three pricing layouts you bookmarked, with their pictures. You never paste anything, and the Sessions tab shows that both sessions read your bookmarks.

## What your data shows

Counts only, from `memory.db` and your tools' folders on 2026-10-19:

- **Bookmarks:** 485 kept from Oct 13 to 16. They are ready to serve:
  - 459 have a note (what it is, how to use it, what its pictures show)
  - 472 have a how-to-use
  - 453 describe their pictures
  - 451 have a thumbnail
- **What they are:**
  - design: 397 (landing pages and sections 95, app screens 73, brand 39, motion 38 and 29, 3D and explainers 35)
  - AI: 32
  - engineering: 18
  - other: 31
  - image generation: 6
  - by kind: 373 design references, 37 tools, 35 techniques, 14 prompts
- **Nothing since Oct 16:** capture stopped with the old feature, so anything you bookmarked since is not in the memory.
- **Dates are weak:** 353 bookmarks arrived on Oct 15 in one scroll through your bookmarks page, so "saved last week" cannot be trusted.
- **Where you build:**
  - Claude Code: 278 titled sessions in the last 30 days, and 157 of them (56%) are interface or visual work by their titles (pages, components, cards, modals, motion, logos, renders). Those are the sessions a design bookmark could change.
  - Codex: 75 sessions in the last 30 days, 21 of them in Panorama. It already uses MCP servers (Notion, Linear, Paper), but it has no session-start hook.
  - Cursor: light use, 3 workspaces in 30 days.
  - The Claude and ChatGPT apps: chats, including the brand and logo chats where bookmark cards worked best on Oct 13.
  - Antigravity: installed.

## Why pull instead of push

The old feature pushed bookmarks into the notch, and it failed for reasons pull avoids:

- **It fired while you browsed.** Page visits were its biggest trigger; cards on other people's sites were dismissed or ignored. A tool asks only while it is working on something.
- **It matched the topic, not the decision.** A design bookmark fit any design-flavoured moment well enough to show up. The agent knows the actual task (the files, the request, the plan), so it asks for what would change the next step.
- **It was mostly ignored after day one.** 1 card in 7 was used from Oct 14 to 16. With pull, an unused bookmark costs nothing: no card, no glow.
- **It needed you to paste.** The rework still planned a prompt to copy. An agent uses the reference itself.

The rework's notch card stays possible for one place that cannot ask: a ChatGPT chat in the browser.

## What you would actually want

From your words in the rework and how you work:

1. **Bookmarks only**, never the feed, likes or messages.
2. **At the moment of making**, for the task in front of you: a session building interface, a chat about a brand, a tool to install.
3. **Something the tool can use directly:** how it is built, how to get it (for a tool), and the picture, since you steer design with reference images.
4. **Found the way you remember it:** how it looks ("that painterly hero"), what it is for ("an onboarding screen"), who posted it ("the Bakers Studio one"), or the tool's name ("Anatomy").
5. **In every tool you build with**, so Codex tomorrow gets what Claude Code got today.
6. **No look-backs and no standing jobs.** Nothing weekly, nothing to approve.
7. **The same trust as the connector:** consent per tool and project in the notch, a receipt for every answer, a cap, and Useful or Not useful per session.

## When it answers, and when it doesn't

**Yes:**

- A tool you allowed asks `references` while working: "card stack swap", "pricing section", "an MCP server for Figma".
- You ask in that tool: "anything in my bookmarks for this?"
- (If you choose it) the session-start index carries one line when your bookmarks fit the project's live threads: "Bookmarks: 6 fit this work (ask references)".

**No:**

- Before your Allow for that tool and project, or past the day's cap.
- Feed posts, likes, replies, direct messages, or a bookmark you removed.
- Freebyrd's own Claude runs, night shift and scheduled jobs (they load no MCP servers).
- Pushed into the notch, unless you choose the ChatGPT-in-the-browser card later.

## Privacy limits

- **Only bookmarked posts:** never the feed, likes or messages; the direct-messages exclusion stays.
- **What a tool gets:**
  - the note Freebyrd wrote (marked guessed)
  - the post's address and author handle
  - the date captured (marked confirmed, since you pressed Bookmark)
  - the picture only when the ask wants one
  - in the first version, not the post's own words, which are someone else's text (your decision 4)
- **Each tool's destination is its own:** what Claude Code and the Claude app get goes to Anthropic, Codex's goes to OpenAI, Cursor's to its model providers. Each tool gets its own consent card and its own line in "What leaves the Mac" before it ships.
- **Who is asking** is checked by the parent process, per tool, as for Claude Code today.
- **Receipts** record the bookmarks' references (`x:<status>`), sizes and a SHA-256, never the text or the picture.
- **Capture** follows the rework's limits: no hidden tabs in your browser, and fetches go only to X's public post data, carrying nothing of yours.

## A small first version

1. **Serve the 485 bookmarks you have, in Claude Code.** `references` searches bookmarks and saved pages together, the four ways above, at most 8 results. No new model calls. (The "Serve X bookmarks to Claude Code via the connector" card is this step.)
2. **Pictures on request.** For a design ask, up to 3 thumbnails come back as images the tool can see, inside their own daily cap.
3. **Capture new bookmarks again.** The rework's bookmark-only reader: a post is recorded when you press Bookmark or open your bookmarks page, never from the feed. One note per new bookmark (about 8 a day), the same Sonnet step as before.
4. **Codex.** The same MCP server in `~/.codex/config.toml`, its own consent card, and its parent-process check. Codex has no session-start hook, so one line in `~/.codex/AGENTS.md` tells it to read the index at the start and ask for bookmarks while building (your decision 7).
5. **Measure for two weeks** (below). Keep it, tighten it, or make it ask-only in fewer tools.

**Not in the first version:**

- the Claude app's chats, Cursor, Antigravity
- the ChatGPT-in-the-browser card
- bookmarks on to-dos
- night shift briefs
- feed posts, Jev, hidden tabs

## Later, if the first version earns it

- **The Claude app's chats** through its local MCP setting (`claude_desktop_config.json`), for brand, logo and image chats, where bookmark cards worked best on Oct 13.
- **Cursor**, through `~/.cursor/mcp.json`, if you use it more.
- **ChatGPT in the browser:** the rework's one-card-per-moment, the only push, for a place no tool can ask from.
- **Design tasks:** a Website Task or design to-do carries its 1 to 3 fitting bookmarks in its thread, so an agent working on it gets them without searching.
- **Night shift briefs** for a design task due tomorrow, from your bookmarks.

## Decisions for you

1. **Pull only** (recommended), or also the rework's notch card for making moments.
2. **Which tool after Claude Code:** Codex (recommended, 75 sessions this month), then the Claude app's chats; Cursor later.
3. **Pictures:** sent as images when a design ask wants them (recommended), or text only.
4. **The post's own words:** left out, the note only (recommended), or sent along, since posts are public.
5. **Which projects:** bookmarks answer in any project you allowed, since they are yours and not a project's (recommended), or only where they match the project's own words.
6. **A line in the index** when at least 3 bookmarks fit the project's live threads (recommended), or nothing until the tool asks.
7. **Codex's start:** a line in `~/.codex/AGENTS.md` telling it to read the index and ask for references (recommended), or ask-only.
8. **Old X data:** keep the 485 bookmarks, notes and thumbnails (recommended). Delete the 4,129 feed posts that were never bookmarked, or keep them unread.

**Your answers (2026-10-19): the recommended one for each.**

1. Pull only: no notch card for bookmarks.
2. Claude Code first, then Codex, then the Claude app's chats; Cursor later.
3. Pictures go as images when a design ask wants them.
4. The note only, never the post's own words.
5. Bookmarks answer in any project you allowed.
6. The index gets one line when at least 3 bookmarks fit the live threads.
7. Codex starts from a line in `~/.codex/AGENTS.md` that tells it to read the index and ask for references (added only with the Codex step, after you see the line).
8. The 485 bookmarks, notes and thumbnails are kept and reused. The 4,129 feed posts had no recommendation, so they stay as they are, unread; deleting them is still your call.

---

## Technical detail

### The ask (`memory::connector`, `references`)

- **Input:** `query` (a few words), `kind` (`bookmark`, `page` or `any`, default `any`), `pictures` (true or false, default false).
- **Search:**
  - FTS over `x_idea_fts` (name, what, use_when, words, shows, how), plus the author's handle and name from `x_post`, plus saved page titles as today.
  - Bookmarks are those with `bookmarked_at` set and `unbookmarked_at` not set, and an `x_idea` in state `idea`.
  - The rework's categories "Business, career and essays" and "Not useful" stay out.
  - Repeats merge, as the rework decided (Anatomy from 7 posts is one entry with 7 sources).
- **Ranking:** words in the note's name and use count most, then shown style tags, then the author. Newest capture breaks ties (dates are weak).
- **Each result:**
  - name and one line
  - how to use it, and how to get it for a tool
  - what the picture shows
  - address `https://x.com/i/status/<status>`
  - author handle
  - date captured
  - the line ends `(date, X bookmark, guessed)` for the note, as the brief's lines do
- **Pictures:** an MCP `image` content block per result, from `x_open.thumb` (360 px JPEG), at most 3 an answer and 40 a day per tool, counted in their own receipt column. Whether Claude Code and Codex both show tool images to the model is checked first, with a made-up memory as on 2026-10-18.
- **Receipts:** items `x:<status>` and `page:<id>`, characters, bytes of pictures, SHA-256 of the text.

### The index line (`memory::threads`)

- **The count:** bookmarks whose notes share two distinctive words with any live thread's items, as `matches` decides for to-dos.
- **When it shows:** with at least 3, the index gets "Bookmarks: N fit this work (ask references)" inside its 420 characters, dropping the lowest thread if needed.

### Capture and notes (`extension/content.js`, `memory::bookmarks`)

- As in `twitter-suggestions-rework.md` "Capture" and "Notes":
  - the bookmark-only reader
  - `imported` for bulk scrolls
  - public post data for the text and media
  - up to 3 pictures
  - one tool-less Sonnet note
  - at most 20 a day
- **Note fields:** the existing `x_idea` columns, plus `author_tags` for the recurring authors and `style` tags for the recurring styles (dark mode, isometric, dither, painterly, glass, hand-drawn), so all four ways of asking find them.

### Other tools (`connector::Tool`)

- **Tools as a type:** `TOOL` becomes `Tool { ClaudeCode, Codex, ClaudeApp, Cursor }`. Each has its own name in consent, receipts, the cap and the notch line ("Codex read 4 items about Panorama today").
- **Who is asking, per tool:** the executables of the parent processes.
  - Codex runs from the ChatGPT app's bundled `codex-cli`.
  - The Claude app and Cursor have their own paths.
  - A process another tool started gets its own consent card.
- **Codex:**
  - registration: `[mcp_servers.freebyrd]` in `~/.codex/config.toml`
  - its folder: from the server's working directory, or the session's roots
  - its session ID: from what Codex sends (to find, as on 2026-10-18 with a logger)
- **The Claude app:** `mcpServers.freebyrd` in `claude_desktop_config.json`. A chat has no folder, so it answers only bookmarks and saved pages, never a project's threads, under its own consent "the Claude app's chats".

### What leaves the Mac, for the README

- **To Anthropic through Claude Code, and to OpenAI through Codex,** for a project allowed in that tool:
  - bookmark notes, addresses, author handles and capture dates that match an ask
  - their pictures when asked
  - all redacted, within the day's caps
- **To X's public post-data service:** a new bookmark's ID, for its text and media (as the rework).
- **To Anthropic (Sonnet):** each new bookmark once, for its note (as the rework).

### Budget

- **Serving:** plain code, no model calls; tens of milliseconds per ask.
- **New notes:** about 8 to 20 a day, about 13 s each (as the rework).
- **Pictures:** bounded by their daily cap.

### Measuring and tests

- **Two weeks, counts only, per tool:**
  - asks that returned bookmarks
  - interface sessions with at least one such ask, against the 56% of sessions that are interface work
  - pictures sent
  - your Useful or Not useful on those sessions
- **Keep it if:** a tool asks for bookmarks in at least 1 in 5 interface sessions, and you mark at least half of the marked ones useful. Otherwise drop the index line and keep it ask-only, or stop it in that tool.
- **Unit tests:**
  - each of the four ways of asking finds its bookmark
  - a feed post, an unbookmarked post and a "Not useful" one are never found
  - the post's own text never goes
  - repeats merge
  - pictures stay within their caps
  - receipts carry `x:` references and no text
  - another tool's process gets its own consent
  - the index line appears only with 3 fitting bookmarks
- **By hand:**
  - a Claude Code session building a card animation
  - a Codex session in Panorama building a section
  - a session in a folder not allowed
