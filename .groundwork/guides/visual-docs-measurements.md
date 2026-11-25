# Visual measurements: docs, review, stats (B, 2026-11-24)

Research only. No product source changed. Spatial board images are the authority; `spatial-references-a/b.md` are navigation only (their "13px dense" reading caused the bad result). Final pt values are **proposals to map onto the lead's master scale**; ratios are the measured part.

## Method and confidence
- Originals read byte-exact from Spatial `Originals/<ID>.png|.mp4` (board untouched). Ink-row profiling (ffmpeg → raw gray → per-row ink runs) for text; visual estimates are marked `~`.
- Calibration: Monocode (`DB2A4628`, real macOS screenshot): menubar "File" asc 12px and traffic-light pitch 24.5px (20pt) → 1.23–1.28 px/pt. Video `F046` frame 0 (3302×2160, unzoomed): sidebar row 53px ≈ 28pt row → s≈1.9. Old native evidence is exact 2×.
- Inter-like metrics used: ascender ≈0.76em, asc+desc ≈0.95em. P = prose size in the same image. Ratios ±5%.

## Root cause of "everything is small" (lead-owned, `theme.rs:109`)
`theme.font_size = 13` → gpui-component `root.rs:731` sets **rem = 13px**. So `text_xs` = 9.75pt, `text_sm` = 11.4pt, `text_base` 13, `h_8` = 26pt, Input Medium = `text_sm` 11.4pt in 26pt (`sizing.rs:245,281`). My views use `text_xs` 46× and `text_sm` 27× → most labels render at 9.75/11.4pt. Kit default is rem 16 (`theme/mod.rs:797`). **Agreed: master rem 16.** The lead owns the final semantic roles. My views must use role tokens, never `text_xs/sm`.

## Measured hierarchy (ratios to P)
| Role | Guide video F046 | Overview img 9 / 8 | Docs img 1 | Monocode img 3 | Old native (pt) |
|---|---|---|---|---|---|
| Page/PR title | 1.40 semibold | – | H1 1.85 semibold | – | doc H1 28, tour title ~15 |
| Section/chapter title | 1.29 semibold | 1.18 semibold; "Overview" 1.07 semibold | H2 1.25 semibold | – | chapter 15, H2 17 |
| Prose P | 1.0, lh 1.50 | lh 1.57 | lh 1.63 | ~13.5pt, lh ~1.65 | 13, lh ~1.3 |
| Paragraph gap | 0.95P | ~0.9P | ~0.6P | ~1P | ~0.5P |
| Heading top gap | – | 2.2P section rule | 1.6P | – | ~1.0P (cramped) |
| List/chapter rows | file rows 0.92P text, pitch 2.1P | chapter list pitch 2.7P, 0.9P text | bullets pitch 2.0P | – | 26pt |
| Meta/labels | 0.85P (Commits, rail labels) | 0.86P (Risk row), counter 0.8P | 0.87P | – | 11–12 |
| Mono diff | 0.9P, lh 1.47P | 0.95P, lh 1.52P | – | – | 12, lh 13.5 |
| Diagram node text | – | 0.9P medium | – | – | ~11 |

Three distinct title tiers (do not flatten):
- **Document H1: 1.85×** body (Scriptify: H1 25px vs body 13.5px; old 116 H1 28pt vs body 13pt = 2.15, keep the ≈1.85–2.0 band). It is clearly the strongest element on the page.
- **Review page/PR title: 1.40×** prose.
- **Section/chapter headings: 1.2–1.3×**, semibold. Doc H2 1.26×, guide chapter 1.29×, overview "Overview" heading 1.07× (that one relies on weight).
- **Panel/popover titles: same size as their rows (0.93–1.0×), semibold.** From the ADFBCD tray: "Plan" cap 26px vs row text 35px asc+desc.

Reference page title vs chapter (1.40 vs 1.29) is close by design. Separate them by placement and spacing, not by an extra size jump.

## Proposed role targets (map to master)
| Token | pt / lh / weight | Uses |
|---|---|---|
| doc.title | 28 / 34 / semibold | doc H1 (unchanged size; fix alignment) |
| doc.h2 / h3 | 19 / 26 / semibold; 16 / 22 / semibold | doc headings, **same in read and edit** |
| doc.body | 15 / 24 / regular, measure ≤ 680pt | doc paragraphs/lists |
| read.title | 20 / 26 / semibold | review tour title |
| read.section | 18 / 24 / semibold | chapter title, "Overview", "How the changes fit together" |
| read.body | 14 / 22 / regular | guide/overview prose |
| ui.body | 13 / 20 / regular·medium | rows, buttons, inputs, tabs, diff header |
| ui.meta | 12 / 16 / regular, text_secondary | meta, counters, captions, rail labels |
| mono.diff | 12.5 / 20 / regular | diff and code lines, gutters |
| ui.micro | 11 / 14 / medium (+0.3 tracking caps only) | stat labels, heatmap axes; nothing below 11 |

## Layout
- **Guide** (video F046, img 8): no 248pt chapter column (`guide.rs:104`). Per chapter a two-column row: prose/files left ≈41%, diff right ≈59% (5:7), column gap ≈2.5P (~36pt), content max ≈1180pt, outer padding 24–32pt. Chapter header: counter "01 / 03" ui.meta + "Reviewed" checkbox inline under the read.section title. Chapters separated by a full-width hairline with ≈2.2P (~32pt) above/below. Optional left chapter tick rail (short 12×2pt ticks, active text colour) replaces the nav column; it keeps chapter jump behaviour. Below ≈900pt width stack diff under prose.
- **Overview** (img 9): risk row on top (ui.body label, tone value, 5-segment meter 10×3pt pills, 2pt gap); then 5:7 columns: left read.section "Overview", read.body prose, numbered chapter list (number box 26×22pt mono ui.meta, radius 5, 1px border; row pitch 38pt), footnote ui.meta tertiary; right read.section heading + diagram panel.
- **Docs** (img 1): breadcrumb header 44pt (Docs › title ui.body medium, save pill), toolbar 40pt; column max 680pt centred; title/meta/body share **one left edge** (old 56: title 872px, meta 856px, body 866px → 3 edges). Title→meta 6pt, meta→body 28pt. Heading spacing ≈24pt above (1.6×) and ≈8pt below (0.6×); paragraph gap ≈10pt; bullets 6pt dot with a 20pt hanging indent and a 28pt row pitch for lists with bold leads (img 1).
- **Stats/Home**, pixel-measured:
  - **DB2A4628 (Monocode)**: scale 1.23–1.28 px/pt from the menubar and traffic lights, about ±5%.
    - Stat tile 189×82px ≈ 151×66pt. Gaps 10px ≈ 8pt both ways. Corner radius ≈ 6pt (edge reaches the side about 6px below the top).
    - Numeral digit height 19px ≈ 21pt, semibold. Label asc+desc 13px ≈ 11pt, secondary. Numeral:label ≈ 1.9.
    - Padding: left 17px ≈ 13pt; top to numeral cap 20px ≈ 16pt (≈ 12pt with a line box); label to bottom 18px ≈ 14pt. Numeral→label gap ≈ 4pt.
    - Section label "Today" ≈ 10pt, secondary, 12pt above the tiles.
    - Heatmap cells 14px ≈ 11pt square, gap 4px ≈ 3pt (pitch 18px), radius ≈ 2pt. Axis labels mono ≈ 9–10pt tertiary.
  - **DFCD11C8 (Home widgets)**: 1230px downscaled screen, unknown display, so ratios only (±10%).
    - Widget = outer card with a header row (icon + title + count; right-side meta tertiary), then an **inset inner card**: inset ≈ 0.8× title size, radius concentric (outer − inset).
    - Hero numeral "54%" cap ≈ 1.7× the title cap; subline the same size as the title.
    - Heatmap cells are **rectangles**: w 10px × h ≈ 13px (w:h ≈ 0.75), x pitch 11.3, y pitch 15.3, so the gap ≈ 15–20% of the cell. Weekday labels ≈ 0.75× title. Legend "Less … More" at the footer.
  - **Proposed** (mapped to rem 16 master):
    - stat_tile: 13pt padding, value 22/26 semibold tabular, label 12/16 secondary, 4pt value→label, radius 8 (tile) inside radius 12 (card), gap 8.
    - Heatmap: cell 11×11, gap 3, radius 2; axis ui.micro.
    - Widget header: 36pt, title ui.body medium, meta ui.meta tertiary, inner inset 8 with concentric radius.
  - Metric strip (img 6, visual only): caps label ui.micro tracking +0.4, value 20 medium, hairline separators.

## Components and states
- **Diff card** (img 8/9): radius 10, 1px border, no shadow. Header 40pt: lang icon 14, dir path text_tertiary + filename text medium (ui.body), `+n` success `−n` danger tabular, "Reviewed" checkbox 14pt + ui.body. Gutters: old/new number columns 36pt each right-aligned tertiary, sign column 16pt, code inset 8pt. Added row bg success@10%, removed danger@10%, plus 2pt left bar full-tone on changed rows; hunk header row ui.meta tertiary on tray. Hover row: hover token; wrapped lines keep gutter blank.
- **Clipping (root fix)**: GPUI `ContentMask` is rectangular (`gpui-pre window.rs:2542`); `overflow_hidden` never clips to radii. Any child with a bg touching a card edge must carry the matching inner radius (outer − 1pt border) on those corners: header `rounded_t(9)`, last diff row/footers `rounded_b(9)`, first/last added rows too; or inset by ≥ radius. Applies to `review/mod.rs:561` panel, `guide.rs:256-300`, diff_tab, overview feedback, stats tiles.
- **File reference rows** (img 8): height 36pt, radius 8, px 10, icon 14 + dir tertiary + name; trailing ±counts. States: rest none, hover tray, selected tray + text medium, focus ring 2pt running@40%.
- **Diagram** (img 9): panel bg tray, 1px border, radius 12, label "Diagram/title" ui.meta tertiary at 16pt inset. Nodes: height 40pt, auto width (text + 2×16pt, min 140, max 260) not fixed 210×38 (`diagram.rs:10-13`), radius 8, surface bg, 1px border, `shadow_xs`, text ui.body medium centred. Rank gap 56pt with edge label ui.micro medium on panel bg (no tray chip, `diagram.rs:215` 10pt). Edges 1pt text_tertiary, orthogonal with 6pt rounded elbows; arrowhead running colour 7×5pt. Hover node border running.
- **Commit rail** (video F046): label row "Commits 20 · +687 −302" ui.meta. Rail 2pt running line; earlier history dotted running@40%; commit dot 8pt with 16pt running@15% halo; head = 12pt ring 2pt; grouped count pill 22pt high running bg white ui.meta medium, with 2 ghost arcs behind. Under-labels ui.meta: plain tertiary, group chip running text on running@10% radius 6 h 22 with chevron, branch chips attention/danger tone with branch icon. Merge branches enter from above as 1.5pt toned bezier with 8pt dot.
- **Rail popover** (F046 3–9s): card surface radius 12, 1px border, shadow md, width ≈300pt, padding 14. Head: tone chip + sha mono ui.meta tertiary + right time ui.meta; title ui.body semibold; desc ui.meta secondary; status row success icon + success text; conflict variant = attention@12% band, warning icon, attention-dark title; hairline; author row avatar 16 + ui.meta; commit rows sha mono + message + ±. Motion: enter 150–180ms opacity 0→1, y 4→0, scale .98→1 ease-out; switching target crossfades content ~150ms in place (no re-drop); exit 120ms. Honour Reduce Motion (opacity only).
- **Tray popover** (ADFBCD30, 1080² @60fps). Frame 4.3s is camera-zoomed, so values are ratios to the row text P (asc+desc 35px). P=13 gives the pt values.
  - Geometry: width 23.6P ≈ 307pt; toolbar row 2.8P ≈ 36pt; header row 2.6P ≈ 34pt; list rows pitch 1.74P ≈ 23pt with ≈0.5P vertical list padding. Rows, header and toolbar are divided by hairlines inside **one** container. Footer row ("See all changes" + external icon) sits under a hairline.
  - Icon buttons: ≈2.6P box with a selected tray fill; red count badge overlaps the top-right corner.
  - Motion, from 30fps segments:
    - Open (2.77→2.80s) and content/size switch (5.60→5.63s): **one-frame cut**. The toolbar container grows downward to include the panel, with no tween on height or content.
    - Promotion (16.67→16.70s): clicking the active icon docks the same content into a side-panel tab ("Branch"), also a cut. The later zoom is the recorder's camera, not app motion.
  - So the only tweened popover motion is the F046 rail popover (fade/blur ≈150ms). Don't add height springs to tray panels.
- **Tray/segmented** (img 2, ADFBCD30): tray bg, radius 12 outer, item radius 8 (concentric = outer − padding 4), selected item surface + shadow_xs, text ui.body medium; badges 16pt danger circle ui.micro white. Tabs Overview/Guide/Diff: pill segmented, h 28, px 12.
- **Inputs/buttons**: h 28 (ui) / 32 (primary actions), radius 8, ui.body text; review feedback textarea ui.body lh 20, padding 10/12, min 3 lines.

## Old native 54 / 116 (exact 2×)
- **54 feedback decision:**
  - Focused textarea shows a ≈4pt double grey ring (rows 513–520px) that crowds the card's 8pt inset, so the focus ring bleeds into the padding. Want: 1pt border plus a 2pt running@35% ring, with the card inset ≥ the ring.
  - Decision segmented control is only 16pt tall (32px), text at xsmall; the selected "Request changes" fill hugs the text.
  - Review-feedback card title and "Saved with the tour" sit on a different baseline than the textarea.
  - Diagram nodes 163×29pt with ≈11pt labels: too small for the box.
- **116 reopen doc:**
  - Every heading has the same gap before and after (heading top→body top 29.5pt; body bottom→next heading 15pt). Want ≈1.6×body above and ≈0.6×body below, so headings bind to their content.
  - Bullet markers are small 4pt dots with a 14pt hanging indent.
  - The selected rail row fill is darker than hover/selected tokens elsewhere.
  - "Goal ** BoldQA**" shows literal asterisks. This is a content/markdown-parse observation, not visual; I'm not changing it.

## Exact source issues (my files)
- `docs/mod.rs:912-916` edit sizes 26/20/17/15 vs read `TextView::markdown` (`:933`) at rem defaults → sizes jump on click. Use one `TextViewStyle` (heading_font_size fn, paragraph_gap) + `text_size/line_height` equal to the editor roles.
- `docs/mod.rs:857-874` title 30pt Input with `px_0` inside `px_2` vs meta `px_2` and blocks `px_2`+`py_0p5` → 3 left edges observed (56). Meta `text_sm` (11.4).
- `docs/mod.rs:640,677-679,730` rail label/breadcrumb/save pill `text_xs/sm` → 9.75/11.4.
- `review/guide.rs:83,158,279,289,347,363,378,397` mono/counter/code at `text_xs` 9.75; `:104` 248pt chapter column; `:129` max 1040 single column; `:293` 44pt single gutter.
- `review/diagram.rs:10-13,215,226-236` fixed node 210×38, 10pt edge chips.
- `review/overview.rs:39,118,144-255` meter 10×4, decision segmented radius 8 on tray at xsmall, chapter numbers mono `text_xs`.
- `review/timeline.rs:53,151,174,215,230,289,312-318` rail 2px border colour, 12pt dots, text_xs shas.
- `review/mod.rs:561` `overflow_hidden` relied on for rounding.
- `stats/heatmap.rs:53,54,93` 9pt labels; `stats/mod.rs:316,344` fixed 620pt widgets; `:504-510` bars 6pt.

## Helper requirements (lead, `theme`/`components`)
1. Role text tokens: `text_role(Role)` setting size + line height + weight (roles above). 2. Radii tokens: 6 (chips), 8 (controls/rows), 10 (cards), 12 (panels/popovers) and `inner_radius(outer, inset)`. 3. `card_section(edge)` helper applying edge radii for header/footer/row bgs. 4. Tone tints: `tone.tint(0.10|0.15)`. 5. Shared `segmented` (h 28, item radius = outer − 2), `meter`, `file_row`, `stat_tile` (spec above), `inset_card(outer_radius, inset)`, `popover_card` (rail: fade 150ms; tray: no tween), `tray_panel` (toolbar/header/list/footer hairline sections in one container). 6. Mono role token (font + 12.5/20). 7. Focus ring token (1pt border + 2pt ring) that fits inside insets.
`stats/common.rs:14-34` (`card`, `card_header`, `muted`) duplicate `components::card/section_label` with private sizes; migrate to lead tokens/helpers and delete; keep only the date helpers there.

## Verification (batched at end)
Native screenshots of Docs (light), Guide, Overview, Diff, a rail popover, Home in light+dark against the Spatial originals above and old 51/53/54/56/116; check edges at card corners at 2×, one left edge in docs, no text < 11pt, read/edit size parity. Report each as compared, not "faithful", unless side-by-side matches.
