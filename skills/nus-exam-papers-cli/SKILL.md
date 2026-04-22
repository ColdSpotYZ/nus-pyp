---
name: nus-exam-papers-cli
description: Use the local `nus-exam-papers` CLI to authenticate against NUS Digital Gems, search exam papers, inspect JSON facet results, refine result sets, and download PDFs. Trigger this skill when Codex should use the installed exam paper command-line tool instead of the desktop UI, especially for automation, scripting, or LLM-to-CLI workflows.
---

# NUS Exam Papers CLI

## Overview

Use `nus-exam-papers` when the task should be completed through the local CLI rather than through the desktop app UI. Prefer it for machine-readable search results, repeatable auth checks, scripted downloads, and any workflow where another agent or tool needs structured JSON.

Default assumption: use the installed command `nus-exam-papers`. Only fall back to:

```powershell
cargo run --bin nus_exam_papers_cli -- ...
```

when working directly in the repo and the packaged command is unavailable.

## Quick Start

Check whether a reusable session already exists:

```powershell
nus-exam-papers auth status
```

Start an interactive login flow if needed:

```powershell
nus-exam-papers auth login
```

Import cookies non-interactively when automation already has a header:

```powershell
nus-exam-papers auth login --cookie-header "cookie1=value1; cookie2=value2"
```

Read a cookie header from stdin:

```powershell
Get-Content cookies.txt | nus-exam-papers auth login --cookie-header -
```

Read a cookie header from a file:

```powershell
nus-exam-papers auth login --cookie-file C:\path\to\cookies.txt
```

## Core Workflow

### 1. Authenticate first

Always start with `auth status` unless the user already confirmed the session is ready.

Interpretation:
- `ready: true` means search and download commands can proceed
- `ready: false` means run `auth login` interactively or import cookies

Treat `auth status` JSON as the source of truth. Do not infer readiness from whether the desktop app is open.

### 2. Search with explicit criteria

Use repeated `--field`, `--operator`, and `--value` flags in parallel order:

```powershell
nus-exam-papers search `
  --field metadata.CourseCode.en `
  --operator contains `
  --value CS2030
```

Multiple criteria:

```powershell
nus-exam-papers search `
  --field metadata.CourseCode.en `
  --operator contains `
  --value CS2030 `
  --field metadata.Semester.en `
  --operator term `
  --value 1
```

Optional controls:
- `--condition must`
- `--page 2`
- `--limit 10`
- `--search-url "<previous search url>"`
- `--q "<raw q clause>"`

The CLI returns JSON by default. Prefer parsing the JSON directly instead of scraping terminal text.

### 3. Refine using returned facet data

Preferred: take a facet `href` from prior JSON output and pass it back to `refine`:

```powershell
nus-exam-papers refine --facet-href "<facet href from prior search JSON>"
```

Alternative: pass raw `q` clauses explicitly:

```powershell
nus-exam-papers refine `
  --q "facet,metadata.Semester.en.keyword,equals,1" `
  --q "must,metadata.CourseCode.en.keyword,contains,CS2030"
```

Use `refine` instead of rebuilding website query syntax manually whenever the previous response already gives `facets[].values[].href` or `queryClauses`.

### 4. Download by `viewUrl`

Download one or more papers:

```powershell
nus-exam-papers download `
  --output-dir "C:\temp\papers" `
  --view-url "https://digitalgems.nus.edu.sg/view/123/example-paper"
```

Optional hints:
- `--file-name "CS2030-2024-2025.pdf"`
- `--download-url "<direct pdf url>"`

Batch example:

```powershell
nus-exam-papers download `
  --output-dir "C:\temp\papers" `
  --view-url "<paper-1>" `
  --view-url "<paper-2>"
```

## JSON-First Handling

Use the CLI as a structured tool.

Important fields from search/refine output:
- `results`
- `totalResults`
- `facets`
- `searchUrl`
- `rawQueryClauses`
- `page`
- `pageSize`
- `pageCount`
- `sessionReady`
- `hasMore`

Important result fields:
- `id`
- `title`
- `courseCode`
- `courseName`
- `year`
- `semester`
- `viewUrl`
- `downloadable`
- `downloadUrl`

Important facet fields:
- `label`
- `count`
- `href`
- `queryClauses`

When chaining commands, prefer:
1. run `search`
2. read `facets[].values[].href`
3. pass one `href` into `refine`
4. read `results[].viewUrl`
5. pass `viewUrl` values into `download`

## Platform Notes

Windows packaged app:
- the MSI is intended to place the CLI on `PATH`
- open a brand-new terminal after installation before testing the command

macOS packaged app:
- the desktop app can expose an `Install CLI` helper
- that helper links the bundled CLI into `~/.local/bin`

Repo-development fallback:
- if `nus-exam-papers` is unavailable but the repo is present, use:

```powershell
cargo run --bin nus_exam_papers_cli -- auth status
```

## Practical Rules

- Prefer `auth status` before `search` if session state is uncertain.
- Prefer `refine --facet-href` over hand-authoring facet clauses.
- Prefer CLI JSON over app screenshots or UI text for automation decisions.
- Use `--cookie-file` or `--cookie-header -` for non-interactive cookie import workflows.
- Do not promise the desktop app must be running; the CLI uses the shared saved session store.
