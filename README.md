# SlopSlide

A desktop slide editor you drive by chatting. Describe a presentation, and an agent writes
the slides. Keep talking to it to restyle, rewrite, split, or add slides while you watch them change.

- **Left:** live thumbnails. Drag to reorder; hover to lock, hide, duplicate, or delete; `+`
  adds a slide. A locked slide (lock icon) can be changed by neither you nor the agent: it
  cannot be edited on the stage, deleted, or saved changed from the code view, and if the
  agent changes it anyway the app puts it back after the turn. Unlock it to change it.
  Sections group slides under a heading in this column (never in the presentation): the
  section button starts one at the selected slide; double-click a heading to rename it, drag
  it to move the boundary, hover to remove it.
- **Middle:** the current slide on a fixed 1920×1080 stage, scaled to fit. Arrow keys navigate.
  The move button below it turns on edit mode: click an element to select it, drag it (or
  use the arrow keys) to move it, double-click (or press Enter) to edit its text, Escape to
  select the parent, Delete (or the trash button in the edit bar) to remove
  it, and ⌘Z / Ctrl+Z to undo. Moved elements keep a
  temporary offset; **Tidy layout** then sends the agent a screenshot so it rebuilds the
  slide's layout around where you put things.
- **Right:** chat with the agent. It knows which slide you're on, and you can attach images
  (paperclip, or drop files anywhere on the window).
- **Styles and layouts:** decks can follow a template, a deck of example layouts (title,
  section, bullets, split, stats, quote, closing) in one style. 22 come with the app
  (Claymorphism, Swiss Design, Synthwave, Wabi-Sabi, …); add your own as folders in
  `~/.slopslides/templates/<name>/deck.html`. Pick a style when creating a deck or with the
  palette button above the slide rail (it puts a restyle request in the chat; an empty deck
  takes it at once); the same menu has **Save deck as template**, which copies the deck there
  with its text replaced by placeholder words. The rail's `+` shows a blank slide and every
  layout of the deck's style as thumbnails and adds the one you click; **Layout** below the
  slide asks the agent to rebuild the current slide on a layout.
- **Present:** full-screen slideshow (arrows/space/click to advance, `Esc` to exit).
- **Export:** saves the deck as **one self-contained HTML file** (attached images embedded)
  that plays in any browser: arrows/space/click to navigate, `F` for full screen, `#slide-id`
  links, print to PDF.

Built with Tauri 2 (Rust) and React. It runs on macOS, Windows, and Linux.

## Requirements

- An installed, signed-in agent CLI: [Claude Code](https://claude.com/claude-code),
  [OpenAI Codex](https://developers.openai.com/codex/cli), or GitHub Copilot. Select its
  model in the chat composer. To use a binary outside your PATH, set
  `SLOPSLIDE_CLAUDE_PATH`, `SLOPSLIDE_CODEX_PATH`, or `SLOPSLIDE_COPILOT_PATH`.
- For development: Node 22+, pnpm, Rust (stable), and the
  [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

## Development

```sh
pnpm install
pnpm app:dev        # desktop app with hot reload
pnpm app:build      # installers for the current OS → src-tauri/target/release/bundle
```

`pnpm dev` alone serves the UI at http://localhost:1420 in a plain browser, with mocked IPC
and read-only access to your deck library (`dev/browserPreview.ts`). This is handy for
UI work. Chat and editing need the desktop app.

Run `./check.sh` for typechecking, frontend tests/build, Rust formatting, clippy, and Rust
tests. Every tool comes from `package.json` or `src-tauri/Cargo.toml`; nothing else needs to
be installed. CI runs this same script on macOS, Linux, and Windows, including approval
routing and cancellation tests against a mock Codex app-server built into the Rust test
binary. The full chat approval flow is tested through events, the store, UI, IPC
responses, and transcript persistence.

The Rust tests also validate every request and approval response we send to Codex against
protocol schemas pinned from Codex **0.147.0** (the tested baseline) in
`fixtures/codex-schemas/`. This catches protocol drift, including changes to MCP approval
shapes, without an account, credentials, or model inference. To check a newer Codex
release, run `scripts/update-codex-schemas.sh <version>` and then `./check.sh`. Live
behavior tests below remain opt-in; schema validation cannot detect every behavioral
change in an external CLI.

The top bar shows whether `deck.html` passes the HTML lint. When it does not, clicking the
status puts fix instructions into the chat composer; the agent then fixes the issues and
re-checks with its `lint_deck` tool.

## Codex permissions

With a Codex model selected, the composer shows a permissions picker:

- **Ask for approval** (default): Codex works inside the deck workspace and asks for extra access.
- **Approve for me**: the same workspace boundary, with Codex's automatic reviewer handling
  eligible approval requests. Automatic reviews can consume additional subscription usage.
- **Full access**: no sandbox or approval prompts. Selecting it requires confirmation.
- **Custom**: use the current Codex configuration, including sandbox, approval policy, and reviewer.

The selection is remembered locally and cannot change during a turn. It is reapplied when
resuming a conversation; it does not modify `~/.codex/config.toml`. Modes unavailable in the
installed CLI or disallowed by managed requirements are omitted. Codex remains responsible
for enforcing the effective policy; unsupported or restricted configurations produce an error.

Requests appear in chat with the command, proposed file changes, network destination, or
requested permissions. Choose **Allow once**, **Deny**, or **Stop**; session approval is
shown when offered by Codex. Additional permission grants are limited to the current turn.
Automatic reviews show their status and rationale. Requests close when their turn ends.
Leaving a deck stops its active Codex turn and saves the interrupted transcript, so an
approval cannot remain waiting in a hidden deck.
MCP tool approval confirmations use the same cards, including session approval when offered
by Codex. Structured input forms, URL authentication, and device verification are not supported
yet and receive an explicit error instead of being automatically approved.

Codex chat uses `codex app-server` with your existing CLI login. ChatGPT sign-in uses your
subscription; API-key sign-in uses API billing. Update an older CLI with `codex update` if
permission discovery fails. Claude and Copilot retain their existing permission behavior.

Checks include an offline mock app-server. An optional live test makes two tiny inference
requests, verifies streaming and resume, and requests no file changes:

```sh
cargo test --manifest-path src-tauri/Cargo.toml real_codex_smoke_test -- --ignored
```

To inspect the installed server's effective permission presets without model inference:

```sh
cargo test --manifest-path src-tauri/Cargo.toml real_codex_permission_modes_test -- --ignored
```

The live MCP regression test approves only `slopslide/lint_deck` on a temporary deck
and makes one model request:

```sh
cargo build --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml real_codex_lint_approval_test -- --ignored
```

## Releases

Push a tag such as `v0.1.0`. `.github/workflows/release.yml` then builds macOS (Apple
Silicon and Intel), Windows (MSI/NSIS), and Linux (AppImage/deb/rpm) installers into a
draft GitHub release. Code signing and notarization secrets are listed in the workflow.

## How it works

```
src/                    React UI (Zustand store, Tailwind)
src-tauri/src/
  deck.rs               deck folders: load/normalize, slide operations, export, snapshots
  html.rs               finds the slide <section>s in deck.html and rewrites them;
                        installs the player runtime (assets/runtime.{css,js})
  templates.rs          built-in and user templates (`~/.slopslides/templates`): listing,
                        new decks in a template's style, saving a deck as a template
  lint.rs               HTML lint for deck.html: well-formed markup plus the deck format
                        rules; shown as the status button in the top bar
  mcp.rs                stdio MCP server (`slopslide --lint-mcp <deck>`) exposing the
                        linter to the agent as its `lint_deck` tool
  agent.rs              manages turns and snapshots; normalizes provider events for the UI
  codex.rs              Codex app-server transport, permissions, and approval responses
  protocol.rs           `slop://` scheme serving deck files to the slide iframes;
                        adds the slide editor (assets/editor.js) for edit mode
  watcher.rs            file watcher → `deck-changed` events, so edits stream into the UI
src-tauri/prompts/      the agent's system prompt and design references
src-tauri/templates/    the built-in templates, one deck.html each (served with the runtime)
```

Each deck is a plain folder under `~/Documents/SlopSlide/<deck>/`:

```
deck.html        the whole presentation: <section class="slide" id="…"> per slide,
                 optional <div class="deck-section" data-title="…"> markers between them
                 that start a section, shared styles, the embedded player runtime, and
                 optionally <meta name="slopslide-template" content="…"> naming its template
assets/          attached images and media (inlined on export)
.slopslide/      chat history, agent session, reference docs, snapshots (app-managed)
```

`deck.html` already plays standalone in a browser. The editor renders individual slides of
it in sandboxed iframes (`deck.html?embed&slide=<id>`), so the thumbnails, stage, and
exported file all use the same player. Before every agent turn and slide deletion, a copy is
saved to `.slopslide/snapshots/` (last 30 kept).

Claude receives file tools (Read/Write/Edit/Glob/Grep plus web search/fetch) and the app's
`lint_deck` tool, with no shell access or other MCP servers. Codex uses its configured tools
and MCP servers, plus `lint_deck`, under the selected permission mode. Copilot uses its
existing deck-scoped permission handler. You can also edit `deck.html` by hand in any
editor; the app picks up the changes.

The design guidance in the agent's prompt draws on
[frontend-slides](https://github.com/zarazhangrui/frontend-slides) (MIT). See
`src-tauri/prompts/THIRD_PARTY_NOTICES.md`.
