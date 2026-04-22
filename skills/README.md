# Skills

This repository includes reusable Codex skills under [skills](.).

## Included Skills

- `nus-exam-papers-cli`
  - teaches another model how to use the installed `nus-exam-papers` command for auth, search, refinement, and downloads

## Install Locally

### Windows

Copy the skill folder into your Codex skills directory:

```powershell
Copy-Item -Recurse `
  "C:\path\to\nus-pyp\skills\nus-exam-papers-cli" `
  "$HOME\.codex\skills\nus-exam-papers-cli"
```

If `CODEX_HOME` is set, copy it into:

```powershell
Copy-Item -Recurse `
  "C:\path\to\nus-pyp\skills\nus-exam-papers-cli" `
  "$env:CODEX_HOME\skills\nus-exam-papers-cli"
```

### macOS

Copy the skill folder into your Codex skills directory:

```bash
mkdir -p "$HOME/.codex/skills"
cp -R "/path/to/nus-pyp/skills/nus-exam-papers-cli" "$HOME/.codex/skills/nus-exam-papers-cli"
```

If `CODEX_HOME` is set, copy it into:

```bash
mkdir -p "$CODEX_HOME/skills"
cp -R "/path/to/nus-pyp/skills/nus-exam-papers-cli" "$CODEX_HOME/skills/nus-exam-papers-cli"
```
