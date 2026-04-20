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

## App identity

- Product name: `NUS Exam Papers`
- Bundle identifier: `com.coldspot.nuspyp`
- npm package name: `nus-pyp`
