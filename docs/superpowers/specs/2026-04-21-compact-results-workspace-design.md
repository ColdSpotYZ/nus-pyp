# Compact Results Workspace Design

## Summary

Redesign the exam papers app into a compact operator workspace that keeps the interface within the viewport on desktop, reduces oversized copy and vertical stacking, and makes the results region the primary scroll container. The selected direction is a command-bar-and-tabs model: a slim top app bar, a compact search command bar, and a dominant results view, with refine and downloads moved behind compact secondary views instead of large always-open sections.

The redesign is intentionally visual and structural, not a feature rewrite. Search, drill-down, pagination, theme switching, auth status, selection, and downloads remain available, but the page should feel quieter, denser, and easier to use in a sustained workflow.

## Goals

- Keep the main desktop workspace within the viewport without requiring full-page scrolling during normal use.
- Make the results pane the main focus and the only long-scroll region when the result set is large.
- Reduce verbosity in headers, labels, helper copy, and surface count.
- Preserve the current functional flow: auth, search, refine, page through results, select, and download.
- Retain dark and light theme support with the existing semantic token approach.

## Non-Goals

- No backend feature changes.
- No new UI library, router, or state library.
- No redesign of the underlying search model or drill-down logic.
- No mobile-only experience rewrite; mobile should remain usable, but desktop operator efficiency is the priority.

## Chosen Direction

The app should move to a **top toolbar + results-first workspace**:

1. **Top app bar**
   - Compact identity line with app title.
   - Session/auth status.
   - Theme toggle.
   - Minimal supporting text.

2. **Command bar**
   - Compact summary of the active search criteria.
   - Primary actions: search, edit query, clear.
   - Secondary actions: refine, select, download.
   - Pagination status and result count can appear here or immediately above the results pane.

3. **Main content shell**
   - Default view is `Results`.
   - `Refine` and `Downloads` are secondary panels, tabs, drawers, or collapsible utility views rather than full-height stacked panels.
   - Results occupy the largest available area.

4. **Scrollable results pane**
   - Internal scroll region with sticky or semi-sticky header controls.
   - Result rows remain readable and dense.
   - Pagination remains visible near the results region, not detached far below.

## Information Architecture

### App Bar

The current masthead is too tall and text-heavy. Replace it with a shallow top bar that communicates:

- product name: `Exam Papers Workspace`
- session state
- theme control
- one-line status or error area

The overview chips can be reduced or converted into lightweight inline status metrics such as:

- results loaded / total
- active downloads
- current drill-down depth

These should read as utility metadata, not hero content.

### Search Command Bar

The current advanced search panel is large and forces the results far below the fold. It should become compact by default.

Recommended behavior:

- Show a condensed criteria summary when the search builder is not being edited.
- Include an `Edit search` control that expands the full criteria editor inline or in a drawer.
- Keep `Run search` and `Clear` immediately accessible.
- Allow `Add criterion` only inside the expanded editor state.

This preserves power-user searching while avoiding a permanently tall form.

### Results Workspace

The results area becomes the center of the app:

- sticky results summary
- current page indicator
- back to previous results
- clear drill-down
- page-level selection action
- download selected

The results table should live inside a bounded container with its own scroll behavior. The surrounding page shell should stay stable on desktop.

### Secondary Views

Refinement and downloads should stop occupying persistent vertical space below the summary.

Recommended structure:

- `Results` view: default and always prioritized
- `Refine` view: compact overlay/panel/tab with facet groups
- `Downloads` view: compact panel/tab for queue monitoring

Implementation can use tabs or segmented controls rather than modal navigation. The key requirement is that these views do not push the results pane offscreen by default.

## Layout Behavior

### Desktop

- App shell should target `100vh` with controlled internal regions.
- Outer page scroll should be minimized or eliminated once authenticated.
- Results pane should be the primary scroll container.
- Search editor, refinement panel, and download monitor should open within bounded regions.

Suggested structure:

- `main.app-shell`
- compact header row
- compact command bar
- workspace frame with fixed available height
- results panel using `min-height: 0` and internal overflow

### Tablet

- Preserve the compact hierarchy.
- Allow some sections to stack earlier.
- Results should still be prioritized.
- Utility panels may become drawers or expandable sections.

### Mobile

- Relax the fixed-height model.
- Stacked layout is acceptable.
- Avoid giant headers and large dead space.
- Keep results near the top after search.

## Visual Design Direction

The new design should feel like a calm internal productivity tool, not a marketing page.

### Typography

- Reduce headline sizes substantially.
- Use one restrained title size for the app bar and smaller section titles elsewhere.
- Remove large serif hero treatment from the main workspace shell.
- Keep hierarchy clear through weight and spacing instead of dramatic scale.

### Spacing

- Tighten panel padding.
- Reduce vertical gaps between major blocks.
- Prefer 8px/12px/16px rhythm over large 24px/32px stacks.

### Surfaces

- Use fewer nested panels.
- Flatten the UI where possible.
- Preserve theme token semantics, but reduce the “showcase card” feel.
- Results and tools should feel integrated into one workspace.

### Copy

- Replace verbose helper paragraphs with short, utilitarian language.
- Keep empty states brief.
- Remove repeated explanatory text where the control naming is already clear.

## Interaction Details

### Search Editing

- Collapsed state shows current rule summary or “No search rules yet”.
- Expanded state shows the full criteria editor.
- Closing the editor should not discard input unless `Clear` is used.

### Refinement

- Refinement opens from a command-bar action or tab.
- Facet groups are shown in a compact multi-column or accordion layout.
- Clicking a facet still runs the linked Digital Gems drill-down URL behavior.
- After applying a facet, return focus to results view.

### Downloads

- Downloads open in a compact side panel, tab, or utility panel.
- Queue visibility remains available without permanently consuming a large section of the page.
- Active download count remains visible in the app bar or command bar.

### Pagination

- Keep compact numbered pagination.
- Place it directly under or above the results region.
- Preserve page navigation state and ensure only the results pane updates.

### Selection

- Selection should remain page-scoped as currently implemented.
- Selection controls should appear near the results summary rather than in a distant action area.

## Implementation Notes

### React Structure

The current `App.tsx` layout can be reorganized without changing the existing data flow:

- extract a compact app bar section
- convert search panel into collapsed/expanded command bar
- introduce view state for `results | refine | downloads`
- wrap the results region in a bounded scroll container

No backend contract changes are required for this redesign.

### CSS Strategy

Continue using plain CSS with the existing semantic tokens. Key structural changes should include:

- full-height authenticated shell
- reduced masthead height
- bounded workspace panels using `min-height: 0`
- `overflow: hidden` on outer workspace containers
- `overflow: auto` on results container
- smaller type sizes and tighter paddings

### Accessibility

- Preserve keyboard access for theme toggle, pagination, refinement actions, and selection.
- Ensure tabs or segmented controls have clear labels and active states.
- Maintain contrast and focus visibility in both themes.

## Risks and Mitigations

### Risk: Full-height layout causes clipping

Mitigation:

- use `min-height: 0` carefully on grid/flex children
- verify results pane overflow on desktop
- allow mobile to fall back to document scroll

### Risk: Search builder becomes harder to discover

Mitigation:

- keep `Edit search` prominent in the command bar
- show current criteria summary clearly
- preserve a strong primary search action

### Risk: Secondary views hide important information

Mitigation:

- keep visible counts for downloads and available refinements
- return to results automatically after drill-down actions
- keep queue access one click away

## Validation Plan

### Functional

- search still runs from compact command bar
- expanded search builder supports all current criterion types
- drill-down history still works
- pagination still works from the compact results shell
- downloads still queue, progress, retry, and cancel correctly

### Layout

- desktop authenticated view stays within viewport at common laptop sizes
- results pane alone scrolls when the results list is long
- refine and downloads no longer force large page scrolling
- long titles and error messages still wrap safely

### Theme

- both light and dark themes remain readable
- compact controls preserve distinct hover, focus, disabled, and active states

### Responsive

- tablet remains balanced
- mobile remains usable with reduced vertical waste

## Open Decision Already Resolved

The chosen direction is **Option B: Command Bar + Tabs**. Implementation should follow that model unless a technical constraint emerges during planning.
