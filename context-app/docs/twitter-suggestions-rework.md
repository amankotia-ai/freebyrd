# X (Twitter) suggestions: scope for a rework

Status: a scope only. Nothing here is built. Written on 2026-10-16, right after the old feature was removed (README "X suggestions: removed"). Build none of it without your go.

## The short version

You use X to find what is new in AI and design. When you bookmark something, you mean "this matters". The rework should bring a bookmark back only when you start making something it would change, with something you can paste right away. It should never bring one back while you are just browsing.

**An example.** One of your bookmarks is a stacked card swap transition. Say you start a Claude Code session with "make the notch cards stack and swap when a new one arrives". Within a few seconds the notch shows a frame of that post, one line on why it fits, and a prompt to paste into the same session: what moves, in what order, roughly how fast, and how to build it. Nothing comes up while you look around a portfolio showcase site, which is what happened on Oct 15: a portfolio bookmark came up there and you pressed Not here.

**What worked before.** On Oct 13, in your ChatGPT chat about the Freebyrd logo, bookmark cards were used 2 times out of 3: you opened one post and copied one prompt. The copied prompt went off brief, because it saw only your last message, and since then chats send your earlier messages too. That is the pattern to keep: you are making something, you have a bookmark for it, and you get paste-ready help in the app you are using.

## What you would actually want

From what you asked for (Oct 12 and 13) and how you work:

1. **Bookmarks only, not the feed.** "On my feed, it doesn't really matter. But if I bookmark something, that means it's high intent."
2. **At the moment you need it, for the task in front of you.** You asked for bookmarks to be recalled "at the right moment in the right context, given the task I'm undertaking". On Oct 13 you asked twice why no card came up, once right after the notch, when asked, said you had a bookmark for exactly that task.
3. **Something to paste, not something to read.** Prompts for Claude Code or ChatGPT, install commands for a tool, and a picture to use as a reference. You steer design with reference images, so a bookmark's picture is often the most useful part.
4. **Mostly design.** 86% of the 459 bookmark notes are design (394), then AI (30), engineering (15) and image generation (5). About 8 new bookmarks a day, read in short bursts on X.
5. **Help with your tasks, not just your moments.** "Incorporate learnings from my Twitter bookmarks and make suggestions based on them for these tasks... do the heavy lifting of research" (Oct 13). That means a design task on your list, such as a Website Task, should come with the bookmarks that fit it.
6. **No look-backs and no standing jobs.** You close recaps without opening them and you turned down standing skills and jobs. So there should be no "your bookmarks this week" digest and no bookmark job to approve.
7. **Show the post when you ask.** "When I ask it something related to my twitter bookmarks, it should be able to show the tweet artifact in line" (Oct 15).

## When they should come up

**Yes, when you start making something:**

- A new Claude Code session, or a request that turns to new work, whose words name visual or interface work (a component, an animation, a layout, a logo, a landing page) or a tool you might install.
- A new chat in ChatGPT, Claude or Gemini about a brand, a logo, an image, a video or a design.
- When you ask: "anything in my bookmarks for this?", or a quick ask on a localhost page you are building.
- On a design to-do or Notion task (a Website Task, say), on the task itself, before it is due (a later version).

**No:**

- While you are reading other people's sites (portfolios, product pages, docs, searches). That is browsing, and a bookmark there is just more browsing.
- On X itself, in mail, chats with people, meetings, or Freebyrd.
- While you are writing (a case study, an agenda, an email), unless the writing is about the design itself.
- Again in a place where you pressed Not here, or ever again after Not useful.

## Where they come from

- **Your X bookmarks**, captured when you press Bookmark or scroll your bookmarks page. Nothing else from X.
- **What Freebyrd learns about each bookmark**, once: what it is, what its pictures show, how to use it, and for a tool, how to get it.
- **The work in front of you**, as it is read today: the session's first request and title, the files it changes, the localhost page's title, and your own messages in an AI chat (never the replies or anyone else's messages).
- Later: your to-do list and Notion tasks.

## What went wrong with the old feature

Measured from your memory (Oct 13 to 16), counts only:

1. **It worked on day one, then mostly missed.** 11 Idea cards included a bookmark. On Oct 13, 3 of 4 were used (opened or copied). From Oct 14 to 16, 1 of 7 was used, 2 got Not here, and 4 were ignored.
2. **It fired while you browsed.** Page visits were the biggest trigger: 30 on Oct 15 alone, 23 of them sent to Claude, 6 cards. Cards on someone else's site (a portfolio wall, an AI video site) were dismissed or ignored. AI chats about your brand were the best fit (3 cards, 2 used).
3. **It matched the topic, not the decision.** Design bookmarks fit any design-flavoured page well enough to reach the shortlist, so cards offered things on the same broad topic instead of something that would change what you were about to do. Late on Oct 15 you told a Claude Code session that a case study section should be a single screen. The card offered a card-swap transition and a skills repo, and you ignored it.
4. **Bookmarks were mixed in with everything else.** One card blended a bookmark with an old session or a skill, which made each card harder to read at a glance.
5. **It was heavy and mostly invisible.** It read 4,614 posts from your feed, though only bookmarks mattered. It opened every bookmark in a hidden browser tab when you were away, took stills from videos, searched GitHub, and had Sonnet write a note on all 485 bookmarks (about 13 s each), including 353 counted on Oct 15 alone, most likely from one scroll through your bookmarks page. One Jev trial burned 2.3 million tokens in a morning. All of that cost was paid before any bookmark was useful.
6. **The first design missed the moments you cared about.** Until Oct 13 it only weighed bookmarks against a Claude Code project every few hours. It made 2 cards in total and was switched to manual.
7. **Posts in answers were never seen working.** The inline posts you asked for on Oct 15 were built and tested in a browser, but never confirmed in the running notch.

## Privacy limits

- Only bookmarked posts. Not the feed, not likes, not replies from other people, and never direct messages (that exclusion stays).
- No hidden tabs in your logged-in browser. A bookmark is read from what the extension saw when you bookmarked it and from X's public post data, which needs no login and carries nothing of yours.
- Fetches for bookmarks go only to public https hosts and carry nothing from your screen or memory. That was the old rule, and it stays.
- What goes to Claude: the bookmark (public), and the same small view of your work that ideas use today: a session's title, the first 100 characters of a request, file names, a page's title, and your own last messages in an AI chat. Everything is redacted. Never page text, replies or other people's messages.
- Jev may rank bookmarks (you approved it for X ideas on 2026-10-13), but the first version does not need it.
- Cards only show. Copy writes to the clipboard and nothing types into another app. Not here and Not useful always work.
- Everything stays in `memory.db` on the Mac. "What leaves the Mac" in the README gets each new item before it ships.

## Remembering bookmarks well (worked through on 2026-10-16)

Recall matters most: a bookmark is only useful if it comes back when you ask, however you ask. You go looking for a bookmark in four ways, and you said all four matter: how it looks ("that painterly hero"), what it is for ("something for an onboarding screen"), who posted it ("the Bakers Studio one"), and the tool's name ("Anatomy").

**Categories,** by what you would be making when you want it back (a first pass over all 485, sorted by hand):

| Category | Count | For example |
|---|---|---|
| Landing pages and website sections | 95 | Steel.dev, Frontal, Tailark sections, heroes on painted backdrops |
| App and dashboard screens | 73 | A CRM record page, settings pages, data tables, onboarding screens |
| Brand identity and logos | 39 | Karda's blueprint brand, Wavelength by HEX, logo construction grids |
| Launch videos and motion graphics | 38 | Replit's kinetic type video, one-prompt Claude showreels |
| Tools, skills and resources | 37 | The Anatomy skill, Emil Kowalski's UI skills, Recordly |
| 3D, isometric and interactive explainers | 35 | The exploded Raptor engine, a Three.js CD case, isometric SVG figures |
| UI motion and micro-interactions | 29 | The flag coin carousel, a stacked card swap, a spring carousel |
| Posters, illustration, icons and type | 24 | Data-viz posters, stamp posters, the Timeless typeface |
| AI product interfaces | 24 | An input that morphs into UI, agent question cards |
| Portfolios and personal sites | 23 | A Famicom cartridge shelf, a boarding pass site |
| Ways of working with AI | 15 | Rough Figma then polish in code, worst-case UI stress tests |
| Jev and TypeSafe | 13 | The Jev classifier, jev() for Postgres |
| AI image and video generation | 7 | A painting-to-video prompt, a character continuity sheet |
| Business, career and essays | 19 | Left out of recall (your call) |
| Not useful | 14 | Left out of recall (your call) |

**Your decisions:**

- **Landing pages split by section.** 57 of the 95 are whole pages and 24 are heroes alone, so each whole page is tagged with every section its pictures show (hero, features, pricing, footer, testimonials, FAQ, call to action), not filed under one.
- **Repeats merge into one entry,** with every post kept as a source. About 15 things were saved more than once (Anatomy from 7 posts, the flag coin carousel from 3, the one-prompt Claude showreel from 5), so about 485 bookmarks become about 460 entries. Shared words get this wrong both ways (they joined three different landing pages and split Anatomy in two), so Claude judges sameness when it writes the note.
- **Not useful and Business, career and essays stay out of recall** (33 bookmarks). They stay in the memory.

**What each entry needs, so all four ways of asking find it:**

- What it is: a name and one line.
- How it looks: style tags and the picture. The styles that recur are dark mode (51), isometric line art (27), blueprint, dither or pixel (27), painterly or classical art behind clean UI (25), frosted glass (9) and hand-drawn (8).
- What it is for: its category, and for a page, its sections.
- Who posted it: handle and name. The ones that recur are @ayushsoni_io (24), Bakers Studio (14), @sterlynavneet (7), Emil Kowalski and @wheresryan22 (5 each).
- The tool's name and how to get it, when it is a tool.
- Its sources: every post that saved it.

Dates are weak for recall: 353 bookmarks came in on Oct 15 in one go, most likely from one scroll through the bookmarks page, so "saved last week" cannot be trusted for them.

## A small first version

**The goal:** when you start making something in Claude Code or an AI chat and a bookmark would change it, the notch shows that one bookmark with its picture and something to paste. When you ask about bookmarks, the answer shows the posts.

1. **Capture bookmarks only.** The extension records a post when you press Bookmark, and everything on your bookmarks page. It no longer reads every post that scrolls past.
2. **One note per bookmark, as it arrives.** Sonnet looks at the text and up to 3 pictures and writes what it is, what it shows, how to use it, and for a tool, how to get it. That is about 8 a day. The 459 notes already in the memory are reused if you keep them.
3. **Only "making" moments.** A Claude Code request or AI chat start that names design, interface, brand or image work, or a tool. Pages, searches and other people's sites are never moments for bookmarks.
4. **One bookmark per card, its own card.** At most one bookmark, never mixed with sessions or skills. It shows a picture, one line on why, and a prompt to paste into that session or chat. At most 3 a day and one every 30 minutes, quiet after Not here.
5. **Asking.** "What did I bookmark about motion?" gets the matching bookmarks, each shown as the post (author, text, picture), and Open goes to X. On a localhost page, the quick ask "Ideas from my bookmarks" returns.
6. **Measure for a week.** Shown, used (Copy or Open), Not here, ignored. Keep it if at least 1 card in 3 is used, the Oct 13 rate. Otherwise tighten the moments or switch it to ask-only.

**Not in the first version:** feed posts, Jev, hidden tabs, GitHub searches, the old project weighing, the iPad's bookmark shelf, bookmarks in night shift, and bookmarks on to-dos.

## Later, if the first version earns it

- **Bookmarks on tasks.** A design to-do or Website Task gets its 1 to 3 fitting bookmarks, inside its draft or as a line on the task, so the research is done before you start. Nothing new pushes into the notch.
- **Night shift briefs.** For a design task due tomorrow, the night writes a short brief from your bookmarks: references, what to borrow, and a first prompt.
- **Tools.** For a bookmarked repository or skill, the card gives the exact install line, checked against the README.
- **The iPad shelf.** Bookmarks again, sorted by the work they fit.

## Decisions for you

1. **Old X data:** keep the 485 bookmarks, their notes and thumbnails for the first version to reuse (recommended), or delete them. The 4,614 feed posts could go either way. The first version does not use them.
2. **What counts as making:** Claude Code requests and AI chat starts about design, interface, brand, image or tools (recommended). Add Blender? Add localhost pages?
3. **Push or ask only:** a card capped at 3 a day plus asking (recommended), or asking only.
4. **Thread reading:** leave out the hidden-tab reads for threads and long posts (recommended), or bring them back for tool bookmarks only.

---

## Technical detail

### Capture (`extension/content.js`, `capture::browser`, `memory::ingest::pages`)

- A bookmark reader replaces the removed feed reader. It records a post only when its Bookmark button turns pressed (`[data-engagement-action=bookmark] button[aria-pressed=true]`, and the older `removeBookmark` test ID), and every post on `/i/bookmarks`. It also records an unbookmark, so the post can be retired.
- It sends the same post shape as before (id, author, name, text, links with their labels, quoted post, media count) in a `bookmarks` field, sent only on a stored read. The page store keeps a read whose only change is a bookmark press.
- `memory::bookmarks` (a new module, not the old `x`) upserts into the existing `x_post` table, setting `bookmarked_at` and `unbookmarked_at`. A post seen on the bookmarks page for the first time gets `imported = 1` (a new column, an appended migration), so a bulk scroll like the 353 on Oct 15 is told apart from fresh bookmarks and does not flood the note step.
- `fixtures/bookmark-test.html` checks the reader against the 2026 markup, as `x-test.html` did.

### Notes (`memory::bookmarks::note`)

- Fresh bookmarks first, then imported ones, at most 20 a working day. X's public post data (`cdn.syndication.twimg.com/tweet-result`) gives the full text, link addresses and media. Up to 3 pictures: photos at "small", and for a video, its poster frame plus one ffmpeg still from the middle (no 6-still sweep). One 360 px thumbnail is kept.
- Sonnet, tool-less, writes into the existing `x_idea` columns (category, kind, name, what, shows, how, use_when, words). A failure is retried after 6 hours, 3 times.
- Existing `x_idea` rows in state `idea` are reused as they are.
- No browser tab and no GitHub search in the first version. A tool bookmark with no install line says so in its note.

### Moments gate and matching (`memory::moments`)

- Bookmarks return as a source, but only for `Kind::Code` and `Kind::Chat` moments whose words pass a "making" check: interface and visual words (component, animation, motion, layout, hero, logo, brand, landing, icon, typeface, color, illustration, video, image, prompt) or tool words (install, library, package, skill, plugin, repo). A moment that fails the check gathers no bookmarks.
- The category must fit the moment: design and image notes for interface, brand and image moments; tool, technique and AI notes for code moments.
- Bookmarks are weighed apart from the other sources. Shared words shortlist up to 8, Sonnet sees those 8 and the moment, and picks at most one or none. Its prompt says to pick only a bookmark that would change what the person types or builds next, and that the same topic is not enough.
- The bookmark card's `PER_SOURCE` cap is 1, and it uses its own daily cap (3) and gap (30 minutes) inside the Idea card limit of 7.

### The card

- Kind `idea`, ref `moment:ID`, data `{ items: ["x:ID"], url, paste, paste_into, thumb }`. The notch and iPad picture code (`card-thumb`, removed on 2026-10-16) comes back for this kind.
- Buttons: Copy prompt (or Copy command for a tool), Open the post, Not here, Not useful. Verdicts go to `idea_verdict`, which still holds the old `x:` rows.

### Asking (`memory::recall`, `claude.rs`, `ui/app.js`)

- A question about bookmarks ("bookmark", "saved on X", "inspo I saved") gets a "From their X bookmarks" section: the notes that match the question and the window's title, up to 8. Other questions get none, so bookmarks stop showing up in unrelated answers.
- Posts in answers come back as built on Oct 15 (`kept_post`, `postBlock`, `postCard`, the prompt rule for post addresses). Confirm them in the running notch before calling it done.

### What leaves the Mac, for the README

- To X's public post-data service and media hosts: a bookmarked post's ID, to fetch its public data and pictures. Nothing of the person's.
- To Claude (Sonnet): each bookmark once (author, text, quoted post, link labels, up to 3 pictures) for its note. For a making moment that passes the gate: the moment as moments already sends it, and up to 8 bookmark notes.
- Nothing to Jev in the first version.

### Budget

- About 8 to 20 notes a day at about 13 s each, and a bookmark weighing only on making moments: by your Oct 13 to 16 counts, under 10 a day, each one Sonnet call on a short prompt.
- No Jev, no hidden tabs, no feed. Logs say why each making moment was or was not weighed.

### Measuring and tests

- `freebyrd-memory bookmarks report`: per day, bookmarks captured (fresh and imported), notes written, making moments, cards, and each card's outcome. The person's usage rate is the go or no-go.
- Unit tests: bookmark-only capture (a feed post is not kept, a press is, an unbookmark retires it, an import is flagged), notes from stand-in data, the making gate (a portfolio page and a case-study request gather no bookmarks, "make the notch cards stack and swap" does), category fit, one bookmark per card, the daily cap and gap, recall only on bookmark questions, posts in answers.
- By hand: a week on the real memory before calling it done, with the report above.
