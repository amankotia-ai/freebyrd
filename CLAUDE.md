# Reverse Engineering workspace

## Folders

- `rea/`: clone of https://github.com/morluto/rea, the REA reverse-engineering toolkit. Reference only. Searches over it can stall because this folder syncs to iCloud.
- `littlebird/FINDINGS.md`: static analysis of Littlebird 0.80.3, an always-on screen-context assistant (how it reads apps, where data goes, its AI sandbox and MCP). Findings are labeled observed, inferred or unknown.
- `context-app/`: Freebyrd, our own Rust context assistant. It has its own CLAUDE.md.

## This Mac

- This folder is in iCloud-synced Documents. Keep build output and large generated files out of it.
- Disk is nearly full (4 to 5 GB free on 2026-10-12; down to 0.3 GB during a Freebyrd test build on 2026-10-13, 1.5 GB after clearing stale caches). Check `df -h ~` before big builds or downloads. Freebyrd's build cache (`~/Library/Caches/Freebyrd/target`, about 11 GB) is the first place to free space: incremental folders under `debug/incremental` untouched for hours are safe to delete; they only speed up rebuilds.
- Rust lives in `~/.cargo/bin` but is not on the shell PATH (`~/.bash_profile` is root-owned). Prefix commands with `export PATH="$HOME/.cargo/bin:$PATH"`.
- Homebrew Node 20 is first on PATH. REA needs Node 22.19+ or 24.11+: `export PATH="$HOME/.nvm/versions/node/v24.18.0/bin:$PATH"`. `nvm use` fails because `~/.npmrc` sets a prefix.
- REA (rea-agents 6.4.0) is registered as an MCP server for Claude Code in `~/.claude.json`, with an env PATH override that pins Node 24.
- Swift builds once failed with "redefinition of module 'SwiftBridging'" from a stale `/Library/Developer/CommandLineTools/usr/include/swift/module.modulemap`, now renamed `module.modulemap.stale-2023`. Check that file first if the error returns. Fixes there need the user's password.
- The Claude desktop app has Accessibility and Input Monitoring access, so tools launched from Claude sessions can read other apps' windows and see keyboard events.

## Rules

- Before saying a Claude model does not exist, check https://platform.claude.com/docs/en/about-claude/models/overview.md. Bundled model lists go stale; Claude Haiku 5.5 (`claude-haiku-5-5`) exists.
- Analysis of third-party apps is for understanding and interoperability. Do not copy their code, prompts or assets into Freebyrd.
