# The context connector: scope

Status: a scope only. Nothing here is built. Written on 2026-10-17, from the third Claude and GPT-6.1-Sol conversation ("turn the stack upside down": your system holds the records and permissions, and AI tools are workers that come to it) and `personal-root-scope.md`. Build none of it without your go.

## The short version

Today Freebyrd answers only in the notch. Every other AI tool you use starts from nothing, and you fill it in by hand. The connector lets Claude Code, and later Codex and others, ask Freebyrd for your context. You grant access one project at a time, every answer leaves a receipt, and each tool has a daily cap. You stop re-explaining yourself, and no tool keeps its own copy of you.

**An example.** You open a new Claude Code session in Unifize Monorepo 2 and type "build the persona pages". Before your first prompt, the session already has a short brief from Freebyrd:

- Ben asked for persona pages in Tuesday's Website review, and they're due Friday.
- Raj wanted fewer cards per row.
- Ten files already exist under `src/app/explorations/personas`, five of them uncommitted.
- An earlier session on this stalled, waiting on copy.

Midway, the session asks Freebyrd "what did Raj say about the cards?" and gets the meeting's line, with its date. The notch shows a quiet line: "Claude Code read 2 items about Unifize."

## Why it can be the strongest part of Freebyrd

Every AI company wants to be where you keep your context, and each one sees only its own tool. Littlebird already lets outside tools search its memory through a connector of its own, served from its cloud (its facts page; `littlebird/FINDINGS.md` §2 and §4 cover where its data goes). Search alone is not the edge.

Four things are:

1. **The why behind the work.** Claude Code knows the repo and ChatGPT knows the chat. Only Freebyrd also knows who asked, what was promised, what the meeting decided and which earlier session stalled.
2. **Correct once, every tool knows.** One record of decisions and corrections, marked seen, guessed or confirmed. A tool can propose a correction, and your press makes it true for every tool (version two).
3. **Trust you can see.** Consent per tool and project in the notch, a receipt for every answer, a daily cap, and a line saying who read what.
4. **Switch tools freely.** Your context lives on your Mac, so Codex tomorrow gets the same brief Claude Code got today.

## What your memory shows

Counts only, from your Claude Code prompts in `memory.db` on 2026-10-17:

- **Prompts and sessions:** 1,738 prompts across 386 sessions.
- **First prompts carry the setup:** they average 271 characters, against 103 for later prompts. You spend each start setting the scene.
- **Context from outside the repo, typed by hand:** 59 prompts name Ben, Raj or Lakshman, 38 mention a meeting or a call, and 91 mention Notion.
- **Corrections:** about 32 prompts read like corrections ("no, ...", "that's not ..."). This is a rough keyword count.
- **From the README:** you run 2 to 10 sessions at once, and typed "push changes to git" 26 times.

The connector would replace the hand-carried part of those first prompts.

## What you would actually want

From your decisions so far:

1. **Your own work first.** Claude Code is where you live (386 sessions). Start there.
2. **Nothing sent without a reason you can see.** Every disclosure already has an entry in "What leaves the Mac". The connector adds one per tool, and receipts show what actually went.
3. **No look-backs.** You close recaps without opening them. The brief arrives inside the work, at the start of a session, never as a digest.
4. **Your rules carry over.** Personal chats stay personal, other people's messages and page text stay out by default, and anything a tool sends back is untrusted.

## When it answers, and when it doesn't

**Yes:**

- A Claude Code session starts or resumes in a folder you have allowed: the brief, once.
- The session asks: a scoped search, or saved references for something it is making.

**No:**

- **A folder you have not allowed:** the tool is told to ask you in the notch, and gets nothing until you do.
- **Freebyrd's own runs:** they run with `--setting-sources ""`, so no hook fires.
- **Night shift and scheduled jobs:** same reason.
- **Past the day's cap:** the tool is told the cap is reached.
- **Personal chats, WhatsApp, X direct messages, screens, window pictures and raw page text:** never, in the first version.

## What it serves

Built by plain code from what the memory already has, short and sourced:

- **Open promises and due dates** (`plan`) and **to-dos** (`todo`) that match the project.
- **What each recap says is done and open for the project** (`recap`, per-project `done` and `open`).
- **Meetings** that match the project: Fireflies summaries and action items.
- **The project's Claude Code sessions:** recent titles, which ones stalled and why (from `sessions.rs`'s reading), and what is left to push.
- **People involved:** names and roles from their pages, never message text.
- **Saved references** for something being made: pages you saved, and X bookmarks once the X rework ships. The old X tables stay unread until you say so.

Each item carries a date, where it came from, and a status:

- **seen:** read from a source, such as a meeting line, a promise in a thread, or a commit
- **guessed:** written by a model, such as a recap line or a summary
- **confirmed:** you pressed something, such as Add, Keep, Done or a verdict

## Privacy limits

- **The destination.** What Freebyrd hands Claude Code goes to Anthropic, and what it hands Codex goes to OpenAI. Each tool gets its own entry in "What leaves the Mac", and its first answer waits for your Allow in the notch.
- **Scope:** one tool and one project at a time. An allowed Unifize grant answers nothing about Panorama or Freebyrd.
- **What never goes:**
  - personal chats (settled personal, or unsettled)
  - WhatsApp messages
  - X direct messages
  - the screen or a window's picture
  - page text and other people's message text

  These stay out in the first version.
- **Who is asking.** Freebyrd checks the app that started the connector (its parent process) against the tools you allowed, not just the name the request gives.
- **Consent lives in the notch,** where the requesting tool cannot press it for you.
- **Receipts** record what went (sections, counts, item IDs and a hash), never the text. They use the same table as `personal-root-scope.md` change 7.
- **Writing back** (version two) is a proposal only. It becomes a card in the notch and is never stored as fact until you press.

## A small first version

1. **Claude Code only, read-only.**
2. **The brief at session start,** through a Claude Code SessionStart hook, at most 1,500 characters. It's sent on `startup`; `resume` sends only what changed since that session's last brief.
3. **Two asks through the connector:**
   - `recall`: a scoped search with sources, dates and status
   - `references`: saved pages for something being made
4. **Consent per folder,** the first time, in the notch: Allow, Allow once, Not this project.
5. **Receipts and a cap:** 40 asks and 30,000 characters a day per tool, proposed first numbers.
6. **A line beside the closed notch** while a session reads, and a Who read what list in the to-do list's Sessions tab.
7. **Measure for two weeks** (below). Keep it, tighten it, or remove it.

**Not in the first version:**

- Codex or any other tool
- writing back
- X bookmarks (until the rework)
- message text
- decisions as their own record (the brief uses what recaps, meetings and promises already hold)

## Later, if the first version earns it

- **Codex,** through its `config.toml`, with its own consent and its own "What leaves the Mac" entry. Other tools as you ask for them. Whether the ChatGPT desktop app can use local connectors is not confirmed, and its connectors sit behind Developer Mode. If it can only reach servers on the internet, it stays out, because the connector stays on the Mac.
- **Proposals back.** A tool proposes "we dropped the pricing page" or "Raj approved the 3-column layout". The notch shows it as a card, and Yes makes it confirmed for every tool. This is "correct once, every tool knows".
- **Decisions as a record of their own,** fed by those proposals and your presses, with dates and what they replaced. This is the decision ledger from the conversations.
- **Handoffs.** "Continue the logo chat in Claude Code": the brief carries what the chat settled (your messages, never the replies).
- **References from X bookmarks,** once the rework's first version is live.

## Decisions for you

1. **First tool:** Claude Code only (recommended), or Claude Code and Codex together.
2. **The brief:** automatic at session start (recommended), or only when the session asks.
3. **Consent:** once per tool and folder (recommended), or every session.
4. **What it may serve:** work items only, as listed above (recommended), or also short excerpts from work threads and emails.
5. **Caps:** 40 asks and 30,000 characters a day per tool (recommended as a first guess), or other numbers.
6. **Writing back:** proposals in version two (recommended), or never.
7. **Which folders:** start with Unifize Monorepo 2 alone (recommended, most of your sessions), or every project in `jev/project-groups.json`.

---

## Technical detail

### The server (`crates/connector`, a new crate)

- **Binary:** `freebyrd-connector`, a stdio MCP server, installed beside the others in `~/Library/Application Support/Freebyrd/bin/`.
- **Registration:** in your user-level Claude Code configuration, not in each repository. It runs with `CLAUDE_PROJECT_DIR` from Claude Code, which names the project folder.
- **The memory:** it opens `memory.db` read-only, plus its own small writable tables for grants, receipts and pending consent, so a bug cannot change the memory.
- **Tools:** named `mcp__freebyrd__recall` and `mcp__freebyrd__references`. Each takes a short query and returns items as text inside a data element (`claude_cli::data_element`'s format), marked as data, with source, date and status per item.
- **Identity check:** the parent process's executable is matched against allowed tools: the Claude desktop app's bundled CLI, or `claude` found by `claude_cli::find_cli`. Anything else gets nothing and a log line.

### The brief (`connector::brief`, plain code)

- **The hook:** a SessionStart hook in your Claude Code settings runs `freebyrd-connector brief`. It reads the hook input (`cwd`, `source`, `session_id`) and prints JSON with `hookSpecificOutput.additionalContext`, which Claude Code wraps in a system reminder before the first prompt. Claude Code caps it at 10,000 characters; the brief keeps to 1,500.
- **Project:** `cwd` mapped to a project through the `project` table and `jev/project-groups.json`. A worktree under `.claude/worktrees` counts toward its repository, as ingestion already does.
- **Matching:** promises and to-dos have no project column, so they are matched by the project's distinctive words. That is the same approach as night shift's already-built check (`night::suggest`'s `already`): two words needed when a title has two or more, and generic words left out. Recaps match by project name in their per-project lists. Meetings match by title words and people. Matching is the weakest part; the measurement below checks it.
- **Order of the brief:**
  1. what is due soonest
  2. what is open from the latest recap
  3. the latest matching meeting's action items
  4. sessions that stalled or have work left to push
  5. the people involved

  Each line ends with its date and status.

### Consent and caps (`connector::grant`)

- **Tables:** `grant (tool, project_id, kind, created_at, revoked_at)` and `pending_consent (tool, project_id, asked_at)`.
- **A first request from an unknown pair:** it waits, and the notch shows a card (kind `access`, right side, a solid key icon in `KINDS`) with Allow, Allow once and Not this project. The tool gets "waiting for the person's OK in Freebyrd".
- **Caps per tool per day:** asks and characters, counted in the receipts. Past the cap, the tool is told so once and gets nothing more that day.

### Receipts (`receipt`, shared with change 7 of `personal-root-scope.md`)

- **One row per answer:** tool, project, ask kind, item IDs, characters, a sha256 of what was sent, and the time. No text.
- **CLI:** `freebyrd-memory receipts --tool claude-code --day D` lists them, and `--row ID` shows which answers included an item.
- **The closed notch:** "Claude Code read N items about P today", counts only, from the receipts (`live.rs`).

### The echo loop

- **The risk:** Claude's replies are ingested as replies. A reply that repeats something from the brief would come back looking like a second, independent source. This is the contamination loop from the conversations.
- **The fix:** sessions that received a brief are marked by `session_id` from the hook input. Recall then never counts a reply from such a session as evidence for an item the brief carried, matched by the receipt's item IDs.
- **What ingestion already handles:** it drops system reminders and tool results from typed prompts (`ingest::claude_code`), so the brief and connector answers are not read back as something you typed. Keep that test.

### Injection

- **What is served by default:** short lines written by Freebyrd's own plain code (titles, dates, names, statuses) and summaries already in the memory. Never raw page text or other people's messages.
- **How it is marked:** everything goes inside a data element, with a line telling the tool it is data about the person's work, not instructions.
- **Redaction:** `memory::redact` runs on every answer before it leaves, as for Claude calls today.

### What leaves the Mac, for the README

- **To Anthropic, through Claude Code, for a folder you allowed:**
  - the brief at session start: dates, titles, names, statuses and short summaries for that project
  - answers to the session's `recall` and `references` asks: the same kinds of items, at most the day's cap
  - all of it redacted
- **Nothing else:** no message text, page text, screen or picture, and nothing about other projects.

### Budget

- **No model calls:** the brief and both asks are plain code over the memory.
- **Context in Claude Code:** at most 1,500 characters at start, plus whatever the session asks for, within the cap.

### Measuring and tests

- **Two weeks, from the memory, counts only:**
  - **Length of your first prompt** in sessions that got a brief, against the 271-character average before.
  - **Corrections in the first three prompts,** by the same rough keyword count.
  - **Whether sessions use the connector** after the brief (`recall` and `references` asks).
  - **Your verdict:** a Not useful on the notch's receipt line, per session.
- **Keep it if:**
  - median first-prompt length falls by a third
  - you mark briefs useful in at least half the sessions you check

  Both are proposed bars. Otherwise trim the brief or make it ask-only.
- **Unit tests:**
  - folder to project, including worktrees
  - matching promises and to-dos to a project, both hits and near misses
  - brief order and length
  - consent waiting, allowed and refused
  - caps
  - a parent process that is not an allowed tool is refused
  - receipts written, with no text
  - a reply from a briefed session is not counted as new evidence
  - nothing outside the project leaks
  - none of them reaches Anthropic
- **By hand:**
  - a session in Unifize Monorepo 2 with the hook on
  - one in a folder not yet allowed
  - one after the day's cap
