# Client interface (v1, 2026-11-24; additions are backward compatible)

Lead owns: root `Cargo.toml`/`Cargo.lock`/`rust-toolchain.toml`, `crates/protocol`, `apps/client/src/{main,app,theme,motion}.rs`, `api/`, `state/`, `components/`, `views/mod.rs`.
Feature views keep the shared entry-point signatures. If a module grows, turn it into a folder (`views/tasks/mod.rs`) with the same `pub fn`s.

| Owner | Files | Entry points |
|---|---|---|
| A | `views/projects.rs` | `section(state, window, cx)` (Projects), `tab(target, ..)` (`TabKind::Project`) |
| A | `views/tasks.rs` | `tab(target, ..)` (`TabKind::Task`) |
| A | `views/workflow.rs` | `inbox_section(..)` (interventions), `settings_section(..)` (settings, harnesses, execution targets) |
| A | `views/onboarding.rs` | `section(..)` (Templates) |
| B | `views/stats.rs` | `section(..)` (Home) |
| B | `views/docs.rs` | `tab(target, ..)` (`TabKind::Document`) |
| B | `views/review.rs` | `tab(target, ..)` (`TabKind::Review`, `target.id` = `code_tour` document id) |

```rust
pub fn section(state: AppState, window: &mut Window, cx: &mut App) -> AnyView;
pub fn tab(target: &TabTarget, state: AppState, window: &mut Window, cx: &mut App) -> AnyView;
// Typically: cx.new(|cx| MyView::new(target.clone(), state, window, cx)).into()
```

## Shared types (`crate::state`, `crate::api`, `protocol`)
- `AppState { api: ApiClient, service: Entity<ServiceState>, workspace: WeakEntity<Workspace> }` (Clone)
  - `open(TabTarget, window, cx)`: focuses an existing tab or opens a new one
  - `replace_active(TabTarget, window, cx)`: navigates the active tab in place
  - `show_section(Section, cx)`, `set_title(&TabTarget, title, cx)`, `data_changed(cx)`
  - `track_draft(&target, &entity, cx)` for views implementing `state::Draft { is_dirty(&self, &App); save(&mut self, window, cx) -> Task<anyhow::Result<()>> }`
  - `close(&target, cx)` (after a delete), `report_error(&ApiError, window, cx) -> SharedString`
  - All workspace updates are deferred, so they are safe to call from view constructors.
- `TabTarget { kind: TabKind, id, project_id }` with constructors `::project(id)`, `::task(project_id, id)`, `::document(project_id, id)`, `::review(project_id, doc_id)`.
- `Section::{Home, Inbox, Projects, Templates, Settings}`
- `ApiClient` (all calls are async and unwrap `{data}`): `get::<T>(path)`, `post(path, &body)`, `put`, `patch`, `delete(path)`; `api::query_value(id)` for query strings.
  - `ApiError::{Unavailable, Status{status, body}, Decode}` with `is_conflict()` (409), `is_not_found()`, `is_not_implemented()` (501).
  - Paths follow `packages/contracts` `ENDPOINTS`.
- `ServiceState`: `status()`, `data_version()`.
- `protocol::*` mirrors the contracts. Unknown enum values become `Unknown`; render a fallback. `TourBlock::content()` returns `None` for unknown kinds; keep those blocks. Statistics and run views display recorded values; missing metrics stay "Not available" rather than becoming zero.

## Data loading pattern
```rust
let _observer = cx.observe(&state.service, |this, service, cx| {
    let version = service.read(cx).data_version();
    if version != this.seen_version { this.seen_version = version; this.reload(cx); }
});
fn reload(&mut self, cx: &mut Context<Self>) {
    let api = self.state.api.clone();
    self.load_task = Some(cx.spawn(async move |this, cx| {
        let result = api.get::<Vec<Task>>(&format!("/v1/tasks?projectId={}", query_value(&id))).await;
        this.update(cx, |this, cx| { this.apply(result); cx.notify(); }).ok();
    }));
}
```
- Keep the `Task` handle on the view; replacing it cancels the stale load. Show loading, error and empty states for each request.
- After a successful mutation, call `state.data_changed(cx)` so the board, list, sidebar and other tabs refetch.
- Feature-local state (`ProjectUi`, `TaskUi`) is fine. There is no global cache.

## Refresh semantics
- The client refetches on service ready, ⌘R or Retry, window re-activation and `data_changed`. When metadata advertises events, a background `/v1/events` reader invalidates runtime/CLI changes; the native coalescer batches bursts within 400ms. The stream reconnects with 1–30 second backoff; ready/reconnect invalidations trigger snapshot refresh because events are not replayed. An older service without the stream retains manual/focus refresh.
- **Dirty edits win.** A view keeps its draft and `base_revision` separately from the fetched record. On refetch, a non-dirty view adopts the new record. A dirty view keeps the draft; if the server revision moved, it shows "Changed elsewhere: Reload / Keep mine". Saves send `revision: Some(base_revision)`. On 409, show the same choice; never retry a save silently.
- Tabs stay mounted while open, so drafts survive tab switches. Closing a tab whose tracked draft is dirty opens Save / Discard / Cancel; a failed save keeps the tab. `replace_active` opens a new tab instead of replacing a dirty one. The window refuses to close while any tab is dirty.

## Factory canvas and live drafts

- `apps/client/src/views/workflow/config.rs::ConfigDraft` treats an unchanged mutation as a no-op: no dirty flag or generation bump. Reverting to the saved value becomes clean when no save is in flight. An unchanged service response avoids notifying the controller, so run SSE updates preserve pending validation and canvas state.
- `apps/client/src/views/workflow/factory/mod.rs` updates inspector fields only when values differ and ignores deferred programmatic echoes that already match the model. Real edits retain the existing revision/conflict/save rules; run updates do not manufacture fresh edits.
- `apps/client/src/components/factory_canvas/` owns graph-space geometry and native gestures. Initial Fit tracks settling canvas bounds until user interaction, never during an active gesture; later data invalidations do not reframe the viewport. Explicit Fit is separate.
- Factory panels use **allocated pane width**: at least 1020pt docks the 220pt library and 320pt inspector while preserving a 480pt canvas; at 800–1019pt the library floats; below 800pt both panels float behind toolbar toggles with their own close rows. Closing floating panels exposes the full allocated canvas.

### Accepted factory form and geometry direction

The [native-canvas ADR](../decisions/native-factory-canvas.md#explicit-visual-amendment) links the four actual Linear screens inspected by Opus 5.5: team settings index, auto-close/archive, status groups and team menu. Their `SOURCES.txt` and viewed `v-*` copies are retained at external `manual-evidence/factory-form-research-linear-2026-11-25/`. This accepted visual research satisfies the user's examples “like Orca or Linear”; Orca was not opened and is not a remaining research prerequisite.

- Observed in Linear: aligned rows, restrained surfaces, clear sections/indexing and dependent settings. The auto-close reference has outlined selects inside an outlined group; it does not establish a no-inner-borders or one-border-level rule.
- Chosen for Karabiner under the user's override: flatten redundant decorative wrappers, using spacing/separators for grouping while each input owns its normal border. Align responsive label/control rows and stack them under narrow allocation. Keep target status chips and long labels readable rather than shrinking them into clipped boxes.
- Organize the inspector into meaningful sections with disclosure for secondary detail. Use a bounded return-target multiselect with a compact selection summary, stable stage IDs and save/reopen persistence. Keep its menu and other form menus inside the visible viewport.
- The explicit user direction overrides old vertical-reference geometry and visible branding: left-to-right graph defaults, left-boundary inputs, right-boundary outputs, consistent edge/hit geometry and no visible React Flow label. Keep the MIT notice and adapted-source attribution in source/distribution.

These implemented choices combine the observed Linear patterns with explicit user overrides; they are not a literal copy of Linear's border treatment. Final Opus review and the passing horizontal-connection run close native acceptance with the exact mixed evidence in the [capability matrix](factory-capabilities.md). At900 width the floating inspector intentionally covers the underlying canvas; closing it reveals the full allocation.

Grid dots use one GPUI path rather than a separate painted primitive per dot. The final clean implementation preserves appearance while avoiding the bounds-tree paint work that starved foreground completion. No temporary phase timing or background-HTTP workaround remains. Actual result rows, saved revisions and visually settled validation—not an OCR label alone—establish the recorded completion behavior.

## Shell behaviour
- Tab layout persists to `PUT /v1/workspace`: serialized saves, one re-read on 409.
- Keys: ⌘T Quick Open, ⌘W close, ⌘⇧T reopen, ⌃Tab/⌃⇧Tab and ⌘⇧]/⌘⇧[ cycle, ⌘1–8, ⌘9 last, ⌘R refresh, ⌘\ sidebar, ⌥⌘I inspector, ⌘[ / ⌘] back/forward within a tab. Middle-click closes; drag reorders.
- Sidebar: sections plus a project tree (open-task count; expanded: tasks with status dots, then documents/reviews). Click replaces the active tab (recorded in that tab's back/forward history, not persisted); ⌘-click opens a new one.
- Inspector (read-only, right panel): context, environment, linked documents (Open in tab), runs and activity for the active tab's record. Absent values read "Not set".
- Tab persistence keeps each stored tab's id and unknown fields; load/save failures show a banner with Retry.
- Theme: `/v1/settings.theme` (system/light/dark) applies on load and refresh; `crate::theme::tokens(cx)` gives canvas/surface/tray/border/text/status colours.
- Service URL: `KARABINER_SERVICE_URL`, default `http://127.0.0.1:43173`.
- Shared UI (`crate::components`): `Tone` + `status_dot`, `chip`, `count_badge`, `card`, `section_label`, `list_row`, `empty_state`, `error_state(msg, retry)`, `loading_state`, `toast_warning/error`, `feature_icon(name, size)`.
- Assets: `apps/client/assets/**` is embedded at build time and served by key (Kubai: `kubai/svg-soft/<name>.svg` via `img()`); every Lucide `IconName` is available.
- Motion: use `crate::motion::{fade, standard}` with `gpui_kit::base::motion`. The system Reduce Motion setting is honoured by Kit.

## Visual verification notes

- Measure from the **raw capture pixels** divided by the native-bounds ratio (Retina is 2 px per pt). Never use coordinates from a tool-rendered, downscaled preview.
- Park the pointer on empty content before judging a normal (non-hover) state.
- Before drawing conclusions after an action, wait until the workspace API revision has advanced and the activation is visible in the window.
- When a service has no records (for example `GET /v1/runs` returns `[]`), verify the visuals through the shell owner's visual-states example. It renders labelled in-memory fixtures with the production components (`TraceView`, `run_step`). Never write fixtures to the service or present them as live data.
- Check responsive families at both 1280 and the 720 minimum (sidebar 240 and 360, inspector overlaid and docked). See "Responsive layout" in `visual-contract.md`.
