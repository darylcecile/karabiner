# Visual measurements — projects / tasks / workflow / onboarding

Research only. No source changed. Scope: `apps/client/src/views/{projects,tasks,workflow,onboarding}/**` incl. `tasks/ui.rs`.
The lead's master contract overrides anything here. Old values in source and old notes are **not** approved.

## Calibration (how pixels became points)

| Ref | Pixels | Scale used (px per pt) | How calibrated |
|---|---|---|---|
| `E62FEC84` Lemonade board (dark, keynote frame) | 1182×664 | ≈0.73 | Column title "Intake & Sizing" glyph box ≈9.5px tall, mapped to the 13pt title role. Treat as **ratios**, not absolutes. |
| `AE10217A` board + drawer (light) | 1916×1639 | ≈1.30 | Sidebar nav pitch 41px → 32pt row; nav text ≈17px → 13pt. |
| `630BC713` list + drawer (light) | 834×942 | ≈1.0–1.07 | Drawer property rows 29px pitch; list rows 40px pitch. |
| `B6BD5981` session view (dark) | 1563×1009 | ≈1.0 | Sidebar nav pitch 32px, side-panel row pitch 24px. |
| Native evidence `client-acceptance-2026-11-24/*.png` | 2560×1600 | 2.0 (Retina, 1280pt window) | Known display. |

## Root causes found in source (fix these, don't patch around them)

1. **rem cascade makes everything tiny.** `gpui-component-0.11.0/src/root.rs:731` runs `window.set_rem_size(cx.theme().font_size)`, and `theme.rs:109` sets `font_size = 13`. So every rem helper is scaled by 13/16 = 0.8125:
   - `text_sm()` = 11.4pt (used 55× in my views), `text_xs()` = 9.75pt, `text_lg()` = 14.6pt.
   - Spacing too: `p_3` = 9.75, `gap_2` = 6.5, `px_1p5` = 4.9, `h_8` = 26.
   - Kit controls: `Button::small()` = `h_6` = 19.5pt with 11.4pt text; `xsmall()` = 16.25pt; `Input` Medium = `h_8` 26pt with `text_sm` 11.4pt; Small = 19.5pt (`sizing.rs:245-288`, `button.rs:712-728`).
   - My views also mix in hard-coded 11/12pt (`text_size(px(11.))` 24×, `px(12.)` 45×). Result: four competing sizes near 11–12pt and no clear hierarchy. That is the "tiny, inconsistent type" the user reported.
   - **Fix (owned by the theme/lead):** pick one of these. (a) Keep rem = 16 and set body text separately. (b) Keep rem = 13 and stop using rem helpers in views. Then views use **named px roles only**, from a shared helper module (below). Don't mix the two approaches.
2. **Rounded clipping does not exist in GPUI.** `ContentMask` is only `bounds` (`gpui-pre-0.3.14/src/window.rs:2542`), so `overflow_*` clips to a rectangle and ignores `rounded()`. Anything with a background, or any scrolled content that reaches a rounded container's corner zone, bleeds past the curve. In my views:
   - The board column body (`board.rs` column-body) scrolls with `px_1p5` (4.9pt) and `pb_1p5` inside a 10pt-radius tray. Cards that scroll under the bottom corners spill over them.
   - **Fix:** the scroll viewport must not reach the corner arcs. Either inset it by at least the radius (cards sit ≥8pt in from the tray edge, with a 10pt tray radius), or make the scroll element its own child that starts below the header and ends `radius` above the tray bottom. Any child fill that touches an edge (footer tray, header band, selected row) must use the concentric inner radius: inner = outer − inset (with a 1pt border: `rounded_b(outer−1)`). **No negative margins/offsets.**
3. **GPUI can't rotate text.** Glyph sprites always use `TransformationMatrix::unit()` (`window.rs:5352/5362`). Only SVGs accept a transform (`elements/svg.rs:69`). So the current `chars()` letter stack is a workaround, and it's wrong (see the rail section).
   - **Feasible faithful path:** `svg().data(bytes)` accepts inline SVG, and usvg resolves `<text>` through fontdb (`svg_renderer.rs:45-66`, "Parsing resolves fonts and converts text to paths"). Render the label as `<svg><text transform="rotate(-90)" font-weight="600" font-size="13">Backlog</text></svg>`, tinted by `text_color`. Measure the width with `window.text_system()` (or a fixed per-title width) to size the element.
   - Risk: the text renders as a greyscale mask, so it will differ slightly from the native text. The lead must accept this or choose an alternative; I can't invent a different design.

## Item → component mapping (which reference controls what)

| Component (file) | Controlling reference | Secondary |
|---|---|---|
| Board columns, column header, exit rule, collapsed rail (`tasks/board.rs`) | `E62FEC84` | `AE10217A` (light card anatomy) |
| Board card (`board.rs render_card`) | `AE10217A` (light) | `E62FEC84` (amber "needs you" line) |
| List rows + group headers (`board.rs render_list`) | `630BC713` left | `97129A65`/`C24C9437` count pill + status ring |
| Task drawer/detail (`tasks/detail.rs`) | `630BC713` right | `AE10217A` drawer activity |
| Runs/step tracker (`tasks/runs.rs`) | video `B457FF32` | `C24C9437` progress ring |
| Project header, progress, tabs (`projects/mod.rs`) | `630BC713` top (breadcrumb, title+pill, bar, Board/List tabs) | — |
| Project settings/agents/config forms (`projects/settings.rs`, `workflow/*`) | `B6BD5981` right panels (card sections, 24pt rows) | `769AF488` button sizes |
| Workflow stage strip/editor (`workflow/editor.rs`) | `AE10217A` card language + `B6BD5981` panels | **No direct reference for the stage editor. The lead must decide.** |
| Onboarding/template picker (`onboarding/mod.rs`) | `85F1DC37` | `769AF488` buttons |
| Skeletons (`ui.rs skeleton_rows`) | `71B3729C` | — |
| Segmented control (`ui.rs segmented`) | `E62FEC84` Me/My team/Company, `AE10217A` Ship/Launch/Triage | — |

## Measured targets (pt)

### Board, from `E62FEC84` (ratios) cross-checked with `AE10217A`
- Column width ≈ 208px/0.73 ≈ **284pt** in E62; AE10217A cards ≈ 245pt plus tray padding ≈ 270pt. Target **272–284pt** (current is 268).
- Gap between columns ≈ 8px → **11–12pt**.
- Column header: title **13pt semibold, primary text color**; count is **right-aligned at the far edge**, 12–13pt, muted. The count is not inline next to the title (current is inline with the title).
- No leading collapse button or color dot in E62 (the chevron lives only on the rail). The current `‹` button plus dot before the title is extra chrome. I recommend keeping collapse but moving it to the hover/right side; **the lead must decide.**
- Exit rule: one line, muted, **≥11pt** (the reference ratio gives ~9.5pt, but the user rejected tiny type, so use the floor). Truncate it, don't wrap. Current native `11-stages` wraps it onto two lines at body size, which makes the description compete with the title.
- Header padding: the title sits ≈8px (11pt) from the tray's top and left. Header to first card ≈ 8px (11pt).
- Card: tray inset ≈ 4–6px each side in E62 (≈ 8pt); AE10217A uses 12px/1.3 ≈ 9pt. Target **8pt** inset and **8pt** gap between cards. Card radius ≈ **8pt** inside a **12pt** tray radius (concentric: 12 − 8 inset ≠ 8 exactly, so use tray 14 / card 8, or tray 12 / card 6; **the lead picks the token**). Card inner padding **10–12pt**.
- Card text: title **13pt medium**, up to 2 lines, line height ≈1.35. Meta line (agent / time / id) **12pt** muted with a 14pt icon. AE10217A shows id+priority on top, title, then agent + time. E62 shows title, then the status line in amber (`2 questions for you`), then a chip row.
- The footer is **not** a separate bordered band in either reference. It's just a row inside the padding. The current `border_t_1` footer band is not in any reference.

### Collapsed rail, from `E62FEC84` (crop x20–280, y100–300, 4× zoom)
- Width ≈ 27px → **36pt**. Full column height, same tray fill and radius as columns, **no visible border**.
- Gap to the next column ≈ 3px → **4pt** (tighter than the gap between columns).
- Order top-down:
  - chevron `›`, centered, at the same y as the column titles' center;
  - count `128`, muted, 12pt, on the same row as the exit-rule line;
  - **8pt** gap;
  - title rotated **90° counter-clockwise (reads bottom-to-top)**, **13pt semibold, primary color** (the same role as the column title).
- Everything is horizontally centered. The label is **not** a vertical stack of letters.
- Current bug (`board.rs:197-218`, user screenshot): 40pt wide, bordered, per-letter `chars()` stack at 11pt, count/chevron spacing from rem `gap_2`/`py_2`, ring on hover. **All of these need to change.**

### List, from `630BC713`
- Group header row ≈ 36pt: chevron, status ring 14pt, label 13pt medium, count 13pt muted.
- Task row ≈ **40pt** pitch: priority bars, id in mono 12pt muted, status ring, title 13pt. Hover/selected fill spans the full row with radius 6.
- Filter chips: 28pt high, 1pt border, radius 8, 13pt text, leading `+` icon.

### Drawer, from `630BC713` right
- Header: id caption 11–12pt muted mono; title **17–18pt semibold**; icon buttons 24pt at the top right. Padding 20pt, with a divider under the header.
- Property grid: label column ≈ 100pt, 13pt muted; values 13pt primary; row pitch **29pt**.
- Section headings ("Description", "Subtasks 2 of 5", "Activity"): 13pt semibold, 24pt above and 8pt below.
- Subtask rows ≈ 33pt with dividers and a 14pt checkbox.
- Native `18-drawer` gets wrong: values are ~11pt (smaller than the labels), there is no header divider, title/id sizes are ad hoc, and the input boxes are 26pt with 11.4pt text.

### Settings / workflow panels, from `B6BD5981`
- Section cards: radius 12, padding 16, gap 10 between cards.
- Header 13pt medium; rows **24pt** pitch at 13pt; secondary values muted, right-aligned.
- Native `31-stage-editor` gets wrong: full-bleed 26pt inputs, mixed 11/12/13 type, label-to-input gaps of 4–6pt, a segmented "Todo" stretched full width, and checkbox rows with borders on every row.

### Controls (all views), from `769AF488` + refs
- Buttons:
  - md **28pt** high, 13pt text, padding 10pt;
  - sm **24pt**, 12–13pt text;
  - icon button 24pt.
- Inputs **28–32pt** high with **13pt** text.
- Segmented: track padding 2, item height 24, item text 13pt, active = raised surface with a shadow.
- Chips: 20pt high, 12pt, radius 6.
- Count pill: 18–20pt, 12pt.
- Current `ui.rs` uses 18pt chips with 11pt text and a 22pt segmented control with 12pt text. All are below these targets.

## Type roles to add (shared helper in `tasks/ui.rs`, px-based)
`title_lg 18/24 semibold` (drawer and page titles) · `title 13/18 semibold` (column/section headers) · `body 13/18` · `meta 12/16 muted` · `caption 11/14 muted` (floor; ids and exit rules only).
Replace every `text_sm/xs/lg` and every raw `text_size(px(..))` in my views with these roles. Spacing becomes a px scale of 4/8/12/16/20/24, not rem helpers.

## Exact issues to fix in my scope (batch)
1. Rail: rotated label via SVG text; 36pt, no border, 4pt gap; chevron and count aligned to the header rows.
2. Column header: count right-aligned; title 13 semibold; one-line exit rule; drop or move the leading `‹` and the dot (lead decides).
3. Scroll body inset ≥ the corner radius; concentric card radius; no footer band.
4. Card anatomy per `AE10217A`; meta at 12pt; amber status line per `E62FEC84`.
5. `ui.rs`: chip, count pill, segmented, `choice_card`, `field`/caption, `save_bar` and skeleton resized to the roles/controls above.
6. Drawer: property grid and section rhythm per `630BC713`.
7. Settings/workflow: card sections per `B6BD5981`.
8. Onboarding modal per `85F1DC37`.

## Lead decisions (accepted)
- Rem base goes to 16 centrally. Views use the shared semantic px roles from the master helpers. No ad hoc 11/12.
- Radii are set per reference component by the lead (not one global pair).
- Rail uses SVG rotated text. System font only, no letter-stack fallback, no new dependencies.
  - Verified in `svg_renderer.rs:140-182`: fontdb runs `load_system_fonts()`. Unknown families fall back to generic SansSerif.
  - The app font is `.SystemUIFont` (`gpui-component theme/mod.rs:796`). fontdb won't recognise that private name.
  - So the SVG names `SF Pro Text` at weight 600 (see "Rail SVG font: exact choice" below), not SFNS.
  - Confirm the glyphs match the app's SF in the end-of-batch native check.
- The expanded column's leading chevron moves into the existing trailing hover/context action. Collapse stays functional.
- Status marks follow the master reference: rings as in `AE10217A`, filled stage dots as in `8AEF34E3`.
- Stage editor is composed from the `630BC713` property grid, the `B6BD5981` inspector panels and the `851174F7` diagram nodes. No new layout behaviour.

### Rail SVG font: exact choice (read-only metadata, verified)
- GPUI uses `usvg 0.46.0` + `fontdb 0.23.0`. `fontdb` reads names from the font's own name table (`parse_names`) and the weight from `OS/2` (`parse_os2`). It indexes each face once and does not enumerate variable-font instances. `usvg 0.46` does not apply `wght` variations to outlines (the only `variation_coordinates` use is `text/colr.rs`).
- `/System/Library/Fonts/SFNS.ttf`:
  - The name table has nameID1 = `System Font`, nameID4 = `System Font`, nameID6 = `.SFNS-Regular`, and no nameID16.
  - CoreText reports `.SF NS` with named instances, but fontdb only sees one Regular face named "System Font". `font-weight="600"` would still render **Regular**.
  - Reject it.
- `/Library/Fonts/SF-Pro-Text-Semibold.otf`: static font, nameID1/16 = `SF Pro Text`, subfamily `Semibold`, `usWeightClass` 600. `SF-Pro-Text-Regular.otf` = 400. Both are present on this machine.
- **Use** `font-family="SF Pro Text" font-weight="600"`. Text optical size is right for the 13pt role.
- Caveat: `/Library/Fonts/SF-Pro-*` comes from Apple's SF Pro download and isn't guaranteed on every Mac. If it's missing, fontdb falls back to its generic SansSerif. That isn't a letter stack, but it won't be SF. The lead should note this as a distribution risk. The final native check confirms the glyphs.

## Open decisions for the lead (superseded by the section above)
- Fix the rem at the root (theme owner), or use explicit px roles in views? This changes every Kit `Button`/`Input` size I use.
- The radius token pair for tray/card/control.
- Rail label: accept the SVG-rasterised text?
- Column chevron/dot chrome.
- The stage editor has no reference.
- Dark refs vs the light default: ratios apply to both, and the palette belongs to the theme owner.
