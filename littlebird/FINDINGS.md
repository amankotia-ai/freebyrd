# Littlebird 0.80.3: findings ledger

Target: `/Applications/Littlebird.app` (bundle id `com.genos.littlebird`, team `ML63743965`, alpha update channel).
Method: static analysis only. Nothing was launched or executed.
Date: 2026-10-12. Tools: REA 6.4.0 (`analyze_javascript_application`, `inspect_analysis_view`), plus direct reading of the extracted bundle and `strings` on the native helper.

Labels: **Observed** = read directly in shipped code or metadata. **Inferred** = my reading of observed facts. **Unknown** = not established.

## Evidence references

| What | Reference |
|---|---|
| Full `app.asar` analysis (sha256 `3157ccba…61b9`) | REA `ev_bef6e0c333b1f88aee57dc8411b4e162de46cb96780cf4e1dfcf20506fc6be50` (3 GB record, mostly third-party) |
| First-party core analysis (main process, `@littlebird/*`, sandbox runtime, parsers) | REA parent `ev_f68f15c594945b878930ab10e5ebd7aca5a60f6c04baf5a66d08ef91a41b5a93` |
| Main module view (`dist-electron/main/index.js`) | REA `ev_cb92204c846411165758d65f55d3503b42e312876ee43397e23520708dcd89c0` |
| UI source | 911 original `.ts/.tsx` files recovered from shipped source maps (`dist/assets/*.map`) |
| Native helper | `Contents/Resources/ContextKitCore.app/Contents/MacOS/ContextKit-cli` (arm64 Mach-O, 50 MB), `strings` output |
| Shipped package sources (section 5) | `app.asar` `node_modules/@littlebird/{sdk,ui-utils,action-cards,axon,axon-toolkits,integrations,local-db}/src` (TypeScript ships unminified) |
| Recovered UI sources (section 5) | `src/features/{journal,world-model,routines,echonotes,hummingbird,home,feed,inputs-db,summary-timeline,activity-timeline,people-wiki,integration-management,push-notifications}`, `electron/shared/{hummingbird,lb-notification}.types.ts`, `src/backendApis.ts` |
| Messaging parsers (read in place) | `ContextKitCore.app/Contents/Resources/ContextKit_LBJSParser.bundle/{runtime.js,JSParsers/{whatsapp,whatsapp_web,gmail,slack}/*_parser.js}` |
| Main, helper, MCP (section 5) | `dist-electron/main/index.js` (`setContentProtection`, `local-db:execute`, EventKit IPC); helper strings (`EventKitCoordinator`, `PlatformActionsEngine`, `LBNativeParsers`); `Contents/Resources/mcp-servers/apple-mail/index.js` |

REA retained evidence lives only in the MCP session that produced it. Function names below are the stable way back into the code.

## Checklist

| # | Question | Status |
|---|---|---|
| 1 | How does it read other apps, and what does it keep? | Answered (static) |
| 2 | Where does collected context go? | Partially answered |
| 3 | What can the AI sandbox do? | Answered (static) |
| 4 | What is MCP used for? | Answered (static) |
| 5 | Journal: source, schedule, storage, contents | Answered (static; the generator is server side) |
| 6 | World Model: types, build, storage, agent | Partially answered (builder and graph store are server side) |
| 7 | Routines: definition, executor, approval, suggestions | Answered (static) |
| 8 | Hummingbird: memory, proactivity, hide from screen share | Answered (static) |
| 9 | To-dos, commitments, calendar writes | Partially answered (detection is server-side LLM work) |
| 10 | Messaging parser fields (WhatsApp, Gmail, Slack) | Answered for JS parsers; native Swift parsers unknown |

## 1. Reading other apps (ContextKit)

- **Observed:** the main process spawns `ContextKit-cli --daemon` and talks to it with newline JSON over stdio (`startContextKitProcess`, `processEventJson`). Env includes `AUTOSTART_CONTEXT_COLLECTION=True` and the default app/domain exclusion lists.
- **Observed:** ContextKit's Swift sources (names from the binary) include `AccessibilityClient`, `LBAccessibility/ElementDepthFirstSequence`, `WindowExtractor`, `ScreenshotCapture`, `SnapshotJobScheduler`, `TriggerRulesEngine`, `BrowserAppleScript`, `ClipboardPasteHelper`, `InputActivityMonitor`, `ModifierKeyTapMonitor`, `IMessageCaptureCoordinator`, `IMessageStoreService`.
- **Observed:** per-app parsers: about 30 JavaScript parsers run in JavaScriptCore (`JSContext`, `LBJSParser`) plus native parsers for Slack, WhatsApp, Signal, Messenger, Discord, Messages, Notes, Terminal, ChatGPT, Gemini, Gmail, Word and others.
- **Observed:** a remote parser URL, `https://jsparsers.littlebird.ai/latest-jsparsers.zip`. **Inferred:** parser logic can be updated without an app update. Whether it is fetched at runtime is **unknown**.
- **Observed:** permission checks for Screen Recording and Accessibility (IPC `check-screen-recording-permission`, `check-accessibility-permission`). Telemetry fields `parser_fallback_screenshot` / `parser_fallback_traversal`. **Inferred:** it walks the accessibility tree first and falls back to screenshots.
- **Observed:** a CGEventTap for modifier keys. Double-tap toggles the Hummingbird overlay; hold starts voice mode (`modifierTapDetected`, `modifierHoldStarted` handlers).
- **Observed:** iMessage capture reads the Messages store with a cursor DB (`imessage_capture_cursor.db`), dedupes, and publishes "conversation snapshots via context API". There is a consent IPC (`imessage:set-consent`).
- **Observed:** meetings use Deepgram (`wss://api.deepgram.com/v1/listen`), Gemini Live, or a local Parakeet transcriber. Local speech/TTS models download from Hugging Face.
- **Observed, default exclusions** (`ExclusionManager`): login window, Keychain, System Settings, Passwords, 1Password, Bitwarden, LastPass, Dashlane and others (not user-modifiable); Finder and Signal (user-modifiable). Domains: password managers and secret-sharing sites. Domain keywords `password`, `login`. Categories excluded by default: Adult, Banking & Finance. Health, Social, Shopping and Entertainment are listed but **not** excluded by default.
- **Observed:** local SQLite stores for dedup hashes, traversed snapshot history, category data (`remote_categories.sqlite` in userData).

## 2. Where context goes

- **Observed:** on `request-auth-token`, the main process sends ContextKit the user's access token and the backend URL (`handleIncomingTokenRequest` → `setTokenAndUrl`; default `https://app.lilbird.co`). ContextKit uploads through its own `APISender` ("context API").
- **Observed:** a pre-upload `RedactionEngine` with patterns for RSA/EC/OpenSSH/PGP private keys, credit cards (incl. Amex/Diners), JWTs, Google Cloud and Anthropic API keys, Bitcoin WIF keys. Uploads carry `X-Redaction-Found`, `X-Redaction-Match-Count` and similar headers. No patterns for names, addresses or health data were found in the strings.
- **Observed:** upload telemetry fields include `bundle_id`, `app_name`, `window_title`, `window_fingerprint`, `browser_snapshot_text`, `snapshot`, `snapshot_gate`, `ingestion_request_id`.
- **Observed:** a live WebSocket to `wss://ws.lilbird.co`; telemetry to Axiom (`api.axiom.co`) and Sentry.
- **Observed:** an Axiom ingest token is hard-coded in the main process and passed to ContextKit (value redacted here).
- **Unknown:** the exact upload endpoint path and full payload schema. Swift stores short strings inline in code, so `strings` misses them. Needs native disassembly (Ghidra).
- **Unknown:** server-side retention and how uploads are used.

## 3. AI sandbox

- **Observed:** the AI's shell tool runs `/bin/sh` wrapped by `@anthropic-ai/sandbox-runtime` 0.0.46 `SandboxManager.wrapWithSandbox` (`SandboxExecFactory`).
- **Observed:** `bash` and `run` always resolve to `needs-approval`; saved tool policies cannot override that (`resolveToolPolicy`). Other tools auto-approve when marked read-only or named like get/list/search.
- **Observed:** writes are allowed only in per-chat folder grants plus temp dirs. Network goes through a local proxy with an allowlist and a per-request domain approval prompt. `allowLocalBinding: false`.
- **Observed:** default read/write blocklist is only `**/.env`, `**/.env.*`, `**/credentials.json`, `**/*.pem`, `**/*.key`. The sandbox runtime itself adds write-only protections (shell rc files, `.gitconfig`, `.mcp.json`, `.vscode`, `.idea`, `.claude/commands|agents`), no read protections.
- **Inferred:** an approved command could read files like `~/.ssh/id_ed25519` or `~/.aws/credentials`, and its output returns to the model. Per-command approval is the main safeguard.
- **Observed:** on platforms other than macOS/Linux, commands run unsandboxed with a warning.

## 4. MCP

- **Observed:** `@littlebird/mcp` is a client for Littlebird's backend MCP registry: `/mcp/servers`, `/mcp/servers/discover`, OAuth initiate/status, catalog. Adding an API-key server POSTs `api_token` to Littlebird's backend.
- **Inferred:** third-party MCP connections run from Littlebird's cloud, and their credentials are stored there.

## 5. Memory, Journal, World Model, Routines and proactive features

Added 2026-10-13. Static only; method in Reproduce steps 6 to 8.

### 5.0 Memory layout

- **Observed:** journals, Echo Notes, the world model, routine runs, to-dos and action cards are all produced by the backend and synced down. The desktop cache is `littlebird.sqlite` in userData, run in a worker thread (`local-db.worker.js`, IPC `local-db:execute`). Schema (`@littlebird/local-db`, Drizzle): one table per entity (`journals`, `action_cards`, `calendar_events`, `threads`, `messages`, `meetings`, `files`, ...) holding a JSON `data` column with generated virtual columns for id and timestamps, plus a contentless FTS5 index (`search_fts`, porter unicode61, bm25 with title weighted 10x) and a trigram title index.
- **Observed:** no on-device text LLM or text-embedding runtime in the main bundle or helper strings; the helper's embeddings are speech and speaker models only.
- **Observed:** internal debug views expose the server memory: `/debug/user_context/` with tables `input` (per-window text; filters `app`, `ck_job_id`, `embedded`, `isHybridSearch`) and `summary` (time-window summaries with `start`/`end`), `/debug/activity_timeline/` (snapshots with app, kind, title, url, `captured_at`) and a per-input `raw-input-chain` diff. **Inferred:** two tiers, raw window inputs plus rolling summaries, searched by keyword and vector.
- **Observed:** Echo Notes ("Memory") are versioned server documents per section (`user_insights`, `topic_highlights`, `deep_user_model`, `memory_gaps`, `custom_memory_doc`, `work_profile`), capped at 5k to 50k characters and stamped `latest_included_message_timestamp`; only internal users see them. Everyone sees Custom Instructions and Assistant Notes (text plus a server file tree, `assistant_notes/vfs/*`). **Inferred:** sections are rebuilt incrementally from data newer than the stamp.
- **Observed:** a People wiki (`/people-wiki/people`) merges one person's Slack, Gmail and Littlebird identities (aliases, emails, accounts, `lastSeenAt`).

### 5.1 Journal

- **Observed:** an entry (`ApiJournalModel`) holds `date`, markdown `text`, an editable TipTap `document`, `sections` (`title`, `tldr[]`, `abstract`, `original_title`), `viewed_at`, `is_edited`, `restore_count`. Edits go to `PUT journal/{id}`; the original can be restored.
- **Observed:** the client never generates entries. Its empty state says entries come from daily Mac activity, and missing days up to yesterday render as "insufficient context" gaps (`detectGapsInJournals`). **Inferred:** one entry per day, built server side after the day ends from that day's captured context.
- **Observed:** settings live on the user profile (`journal_config`): notification hour 6 to 12 (default 9), `full` or `tldr` notification, custom sections (title, description, points or paragraphs) seeded from `journal/suggested_sections`, an optional weekly progress section, 12 languages, a mobile push toggle.
- **Observed:** generation pauses itself when recent entries go unread (`journal/generation_status`); resuming marks all entries viewed. Entries are cached in the local `journals` table.

### 5.2 World Model

- **Observed:** behind feature flag `WORLD_MODEL`, shown as a tab next to Memory. Data comes only from the backend: `/world_model/user-docs` (documents) and `/world_model/graph-data`.
- **Observed:** the graph is entities `{id, name, type, properties, content}` and relationships `{source, target, semantic, context[], metadata}`; each context item is `original_context` or `update`, with text and timestamp. Entity types and relation semantics are free strings; the legend is built from whatever the server returns.
- **Inferred:** built and updated server side from captured context; each edge keeps its first evidence plus a dated update trail.
- **Observed:** the KG agent runs on the backend (`POST /world_model/kg-agent/chat`, SSE events `token`, `thought`, `tool_call {name, arguments, result}`); the client resends the whole history each turn and keeps the session in localStorage. The main chat agent also has a `world_model` tool that takes a `query`.
- **Unknown:** graph store, extraction prompts, entity merge rules.

### 5.3 Routines

- **Observed:** a routine (`ApiRoutineModel`, backend name `report_templates`) has a title, a prompt, `recurrence_type` (`time_interval`; the library schema also allows `context_trigger`, meaning unknown), an iCal RRULE (minutely to yearly, minimum interval 5 minutes), `kind` (`digest`, `watcher`, `reminder`; a reminder fires once and then sets `completed_at`), `agent_mode` (`lite` to `god`), a project and delivery settings.
- **Observed:** event triggers: a Gmail search query on INBOX or SENT per connected account; meeting started or summarized, filtered by title words, attendee emails or domains; a generic webhook (secret URL, conditions on JSON paths: equals, contains, exists, regex, with negate).
- **Observed:** runs execute on the backend: `POST report_templates/{id}/generate` creates a report and a workflow (cooldown enforced with 429), and Axon's notes name Temporal. A run has status `clear`, `found`, `error` or `running`, a narrative, a step transcript, cost and tokens, artifact files, and can emit action cards (`origin: 'routine'`). Each routine has its own server memory (`report_templates/{id}/memory`: notes plus files).
- **Observed:** delivery: a notification-panel card with a canvas preview, APNs push carrying `report_id`, email, or `delivery_channel` `imessage` or `whatsapp_business`. Routines can auto-pause (`auto_inactivity`); some are seeded at signup or assigned by experiments (`origin`, `is_admin_paused`).
- **Observed:** creation paths: a form; a curated library (`routine_library?v=3`, system and team templates); server-generated suggestions (`routine_suggestions`: title, prompt, RRULE, rationale, integrations; dismissable; deduped by normalized title); a chat that asks the agent to propose and create routines (`created_in_chat_id`); and "Improve" (`can_improve`).
- **Observed:** approval: tools used in a run follow the per-tool permissions in 5.5, and consent is held by the backend (`AWAITING_APPROVAL`, denied on timeout, per Axon's notes). **Inferred:** an unattended run that needs an "ask first" tool waits, then fails closed.

### 5.4 Hummingbird

- **Observed:** a question is a normal backend chat message. Context attached: the front app as an auto-included mention plus a window screenshot, an optional text selection, an optional live meeting transcript, and any mentioned apps, cards or skills.
- **Observed:** no retrieval runs in the overlay. Backend agent tools cover history: `chat_with_user_context` (search queries, optional date range and filters), `chat_with_previous_context`, `chat_with_memory` and `edit_assistant_notes` (find-and-replace edits scored for usefulness), `memory`, `world_model`, `get_screen_context`; Axon's `desktop_windows` toolkit lets the backend list, read or capture live windows. A user setting, `disable_screen_context_search`, turns the context search off. **Inferred:** answers can draw on history, decided server side.
- **Observed:** the overlay itself is not proactive. Proactive surfaces are a separate notifications panel window (meeting detected or upcoming, routine report, basic) with a queue (`queued`/`expedited`, `expiresAt`, sound, actions, route); native notifications whose click can open a thread in Hummingbird; APNs push; Home "Need you" (at most 5 cards, filtered by a 0 to 10 `importance` threshold); Feed tabs For you, In progress, Todos, History.
- **Observed:** hide from screen share is Electron `setContentProtection()`. Hummingbird follows `hideFromScreenShare` (default off), the notifications panel is always protected, and the meeting widget defaults to protected. The panel floats on top, shows on all Spaces and is created click-through.

### 5.5 To-dos, commitments and calendar

- **Observed:** no client-side promise detector. To-dos are `informational` action cards with `kind: 'todo'` (title, body, source app and people, optional CTA prompt). Cards come from chats, routine runs, Meeting Assist (`sourceMeetingId`), messages to Littlebird over iMessage, WhatsApp Business or email (`sourceMessagingChannelType`), or quick add (`POST /action-cards/todos`). A shipped fixture shows a morning-brief routine emitting a to-do that names a person and a deadline. **Inferred:** commitment finding is LLM work inside routines and Meeting Assist.
- **Observed:** a `TodoStatus` enum (`suggestion`, `stale_suggestion`, ...) exists but nothing in the client reads it. Connecting an integration can trigger quick initial card suggestions (`/action-cards/initial-suggestions`).
- **Observed:** action cards are proposals: `draft` until the user confirms (`POST action-cards/{id}/run`) or dismisses (optionally with a reason). Kinds: Gmail and Outlook drafts, Google Calendar event and RSVP, Google Tasks, Todoist, Slack, Linear, GitHub, HubSpot. Google Calendar cards run through Composio (`GOOGLECALENDAR_CREATE_EVENT`).
- **Observed:** auto-review rules (up to 50 plain-language rules, each `auto_run` or `ask_first`; ask first wins a conflict) can run cards without a click. Each tool is `read`, `write` or `delete`, may be `sensitive`, and is `allow`, `ask` or `deny` with a server default; the settings copy says tools run on their own unless set to Ask first.
- **Observed:** Apple Calendar, Reminders and Contacts are on-device toolkits: a backend tool call reaches Axon on the Mac, then ContextKit `invoke-platform-action` (`create-calendar-event`, `create-reminder`, ...) into EventKit (`PlatformActionsEngine`). An event takes title, start, end, notes, location; a reminder takes title, notes, due date.
- **Observed:** `EventKitCoordinator` reads Apple Calendar and Reminders, refreshes on `EKEventStoreChanged`, honors exclusions and uploads events (`/calendar/eventkit/events`, `/delete`, `/purge`). Google and Outlook calendars sync server side (`POST /calendar/sync`). Automatic meeting prep defaults to 12 hours ahead, with email.
- **Observed:** a bundled on-device MCP server, `mcp-servers/apple-mail`, reads Mail's Envelope Index SQLite and uses AppleScript; its compose tool only opens drafts.

### 5.6 Messaging parsers

- **Observed:** JS parsers return `activeConversations[]` of `{app, channel, isGroup?, messages[{sender, content, timestamp, timestring?, messageType?, extraMetadata?}]}`. The user's own messages use sender `[user]`, and `NOT_HANDLED` falls back to generic traversal.
- **Observed:** WhatsApp app: the open chat only; group detection from accessibility phrasing; types text, photo, video, voice, document, sticker, gif, location, poll, contact, event, voice_call; a forwarded flag; English and Portuguese token tables.
- **Observed:** WhatsApp Web: same shape; tiered sender resolution; attachments become bracketed placeholders; quoted previews, reactions and read receipts are skipped; dates come from day separators.
- **Observed:** Gmail web: the open thread only (channel is the subject); sender `{name, email}`, body or collapsed snippet, display time; compose and inline replies are emitted as `[draft]`; the inbox list is declined; no recipients, labels or unread state.
- **Observed:** Slack: main pane and open thread as separate conversations, ISO timestamps, composer text as a `[user]` draft, huddles as `meeting`, canvases as `documents`; reactions and reply counters are skipped.
- **Observed:** none of these parsers extracts unread state or chat lists. Native Swift `WhatsAppParser` and `GmailWebParser` (with a list-view path) also exist. **Unknown:** which one runs, and what the native list view yields.

## Other features seen in recovered UI source

Hummingbird (overlay quick-ask with screen context, optional hide-from-screen-share), Meetings, Journal, Routines, World Model (knowledge-graph view with an agent), Hive (team questions), Cowork (agent chats with working folders), Echo Notes, Arena. Section 5 covers Hummingbird, Journal, Routines, World Model and Echo Notes.

## Reproduce

1. Extract first-party files from `app.asar` (main, preload, `@littlebird/*`, sandbox runtime, parser JS).
2. `analyze_javascript_application` on that directory with `detail: "summary"`.
3. `inspect_analysis_view` item for `dist-electron/main/index.js`.
4. Rebuild UI sources from `dist/assets/*.map` `sourcesContent`.
5. `strings -n 8` on `ContextKit-cli`.
6. Section 5, without REA: parse the asar header with a short Python reader and extract only `dist-electron/`, `node_modules/@littlebird/` (its `src/` ships unminified) and `dist/assets/*.map` (about 105 MB, into a temp folder).
7. Read the JS parsers in place under `ContextKitCore.app/Contents/Resources/ContextKit_LBJSParser.bundle/`.
8. `grep` the helper's `strings` output for `EventKitCoordinator`, `PlatformActionsEngine` and `LBNativeParsers`.
