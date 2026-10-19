# Freebyrd as a personal root: scope for seven changes

Status: a scope only. Nothing here is built. Written on 2026-10-17, after three long conversations between Claude and GPT-6.1-Sol about InFlight (the local model over your bookmarks that started all this), and a comparison with Littlebird. Build none of it without your go.

## The short version

Freebyrd already has most of what those conversations ended on. Your memory lives on this Mac (`memory.db`). Plain code works over it before any model sees it. "What leaves the Mac" in the README lists every disclosure. Cards only show, and Copy is the way out. Night shift's fences bound what a run can change, and Keep is your press.

Seven gaps remain. They are also what keeps Freebyrd from being Littlebird with a local cache: Littlebird builds its journals, world model and routines on its servers, and ships no on-device text model (`littlebird/FINDINGS.md` §5.0).

**An example.** You are on a flight with no Wi-Fi and double-tap Option: "what did Ben and I agree about the persona pages?" Today the notch gathers the memory for that question in about 150 ms, then stops, because every answer goes through the `claude` CLI. With change 1, a small model on the Mac answers from that same memory with its sources, the overlay says it answered offline, and the question waits to go to Claude once you land.

## What the conversations concluded

The short form, as it applies to Freebyrd:

1. **Your system holds the records, the permissions and the clock. Models are workers it calls.** A worker can be replaced without losing anything of yours.
2. **Weights are a cache, the ledger is the asset, the test set is the contract.** A model earns a job by passing tests built from your own past decisions.
3. **The flight is the test.** With no network, a personal system must still find things in its records, answer from them with sources, record what happens, refuse what it should, and queue the rest.
4. **Anything compiled from your data must pay for itself, and must go when its source goes.** Deleting a source should reach everything built from it.
5. **What you saved on purpose is a stronger signal than what passed in front of you.** "If I bookmark something, that means it's high intent" (your words, quoted in `twitter-suggestions-rework.md`).

What did not survive: training a model on your bookmarks (it learns a story about you, not you), letting a model act alone because it often agreed with you, and sharing learning across people.

## What your memory shows

Counts only, 2026-10-13 to 2026-10-17:

- **Moments:** 178 moments, 72 sent to Claude, 30 Idea cards.
- **Ideas on those cards:** 46 shown, 10 used (Copy or Open), 10 Not here, 2 Not useful.
  - From X bookmarks: 6 of 15 used.
  - From earlier Claude Code sessions: 3 of 22 used, and 7 of the 10 Not heres.
  - From pages: 1 of 3 used.
  - From skills: 0 of 6 used.
  
  These are small numbers, but the things you chose to save were used most.
- **Recaps:** 6 written, 4 opened, 3 dismissed.
- **Night shift:** 12 jobs. 4 handed to Claude Code, 1 done, 3 picked and waiting, 3 skipped, 1 removed.
- **X before it was removed** (README): 4,614 feed posts read and 485 bookmarks given Sonnet notes. One Jev trial burned 2.3 million tokens in a morning, before a bookmark was useful.
- **Jev backtests** (README): chats 8 of 11 right, projects 75 of 99.
- **Disk:** 56 GB free today, up from 4 to 5 GB on Oct 12. That matters for change 1.

## The seven changes

### 1. Answer offline (the flight test)

**Later (your call, 2026-10-17).** This one waits; the rule against local models stays as it is until you come back to it.

**This change needs a decision from you first.** Freebyrd's rules say all AI work goes through the `claude` CLI, with no local models (your decision). This change asks for one narrow exception: a small model on the Mac, used only to answer a notch question when Claude cannot be reached.

What it would do:

- answer from what a question already gathers: the screen text, the part in view and the `<memory>` element, cut to fit a small model
- show its sources the way answers do now, with a quiet line saying it answered offline (it still never names the model, per your rule)
- keep the question so Claude can answer it once the network is back

What it would never do:

- run moments, Jev, drafts, recaps, night shift or anything else in the background
- send anything off the Mac

**How we would know:** in airplane mode, ask 20 questions you have already asked Claude and compare the answers side by side. Keep it if you judge at least 15 useful. That is a proposed bar, not a measured one.

### 2. Models become replaceable workers

**Today:** `claude-cli` is the only path, and `models.rs` picks Haiku 5.5 or Sonnet 5.5 from the question's words.

**The change:** Freebyrd keeps a short list of workers: Claude through the CLI, and the local model from change 1. Later, if you want, Codex with GPT-6.1-Sol, which this Mac already has. Each task says what it needs (web search, a picture, file tools), and only workers that can do it safely are eligible. Among those, the test set (change 3) decides, not a hard-coded rule.

**What stays the same for every worker:** redaction before sending, `claude_cli::data_element` for anything untrusted, no memory kept between calls, and an entry in "What leaves the Mac" before a new worker gets its first byte.

### 3. Your decisions become the test set

**Today:** Freebyrd already keeps:

- verdicts in `idea_verdict` (shown, used, Not here, Not useful)
- cards seen and dismissed
- promises you undid
- chats settled as work or personal
- project groups
- night shift's Keep and Discard

The Jev backtests already replay some of these.

**The change:** freeze those settled cases into a test set for each use: ideas, chats, projects, promises and cards. Hold back the newest fifth of them as a future test that nothing is tuned on. Before any model, prompt or ranker change goes live, it runs against the set, the way a Jev shadow needs your read of its report today.

**Two cautions** from the conversations:

- A set built from past choices rewards repeating them, so it is refreshed as new verdicts come in and never treated as your taste forever.
- Choosing among many candidates against the same cases slowly tunes to them, which is why the held-back fifth exists.

### 4. Background work pays its way

**Today:** jobs pause after five unread results, and Jev has a daily token budget. The X suggestions were removed only after their cost ran far ahead of their use.

**The change:** every step that spends tokens without your press records what it cost and whether you used what it made. That covers notes, Jev shadows, project tags, recaps, drafts and night picks. Use means a card opened, a Copy, a Keep, or a recap read. When a step's cost per use passes a line you set, it pauses itself and leaves a card saying why, with Resume. Littlebird pauses journals that go unread (`littlebird/FINDINGS.md` §5.1) and routines that go unopened (§5.3). This would do the same, counting cost as well as reads.

### 5. What you save counts as intent

**Today:** X bookmarks are gone, saved pages are page snapshots, and browser history keeps how long each page was open. The X rework scope proposes bringing bookmarks back only when you start making something.

**The change:** one record of what you saved on purpose, across sources:

- X bookmarks, as the rework captures them
- browser bookmarks
- pages you kept open for a long time

Each save keeps what it is, where it came from and when. Why you saved it stays blank until a moment where it gets used fills it in. Moments look at saves first, using the rework's "making" gate.

This folds the rework's first version in rather than replacing that doc. Reading browser bookmarks is a new source and needs your yes.

### 6. Moments and night picks are logged as experiments

**Today:** a moment keeps its picks and its top score, and `idea_verdict` keeps what happened. The full ranked list and the reason the shown ideas won are lost.

**The change:**

- **Moments:** keep every idea ranked for a moment, with its score, its place in the order, and whether it reached the shortlist or the card.
- **Night shift:** when it picks a job, it records how likely it thinks you are to keep the result. Keep or Discard settles it.

That makes two things possible:

- judging a new ranker (Jev, shared words, the local model) against what really happened, without showing you its cards first
- seeing whether Freebyrd's confidence means anything

Optionally, with your yes, one card in ten shows the runner-up, so the log covers more than the top pick.

**Honest scale:** 30 cards in five days is about 180 a month at most. That is enough to choose between rankers. It is not enough to let anything act on its own, and this change never does.

### 7. Deleting and correcting reach everything derived

**Today:** `todo.source_id` records where a to-do came from, but most derived things do not. Removing X left behind:

- 4,614 rows in `x_post`
- 459 in `x_idea`
- 70 in `x_fit`
- the thumbnails in `x_open`
- the X logs under `jev/`

The TypeSafe key that reached a prompt was scrubbed by hand.

**The change has two parts:**

- **Lineage.** Each derived thing (a recap, a note, a card, a Jev log line, a summary) records the rows it came from. Then `freebyrd-memory forget` can list everything that depends on a source and remove it after you confirm.
- **Receipts.** Each call to Claude, TypeSafe or a future worker stores a receipt: which sections went, how many characters each, which memory rows were included, and the cost, never the text itself. "What did Anthropic see about Ben this week?" then has an answer, and "What leaves the Mac" becomes a log you can check rather than a promise.

## Not in this scope

- **A personal LoRA or any fine-tuning.** Recall (BM25, titles weighted five times) works. A trained adapter is only worth it if it beats recall on the held-back test set, counting its cost. The one later candidate is a small classifier on the Mac trained on your settled chats. It would replace the `chats` shadow, which today sends a chat's last 8 messages to TypeSafe.
- **Anything acting alone because it agreed with you often.** Copy, Keep and Add stay your presses.
- **Learning shared across people, or anything leaving this Mac beyond what the README lists.**
- **Widening what moments may read.**

## Privacy limits

- **The local model** reads only what a notch question already gathers, sends nothing, and its answers are shown as text like Claude's.
- **Receipts** hold section names, counts, row IDs, hashes and costs, never the text that was sent.
- **The test set** is built from verdicts and titles already in `memory.db`, and stays under `Application Support/Freebyrd`. Running it against Claude or Jev sends those cases, as `jev backtest` does today, and only when you run it.
- **A new worker** (Codex, which sends to OpenAI) is a new destination. It needs your yes and its own "What leaves the Mac" entry first.
- **Showing a runner-up idea** (change 6) only happens with your yes, at most one card in ten.

## A suggested order

1. **Change 3, the test set, and the receipts half of change 7.** Both are plain code and send nothing new, and everything after is judged with them.
2. **Changes 4 and 6**, the cost and experiment logs.
3. **Change 5**, together with the X rework's first version.
4. **The lineage half of change 7**, before any large deletion, including the old X tables if you choose to delete them.

Later, when you come back to it: **change 1**, offline answers, judged on the test set and the 20-question flight test, then **change 2**, workers, once there are two to choose between.

## Decisions for you

Decisions 1 to 3 wait for change 1, which is for later.

1. **A local model:** allow one for offline answers only (recommended), or keep "no local models".
2. **How it runs:** llama.cpp's `llama-server` as a separate process on 127.0.0.1 (recommended: Metal, no new Rust machine-learning crates, easy to fence with `night::sandbox`'s profile), or in-process Rust (mistral.rs or candle).
3. **Model size:** a 3 to 4 billion parameter instruct model at 4-bit, about 2 to 3 GB (recommended; Gemma 3 4B and Qwen 3 4B to compare on the flight test), or larger.
4. **Codex as a second remote worker:** not now (recommended), or yes.
5. **Runner-up cards in moments:** log only at first (recommended), or one card in ten.
6. **The payback line for change 4:** pause after 5 runs with nothing used, as jobs do (recommended), or a cost per use you name.
7. **Browser bookmarks as a source for change 5:** yes, read-only (recommended), or X bookmarks only.

---

## Technical detail

### 1. Offline answers (`claude.rs`, `models.rs`, a new `local` worker)

- **When:** the CLI call fails to reach Anthropic, or macOS reports no network path before the call. A failed call already returns an error from `claude_cli::stream`; the fallback starts there.
- **Runtime:** `llama-server` started by Freebyrd on demand, on 127.0.0.1 and a free port, under `night::sandbox`'s profile with outbound network denied. The model file lives under `~/Library/Application Support/Freebyrd/models/`, never under Documents. It stops after 10 minutes idle.
- **Input:**
  - the same `<screen>`, `<in_view>` and `<memory>` elements built with `claude_cli::data_element`
  - memory cut to about 8,000 characters instead of 20,000 (a proposed first budget; measure)
  - no picture in the first version
  - no web search
- **Output:** streamed into the overlay as text, with a line saying it answered offline. The question is saved in a new `pending_question` table, and the overlay offers Ask Claude once a connection returns.
- **Tests:**
  - unit tests for when the fallback runs
  - `freebyrd --ask --local` on a copy of the memory
  - the 20-question flight test in airplane mode
  - a check that `llama-server` cannot reach the network (a request to example.com refused, as night shift's sandbox was checked)

### 2. Workers (`crates/claude-cli` and a new worker list)

- A small interface with `stream` and `ask`, implemented by today's `claude_cli` and by `local`.
- `models.rs` becomes the choice among eligible workers per task, reading the test set's last results; until change 3 exists, it keeps today's rule.
- Eligibility by need:
  - web search, connector reads (`fetch`) and file tools (`agent`) stay Claude-only
  - the local worker gets plain questions and nothing else
- Every worker takes untrusted text only as data elements over stdin or the request body, never in its instructions.

### 3. The test set (`memory::evals`, a new module)

- **Table:** `eval_case (use, case_ref, expected, settled_at, split)`.
- **Built by plain code from:**
  - `idea_verdict` (used against Not here and Not useful, per moment)
  - chats settled as work or personal
  - `jev/project-groups.json` with sessions already tagged
  - promises kept against undone
  - cards dismissed unseen
  - night jobs kept against discarded
- **Split:** by date, with the newest 20 percent as `holdout`.
- **CLI:** `freebyrd-memory evals build | run USE --worker W | report`. A run against Claude or Jev sends the cases it scores, so it is by hand only and listed under "What leaves the Mac".
- **A first check:** rerun today's setup and reproduce the README's numbers (chats 8 of 11, projects 75 of 99) before trusting the set.

### 4. Payback (`memory::payback`, a new module)

- **Table:** `spend (step, at, tokens, dollars, output_ref)`, plus a use mark set when you open, copy, keep or read what `output_ref` points to.
- **Rule per step:** the default is 5 runs with nothing used, then pause, which generalises the jobs' five-unread rule. The step leaves a card (kind `paused`, left side, a solid pause icon in `KINDS`) with Resume.
- **Covered steps:** recaps, drafts, Jev shadows, project tags, night picks, and notes if the X rework comes back.

### 5. Intent (`memory::intent`, a new module)

- **Table:** `intent (id, source, ref, title, saved_at, open_ms, why, used_at, retired_at)`.
- **Sources:**
  - X bookmarks through the rework's capture
  - Chrome's `Bookmarks` file in each profile, read-only (Arc's pinned tabs live elsewhere; where needs checking)
  - pages whose total open time in History passes a threshold (a first guess: an hour over three or more days)
- **Moments:** for a moment that passes the rework's making gate, gather from `intent` before pages and sessions. When an intent item is used, `why` gets the moment's first words, so the save learns what it was for.
- **Retiring:** an unbookmark, or Not useful, retires an item. Nothing nags about unread saves (the rework doc: no look-backs).

### 6. Logging (`moments::rank`, `night::suggest`)

- **Table:** `moment_rank (moment_id, item, source, score, rank, shortlisted, picked, explored)`, written from `rank()` for every idea it scores.
- **Runner-up cards:** an optional `explore` share (default 0) picks the runner-up for one card in N, and records that it did.
- **Night shift:** `night_job` gains `predicted_keep REAL`, written by the pick and settled by `action`.
- **Report:** `freebyrd-memory moments report` shows, per source and per score band, how many ideas were shown, used, Not here or ignored.

### 7. Lineage and receipts (`store`, `claude-cli`, `jev`)

- **Table:** `derived (artifact, source)`. It is written by recap (days to recap), cards (card to plan, moment or job), drafts, notes and Jev log lines.
- **Forget:** `freebyrd-memory forget --source x --dry` prints the counts it would remove. Without `--dry` it needs `--yes`.
- **Table:** `receipt (at, worker, use, sections, rows, sha256, dollars)`, where:
  - `sections` maps each element name to its characters
  - `rows` lists the memory row IDs included
  - `sha256` is of the whole request

  It is written in `claude_cli` and `jev::ask`. A request whose sections are not in the README's list is refused, so the list and the code cannot drift apart.
- **CLI:** `freebyrd-memory receipts [--day D] [--row ID]`. `--row` answers "which calls included this row", which is how "what did Anthropic see about Ben" gets answered without storing the text.
- **Not in the first version:** deleting from Anthropic's or TypeSafe's side, which receipts can only record.

### What leaves the Mac, for the README

- **Change 1:** nothing. The local model runs with outbound network denied.
- **Change 2:** nothing new until a new worker is approved. Each one gets its own entry first.
- **Change 3:** a test run against Claude or Jev sends the settled cases it scores, by hand only, like `jev backtest`.
- **Changes 4 to 7:** nothing. They are plain code over the memory, and browser bookmarks are read on the Mac.

### Budget

- **Change 1:**
  - 2 to 3 GB of disk for the model
  - about 3 to 4 GB of memory while `llama-server` runs (estimates; measure on the M3 Pro)
  - no token cost
- **Change 3:** a run against Claude costs about what a `jev backtest` costs on the same cases. Small, and only when run.
- **Changes 4 to 7:** no model calls.

### Measuring and tests

- **Change 1:** 15 of 20 flight-test answers judged useful by you, and the network-denied check passes.
- **Change 2:** switching a task to another worker keeps its test-set results within 5 points on every slice with enough cases. That is a proposed bar from the conversations, not a measured one.
- **Change 3:** the set rebuilds the same cases twice in a row, and today's setup reproduces the README's numbers.
- **Change 4:** a week with the rule on and nothing paused that you would have kept.
- **Change 5:** at least 1 card in 3 that carries a save gets used, the X rework's bar.
- **Change 6:** the report shows use by score band, and a second ranker is judged from the log alone.
- **Change 7:**
  - `forget --source x --dry` lists the counts the README gives
  - one day of receipts matches "What leaves the Mac" with nothing extra
- **Unit tests** for each new table and rule, none of which reaches Anthropic, TypeSafe or a model file.
