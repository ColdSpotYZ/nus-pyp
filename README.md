# NUS Exam Papers

`NUS Exam Papers` is a desktop app built with Tauri, React, and TypeScript to make it easier for NUS students to search and download papers from the NUS Digital Gems Examination Papers Database.

## Why this app exists

The official Digital Gems flow works, but it is not optimized for students who want to quickly:

- search across exam papers using the same metadata filters as the website
- review multiple results in one place
- select several papers at once
- download them without repeating the same browser steps over and over

This app exists to reduce that friction. It keeps the official NUS Digital Gems login and access model, but wraps it in a desktop workflow that is faster and more convenient for students who need to gather papers for revision.

## What the app does

- reuses a Digital Gems session after login when the saved session is still valid
- falls back to a simple sign-in gate when the session is missing or expired
- searches the Examination Papers collection at `https://digitalgems.nus.edu.sg/browse/collection/31`
- supports the same advanced search fields and operators used by the website
- shows normalized result rows with paper title, course, year, and semester
- lets users select individual papers or all loaded results
- opens a paper in the Digital Gems viewer when the title is clicked
- downloads papers asynchronously with a queue and per-item progress
- lets users cancel and retry downloads

## How it works

The app uses a hidden authenticated Digital Gems webview as the source of truth for session cookies. That lets it:

- validate whether a user is already logged in on startup
- reuse the same authenticated session for future launches when possible
- query the Examination Papers collection directly with the same search semantics as the website
- download the real PDF asset behind the Digital Gems viewer instead of saving the viewer page itself

The goal is convenience, not bypassing access controls. Users still authenticate through the official NUS login flow, and the app depends on the same permissions and availability as the Digital Gems site.

## Tech stack

- Tauri 2
- React 19
- TypeScript
- Vite
- Rust

## Installation

### Windows

#### App

1. Download and install the Windows `.msi` build of `NUS Exam Papers`.
2. Launch the app from the Start menu or desktop shortcut.
3. Sign in to NUS Digital Gems when prompted.
4. After login, search and download papers directly in the app.

#### CLI

The packaged Windows MSI includes the `nus-exam-papers` CLI and adds the bundled CLI directory to your user `PATH`.

After installing:

1. Close any terminals that were already open.
2. Open a new PowerShell or Command Prompt window.
3. Run:

```powershell
nus-exam-papers auth status
```

If the desktop app has already saved a valid Digital Gems session, the CLI can reuse it immediately.

#### Codex skill

To teach another Codex/LLM session how to use the CLI:

1. Copy [skills/nus-exam-papers-cli](skills/nus-exam-papers-cli) into your Codex skills directory.
2. On Windows, that is typically:

```powershell
Copy-Item -Recurse `
  ".\skills\nus-exam-papers-cli" `
  "$HOME\.codex\skills\nus-exam-papers-cli"
```

If `CODEX_HOME` is set, install it under:

```powershell
$env:CODEX_HOME\skills\nus-exam-papers-cli
```

### macOS

#### App

1. Download the packaged macOS app build and move `NUS Exam Papers.app` into `Applications`.
2. Open the app.
3. Sign in to NUS Digital Gems when prompted.
4. Search and download papers from the desktop UI.

#### CLI

The packaged macOS app bundles the `nus-exam-papers` CLI inside the app, but macOS does not automatically place it on `PATH`.

To enable terminal usage:

1. Open the app.
2. Use the in-app `Install CLI` action.
3. The app will link the bundled CLI into `~/.local/bin/nus-exam-papers`.
4. If `~/.local/bin` is not already on your shell `PATH`, add it in your shell profile and open a new terminal.

Example for `zsh`:

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

Then verify:

```bash
nus-exam-papers auth status
```

#### Codex skill

Install the reusable Codex skill by copying the folder into your Codex skills directory:

```bash
mkdir -p "$HOME/.codex/skills"
cp -R "./skills/nus-exam-papers-cli" "$HOME/.codex/skills/nus-exam-papers-cli"
```

If `CODEX_HOME` is set, install it under:

```bash
cp -R "./skills/nus-exam-papers-cli" "$CODEX_HOME/skills/nus-exam-papers-cli"
```

## Development

Install dependencies:

```bash
npm install
```

Run the desktop app in development:

```bash
npm run tauri dev
```

Build the frontend bundle:

```bash
npm run build
```

## CLI

The repository now also ships a pipeline-first Rust CLI for search, refinement, and downloads.

Run it from `src-tauri`:

```bash
cargo run --bin nus_exam_papers_cli -- auth status
cargo run --bin nus_exam_papers_cli -- search --field metadata.CourseCode.en --operator contains --value CS2030
cargo run --bin nus_exam_papers_cli -- refine --facet-href "https://digitalgems.nus.edu.sg/browse/collection/31?q=facet,metadata.Semester.en.keyword,equals,1&q=must,metadata.CourseCode.en.keyword,contains,CS2030&limit=10"
cargo run --bin nus_exam_papers_cli -- download --output-dir C:\temp\papers --view-url https://digitalgems.nus.edu.sg/view/123/example
```

CLI notes:

- default output is JSON for easy chaining into other tools or LLM workflows
- the CLI reads the same shared Digital Gems session snapshot used by the desktop app
- if no saved session exists yet, sign in once through the desktop app, run `nus-exam-papers auth login`, or import a cookie header with `auth login --cookie-header`
- packaged app behavior:
  - Windows MSI installs the bundled CLI into the app directory and adds that directory to user `PATH`
  - macOS bundles the CLI inside the app and exposes an in-app `Install CLI` action that links it into `~/.local/bin`

## Codex Skill

This repo also includes a reusable Codex skill for operating the CLI:

- [skills/nus-exam-papers-cli/SKILL.md](skills/nus-exam-papers-cli/SKILL.md)

Install notes for other users are in:

- [skills/README.md](skills/README.md)

## App identity

- Product name: `NUS Exam Papers`
- Bundle identifier: `com.coldspot.nuspyp`
- npm package name: `nus-pyp`
