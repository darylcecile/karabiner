# Visual contract (master)

The visual source of truth for the client redesign. All values are native points (pt = GPUI px at 1×). Where a reference conflicts with this document, the user's Spatial board (`D3C651DA…` › folder `844C40CB…`) wins and this document should be corrected. `spatial-references-a/b.md` are navigation notes only. Their 13px/dense estimates are **not** authoritative. Per-view measurements are in `visual-project-measurements.md` and the B researcher's guide. Where those overlap with this file, this file decides.

## Root causes (historical, gpui-pre 0.3.14 / gpui-component 0.11.0)

These explain the original redesign. They are not claims about the current source. Item 3 remains a platform fact.

1. **The rem is set to the theme font size.** `gpui-component/src/root.rs:731` calls `window.set_rem_size(theme.font_size)`, and `theme.rs` sets `font_size = 13`. Every rem helper therefore shrinks by 13/16: `text_sm` = 11.4, `text_xs` = 9.75, `p_2` = 6.5, icon `size_4` = 13. Every Kit control shrinks too, because Kit sizes are rem-based (`button.rs:709-728`, `sizing.rs:245-288`): Button md/sm/xs = 26/19.5/16.25 and Input md = 26 with 11.4 text. Views then hard-code `px(11.)` (24×) and `px(12.)` (45×) to compensate. The result is around four competing sizes between 9.75 and 12, which is why the type looks small and has no hierarchy. **Fix:** `theme.font_size = px(16.)`, the Kit default, so rem = 16 and Tailwind/Kit tokens are nominal again (`p_2` = 8, `h_8` = 32, Kit small = 24, xsmall = 20). The 13pt UI default is set separately on the shell root with `text_size`. All text comes from the role helpers below.
2. **The default line height is φ.** `style.rs:494` sets the line height to `phi()` (1.618× the font size), so rows look loose even though the text is tiny. Each role helper sets an explicit px line height, and the shell root sets `line_height(relative(1.4))` as the fallback.
3. **Clipping is rectangular.** `ContentMask` (`window.rs:2542`) is bounds-only. `rounded(..).overflow_hidden()` does **not** clip children to the curve. Child fills and scrolled content paint square corners over the parent's arc. See the clip policy.
4. **Text cannot be rotated.** Glyph sprites always use `TransformationMatrix::unit()` (`window.rs:5352/5362`). An SVG `<text>` rendered via `img(Arc<Image{Svg}>)` goes through usvg with the system font DB (`svg_renderer.rs:46-110`), which provides real glyphs. This is the only approved route for the rotated rail label. A per-letter stack is **forbidden**.
5. **The traffic lights are misaligned.** Kit `TitleBar::title_bar_options()` places them at (9,9), which is designed for a 34pt bar. Our bar was 38pt, so the lights sat about 4pt high. Override `traffic_light_position` in `main.rs` window options to match the bar height below, and verify natively.
6. **The tab strip hard-clips with no affordance.** The strip clips at the nav buttons (baseline `.groundwork/tmp/client-acceptance/visual-baseline-current.png` shows "ument out…"). There is no fade, no overflow arrows, and no reveal margin (`5EEF77CF` marks this ✗).

## Typography roles (`components::typography`, trait `Typography: Styled`)

Font: `.SystemUIFont`. Each helper sets size, line height and weight. Apply colour separately using the colour roles.

| Helper | Size/line | Weight | Use (reference) |
|---|---|---|---|
| `t_title()` | 22/28 | semibold | Page title ("General", "Models", "Plan & usage": `92D96623`, `1721E892`, `527B3F33`) |
| `t_headline()` | 17/22 | semibold | Compact drawer/dialog title (`630BC713`, `85F1DC37`) |
| `t_section()` | 15/20 | semibold | In-page group heading (Overview, "How the changes fit together", Daily usage `527B3F33`) |
| `t_body()` | 14/20 | regular | Page subtitle, descriptions, settings-row titles (medium) and descriptions, empty-state copy, form help (`92D96623`, `B97FF317`) |
| `t_label()` | 13/18 | regular, or medium when emphasised | **UI default:** sidebar nav, tabs, list/board titles, buttons, inputs, menu items, property values, column titles (semibold) |
| `t_trace_label()` | 16/22 | regular | **Only** trace span names (measured on `F263F6D3` 38pt rows) |
| `t_caption()` | 12/16 | regular | Meta (time, id, counts), chips, the exit-rule line, small buttons, legend |
| `t_micro()` | 11/14 | medium, tracked +0.4, uppercase | **Only** stat-strip labels (`CCCF2D3A`: DURATION/COST…) and kbd hints. Never descriptions, labels or values. |

Rules: a view must not call `text_xs/sm/base/lg/xl` or `text_size(px(..))` directly. The only exceptions are numeric display figures (`t_display()` 28/32 semibold, for the stat value "1,705" and "$20" in `527B3F33`). Each role means the same thing everywhere. Secondary text uses a colour change, not a smaller size. Hierarchy comes from a size step **and** a weight step (title → body is 22 → 14).

## Colour roles (`theme::Tokens`, unchanged values plus additions)

Text: `text` (#171717 light / #EDEDED dark) is primary. `text_secondary` (#737373 / #8A8A8A) is used for descriptions, meta and inactive tabs. `text_tertiary` is for placeholders, disabled text and decoration only, never information that has to be read. Add `accent_fill` (#171717 / #EDEDED) and `accent_text` (white / #171717) for primary buttons ("Upgrade" `92D96623`) and selected toggles. Surfaces: light `canvas` #FFFFFF (page, sampled from Hexta) / dark #121212; `surface` #FFFFFF / #1A1A1A; `section_fill` #FAFAFA / #1C1C1C for grouped settings cards (`92D96623`, no border); `board_column` #F4F4F4 / #1D1D1D and `board_card` #FFFFFF / #2A2829 for the board only (`E62FEC84`, `AE10217A`). Status colours are unchanged. Focus ring is `running` at 2pt.

## Spacing and layout

The scale is 2, 4, 6, 8, 12, 16, 20, 24, 32, 40 (rem 16 makes Tailwind tokens nominal). **Each gap has one owner.** A parent `gap` *or* a child margin owns a gap, never both. Space within a label/control pair is tighter than space between groups (Impeccable layout).

- Shell: one frame owner, the content wrapper beside the sidebar, pads the card 8 on the right and bottom, and on the left by the collapse gutter (0 when the sidebar is open, 8 when it is collapsed). The card fills the wrapper with `flex_1`/`min_h_0`; it never uses `h_full` plus a margin. The sidebar clip is only the animated sidebar width. `card_left` for the tab bar is the sidebar width plus the gutter. The card's top edge meets the titlebar (the active tab connects to it).
- Page gutter inside the content card: 24 horizontal, 20 top, 24 bottom. Forms and settings use a reading width of max 720, left-aligned to the gutter (`92D96623`).
- Page header: title, then 4, then subtitle (`t_body` secondary), then 24, then first block.
- Section rhythm: 28 above a section label, 8 from the label to its card, and the label is indented to the card's inner padding (Hexta "Startup").
- Section/settings card: `section_fill`, radius 16 (`R_SECTION`), no border, padding 16 horizontal. Rows are at least 56 (title + description), with 12 vertical padding and a 1px `border` divider inset 16 from both card edges. Controls are right-aligned and vertically centred (`92D96623`).
- Drawer: padding 20, divider under the header, property-grid label column 100, row pitch 28 (`630BC713`).
- Shell inspector (`shell/inspector.rs`, `B6BD5981`): 320 panel inside the content card on the card's `canvas` (so an active tab above it joins one continuous card), cards one step raised on `section_fill`; header hover = `hover` fill R6 extending 4pt past the text. Docked while the primary view keeps ≥ 480 (`PRIMARY_MIN`: drawer 100 labels + ~300 values + 2×20 padding); narrower, it floats over the card's right side (absolute, `shadow_lg`, occludes), keeping the full 320 and the toggle. Activity meta: time and kind wrap as whole units with no separator.
- Inspector layout: 44 "Inspector · Read-only" header, cards inset 12 with 8 gaps. Each section (Context, Environment, Linked documents, Runs, Activity) is a `card` (R12, 1px border, surface) with 16 padding. Header 24: `t_panel_title`, 12 chevron in a 16 box (real collapse, all open by default), real count as right `t_caption` meta (hidden at 0). Rows 24 pitch, label column 88 (264 content − borders), values and chips centred on the row. Activity shows 5 entries then "Show N more"/"Show less"; history is never dropped. No invented Checks/MCP/PR data.
- **Task detail has two layouts.** (a) The **compact drawer** (`630BC713`) overlays the project card at the **full content-card height**: top to bottom, top-left radius concentric with the card, not bounded by the board/list area. Its description is plain prose, not a bordered box. (b) The **full Task tab** uses a Linear-style split (user direction): main column on the left (title, description, subtasks, activity) and a properties sidebar on the right with its own hairline. It does not reuse the drawer's single column.
- The board grouping control defaults to **Stages** (user, verbatim: "stages should be the default").

## Component metrics

| Component | Height | Padding/gap | Radius | Text | Ref |
|---|---|---|---|---|---|
| Titlebar | 40 | tab shape 32 (y 8–40) = 24 interactive face (y 8–32, centre 20; close 20 at y 10–30) + 8 paint-only foot to the card; leading/trailing controls, overflow arrows and fades span the full 40 bar, so 28 hitboxes sit 6/6 and 16 glyphs 12/12 (centre 20); traffic lights at (14, 13) (centre ≈ y + 7 = 20); 80 reserved | – | – | `EB27363B`, `789CB4EF` |
| Titlebar icon button | 28 | icon 16 | 6 | – | `EB27363B` |
| Workspace tab | 32 shape = 24 face + 8 foot (see Titlebar) | px 10, gap 6, icon 14, max w 200, min w 88 | 8 top | `t_label`; active is medium/`text`, inactive is regular/`text_secondary` | `5EEF77CF`, `EB27363B` |
| Sidebar | default width 240 (drag-resizable, see Sidebar resize), px 8 | row 30, px 8, gap 8, icon 16, gap between rows 2 | row 6 | `t_label`; group label is `t_caption` medium secondary, pt 16 / pb 4 | `EB27363B`, `92D96623` |
| Count (sidebar/column) | – | right-aligned | – | `t_caption` secondary, tabular | `EB27363B` |
| List row | 36 (board list 40) | px 8, gap 8 | 6 | `t_label` | `EB27363B`, `630BC713` |
| Button md / sm / xs | 28 / 24 / 20 | px 12 / 10 / 6, gap 6 | md 8 (`button_md`), sm/xs 6, row action 10, pill for the hero lg 36 / footer CTA | md and sm `t_label`, xs `t_caption` | `769AF488` |
| Icon button | 28 (md) / 24 (sm) | – | 6 (square chrome/toolbar) | – | `769AF488` |
| Input / select | 28 (page search 32) | px 10, icon 14 | 6 (32 → 8) | `t_label` | `1721E892`, `92D96623` |
| Segmented | track 28, padding 2, item 24 | item px 10 | track 8 / item 6 | `t_label`; active item is surface + `shadow_xs` | `1721E892` "All/Enabled", `EB27363B` filters |
| Chip / tag | 20 | px 6, dot 6 | 6 | `t_caption` | `EB27363B`, `1721E892` |
| Status pill | 20 | px 8 | full | `t_caption` medium | `527B3F33` "Active" |
| Switch | 18 × 32 | – | full | – | `92D96623` |
| Board column tray | – | inset 8, card gap 8, header 12 top | 12 | title 13 semibold, count right-aligned | `E62FEC84`, `AE10217A` |
| Board card | – | padding 12 | 8 | title `t_label` medium (2 lines), meta `t_caption` | `AE10217A`, `E62FEC84` |
| Collapsed rail | width 36, gap 4 to the next column | chevron row, count `t_caption`, 8, rotated label | 12 (tray fill, no border) | 13 semibold, reads bottom-to-top | `E62FEC84` |
| Popover / menu | menu row 28 | padding 6, header 12 | 10 | `t_label`; header `t_label` semibold with count right-aligned | `ADFBCD30` |
| Property menu (status/priority/agent/stage) | menu row 28 | leading glyph slot 16 (same glyph as the property value) and a **trailing** check on the current value | 10 | `t_label` | user U6/U7 |
| Due date (`views/tasks/date_field.rs`) | trigger: property value | Trigger: one leading 16 glyph + gap 6 like other values, formatted date + relative time, empty "Set due date", ✕ clear beside it. Popover is the full `661192A5` composition at 0.75× of @2x: `tray` shell R14 with a white card inset 3 (R11, hairline). Header 37 holds a Calendar 16 icon and a real date input ("What date is this due?") over a hairline. Left rail 112 (rows 25, 12 inset both sides) of real draft quick choices (Today, Tomorrow, This weekend, Next week, In 2 weeks, Next month, No date). Two Monday-first month panes when the window is ≥ 800 (popup 631 + ~84 air each side); one month (popup ≈ 370) below, incl. the 720 minimum, each with a fully rounded "Month Year ⌄" pill (24, hairline, medium) that opens a month/year chooser. ‹ › 24 on the outer panes. Single-letter weekdays `t_micro`. 33×26 cells, adjacent-month days blank, 22 circles: today is a `selected` ring, chosen is an accent fill, hover is a fill, past is tertiary but selectable. Tray footer 42 holds the hint/parsed date and an "Apply date" pill (24, accent; grey when unchanged or invalid). Base `CalendarState` holds month/view/pending date. Nothing changes the task until Apply/Enter; open/close discards. A clean record refresh while the picker is open keeps the pending date and typed text. Enter applies the parsed text. | 14 | `t_label`, header `t_body` | `661192A5` |
| Split button ("New doc", merge) | 28 | one bordered R8 control. Each half fills the full inner height edge to edge: outer corners R−1, square seam. The hover fill stays in its half, and the 1×16 divider hides while a half is hovered. Chevron half is a 28 square. | 8 | `t_label` medium | `669B3D47`, `803531A8` |
| Property/menu popovers (`components/menu.rs`) | menu row 28 | `menu_popover(id, trigger, anchor, gap)` opens a Kit `PopupMenu` with an 8pt gap from the trigger. `glyph_item(icon, label)` gives a 16 leading glyph slot (gap 8) whose label aligns with the trigger's value text. Kit trailing check. Floating menus keep ≥8 from the window edge and may overlap the card edge. | 10 | `t_label` | U6/U7 |
| Bare prose field (`controls::plain_textarea`) | – | Base headless `Textarea` on the same `TextareaState`, `set_editor_paddings(0)`, so text is flush with the title/label edge. Bordered fields stay `textarea_box`. | – | caller's role | `630BC713` |
| Diff rows (review guide) | code row 20 (12.5 × 1.6), hunk header 24, max 28 rows | card height comes from the rendered rows plus hunks, not raw patch lines, so the last line is never clipped | – | `t_code` | `AEF224FF` |
| Full task tab | – | Linear split: properties rail 296 on the right. Below 776 wide it falls back to a single column with the properties above the content. | – | – | user U4 |
| Templates page | – | flat on the page gutter (no outer card), filters + grid, footer as a full-width tray band at the content-card bottom | grid tiles 12 | – | `85F1DC37` (adapted from a modal to a page) |
| Command palette | header 64, row pitch 45 | see "Command palette" | 16 | header 16, rows 15 | `0945F5E8` |
| Dialog | – | padding 20, footer band in `tray` | 14 | title `t_headline` | `85F1DC37` |
| Run trace (`components/trace.rs`) | header 52, ruler 36, row 38 | Span column is 40% of the card, capped at 320 (shared by ruler, rows, grid and popover anchor). Tile 24, bar 16, indent 16. Stats: 6 cells in one row ≥720, otherwise 2×3. 1pt tree guides only under expanded parents, ending at the last visible child's glyph centre. TraceView owns its single border as a top layer; hosts must not frame it again. Unrecorded values show "—"; no invented LLM, cost or provider data. Ruler starts at "0s". | card 12; last row fill uses the card radius | names `t_trace_label`, kind/ruler/durations `t_body`, stat values `t_stat_sm`, title `t_headline` | `F263F6D3`, `CCCF2D3A` |
| Run step (`components/run_step.rs`) | row 32 | 14 glyph; 1pt connected rail through glyph centres ending at the last step (`last`). Running glyph is a `Spinner` arc (static under Reduce Motion). Detail and approval content align with the title. Empty details say "No agents assigned" only with no agent; otherwise "No step details recorded." | control | `t_label`, meta `t_caption` | `B457FF32` |
| Card (generic) | – | 16 | 12 | – | – |
| Content card | – | – | 12 | – | `EB27363B` |

Borders are 1px `border`. Shadows: none on flat cards (border or fill only). `shadow_xs` on the active segment and raised buttons. Kit `shadow_lg` on popovers and dialogs. `shadow_md` on dragged tabs and cards.

## Workspace tab bar (`shell/titlebar.rs`, refs `5EEF77CF`, `EB27363B`, `789CB4EF`)

- The strip starts after the nav buttons and ends before the `+`, overflow and inspector buttons. The strip spans y 8–41; 32pt tab shapes are bottom-aligned to the bar's bottom edge, so every face is y 8–32.
- **Active tab = folder tab connected to the content card.** Its fill is `canvas` (the card's fill), with a 1px `border` on the top, left and right, and top radius 8. It extends 1pt down to cover the card's top border. The flares are **true concave 8pt radii**: the tab outline continues into the card border with no horizontal hairline under the active tab (`5EEF77CF` ✓ at about 56s; the ✗ at about 1s shows a gap band). Its text is `text`, medium.
- Inactive tabs have no fill. Hover shows a `hover` pill filling the 24pt face (y 8–32, R6, inset 3 horizontally); an active tab that is not fully visible shows the same face as a bordered `surface` pill. There is a 1×14 `border` divider between adjacent inactive tabs, hidden next to the active or hovered tab (`5EEF77CF`).
- Overflow: when `max_offset.x > 0`, show a 24 icon button (‹ and/or ›) at the overflowing edge, plus a 24pt `linear_gradient` fade from the titlebar colour to transparent over the tabs. Never show a scrollbar and never hard-clip text without the fade (`5EEF77CF` ✗ frames). The ‹ button appears only when `offset.x < 0`.
- Reveal: when activating a tab, scroll so its full bounds **including the 8pt flares**, plus 8pt of clearance, sit clear of the arrow and fade (`bounds_for_item` + `set_offset`). Keep tracking during the open animation (`5EEF77CF` ✓ at about 57s).
- Preserve: close on hover/active, the dirty dot, middle-click, drag reorder, the overflow menu, `+` → Quick Open, the shortcuts, and the open/close ghost animation.

## Clip and inset policy (rounded containers)

1. Fills that touch a rounded parent edge must have the concentric radius on the touching corners. A flush child takes outer minus border (e.g. `rounded_t(11)` inside a 12 card with a 1px border). An inset child takes outer minus inset, with a minimum of 4.
2. A scroll viewport inside a rounded container must be inset by at least `ceil(0.3 × R)` on **both** axes at each corner, or start below a header and end at least R above the bottom edge. Use `components::clip_inset(R)`. For board trays (R 12): the scroll body has padding 8 and the scroll element is its own child.
3. No negative margins and no offsets to hide bleed. Arbitrary content offsets are not a clipping mechanism.
4. The content card itself: views fill it with `canvas`. Full-bleed bands (toolbars, tab rows) use `rounded_t(11)` / `rounded_b(11)` when they touch the card edge.
5. **`rounded_frame`** (`components/surfaces.rs`) emulates curved clipping at render time. It is the shell card's last child: corner caps painted in the colour outside the card plus the 1pt border ring, with no hitbox. It covers full-bleed children, scrolled content and the drawer at the card's corners. It is a paint overlay, not a mask, and it does not make `rounded + overflow_hidden` sufficient (root cause 3 still holds). Use it only where a container's outside colour is known.

## Responsive layout (allocated width; `views/responsive.rs`)

Supported minimum: a 720 window with a 240–360 sidebar, inspector docked or overlaid, which leaves a main card of about 352 and roughly 302 inside the gutters. Nothing may clip horizontally at that width.

1. **Measure allocated width, not window width.** `WidthCell` + `width_probe` (the TaskDetail canvas pattern; the parent must be `relative()`) and `narrower_than(cell, threshold)`. The inspector, sidebar and splits all change the available width.
2. **Scroll extent comes from the scroller's direct child.** GPUI computes `max_offset` from immediate child bounds only. A horizontal scroller is a bounded viewport (`w_full`, `min_w_0`, `flex`, `overflow_x_scroll`) whose single direct child is content-sized (`flex_none`, plus `min_w(relative(1.))` when it should also fill). Never wrap oversized content in an auto-width block. `x_scroll_fades(handle, bg)` marks hidden sides and needs `track_scroll`.
3. **Only content scrolls.** Captions, titles and notes stay outside the scroller (diagram captions sit above the graph scroll). Navigation stays outside content scroll panes.
4. **Flex children that must shrink** need `min_w_0`. Use `flex_basis(n) + min_w_0` for wrap points. A hard `min_w(n)` is only allowed when n fits about 302.
5. **Breakpoints in use:**

   | View | Rule |
   |---|---|
   | Docs | <700: rail becomes a 200-max block above the editor; format toolbar scrolls with fades; header wraps controls, title truncates |
   | Review guide | <900: story stacks above the blocks in one tracked scroll (`focus_block` offset +1) |
   | Review overview | <900: diagram goes under the narrative |
   | Review diff | <760: file list stacks above (max 220). Card header has a filename group (basis 200) and a meta group (chip ≤140 clipped, ±, Reviewed) that wraps to a second line |
   | Home | <880: one widget column; heatmap scroller pinned once to the newest week; status legend items on a 180 basis; date/range bar wraps |
   | Project settings | <760: nav becomes a wrapping tab row above the pane, and General rows put title + description above a full-width control (no label column beside fixed 280 inputs). Agent and library forms: basis 360 with `min_w_0`; paired fields min 160 each |
   | Templates | ≥680 3 columns, ≥460 2 columns, else 1 |
   | Global settings | Harness chips wrap below the name; theme tiles wrap (min 120) |

6. **Bars that can't fit wrap to a second line of the same height.** The project tab row wraps its action group to a second 36 line. Essential actions are never faded out or hidden. Rails (workflow stages, commit timeline, heatmap, diagrams, docs toolbar) scroll with fades instead.
7. **Small or unknown data must not produce fake content.** Renderers show only stored fields. QA uses labelled in-memory fixtures in the example only, never product data.

## Motion (`motion.rs`)

| Name | Duration | Easing | Use |
|---|---|---|---|
| `FAST` / `fade()` | 120 | ease-out | hover, colour, small reveals |
| `TAB` / `tab()` | 160 | ease-out | tab open/close width + opacity |
| `POPOVER` / `popover()` | 160 | ease-out | popover and rail-detail **fade only** (`F046D5DF`, 150–180) |
| tray open / content swap / promotion | **instant cut** | – | `ADFBCD30` (the zoom is a camera move, not app motion). No height animation. |
| `STANDARD` / `standard()` | 200 | ease-out | sidebar toggle, inspector, drawers |
| sidebar drag | **live, no animation** | – | width follows the pointer; see Sidebar resize |
| scroll reveal | 200 | ease-out | strip `set_offset` (instant when Reduce Motion is on) |

Base already honours Reduce Motion. Do not add any new motion to the views.

## Sidebar resize and collapse (refs `789CB4EF` toggle, `0B143E01` drag)

- `789CB4EF` shows the toggle animation: collapse at about 1.9–2.0s, reopen at about 5.75–5.85s, roughly 100–150ms. Content is clipped at the moving edge, not reflowed, with no intermediate width.
- `0B143E01` (`Marked_-_Drag-to-resize_sidebar…mp4`, 32.5s) is an actual drag reference: hovering the seam shows a col-resize cursor, a 1pt **blue** line on the seam (we use `running`; `accent_fill` is near-black) and a "Drag to resize sidebar" hint; dragging below 160px snaps the sidebar to an **icon rail** (labels fade 0.16s, width 0.4s), and the toggle reopens it. Resolved (user): adopt the blue seam line (`running`, already the focus blue elsewhere) and the "Drag to resize sidebar" hint as the Kit `Tooltip` on the resize band (pointer-anchored, hover only). Keep full collapse + edge reopen and the 120ms timing; the icon rail and 0.16s/0.4s timings are deliberately not adopted.
- `F263F6D3` (`8qr4FRb5_WyzNt3g.mp4`, 33.4s) was inventoried as a sidebar ref, but its frames show a live agent **trace timeline** (span tree, bars, detail popover) with no sidebar; route it to the inspector/run views.
- Divider at the sidebar/content seam (9pt band, see Bounds) with a `col-resize` cursor. Dragging resizes live and clamps to the shell bounds; releasing below the collapse threshold collapses with `standard()`. Reopen by dragging from the collapsed edge or with the existing toggle/shortcut. Keep the 8pt collapsed gutter and the tab geometry.
- Bounds (`app.rs`): usable width 200–360 (default 240). During a drag the edge follows the pointer 1:1 from 0 to 360, and contents keep at least 200 wide, so they clip rather than squeeze. On release, below 100 collapses; otherwise the width settles into 200–360. Toggle, release settle and collapse take 120ms ease-out (`motion::fade`), and Reduce Motion skips them. The divider is Base's 9pt resize band with a column-resize cursor; its 1pt blue (`running`) line shows only on hover or drag (`0B143E01`). Double-clicking it toggles the sidebar. Input invariant: only moves with the left button pressed resize. GPUI dispatches `on_drag_move` for any mouse move while the drag is active (pinned `gpui-pre-0.3.14` `div.rs` checks only the drag type), so a move with no button pressed ends the drag at the last held width and calls `stop_active_drag`; it never follows the free pointer (a fast release then park produced the 360 cap). When collapsed, the band is centred on the card seam at x 8 (the gutter edge), and dragging right reopens. The last usable width is kept for the session only (not persisted). There are no springs and no minimap.

## Command palette (`shell/quick_open.rs`; refs `0945F5E8` primary, `3158C0B8`, `22F3769A`)

- The container is white, R16, with no visible border, a large soft shadow and the backdrop dimmed about 15%. It is about 740 wide, placed upper-centre. There is no outer tray and no title.
- Search header, 64 high: 18 icon at x 24 (secondary), placeholder 16 (tertiary) at x 60, and an `Esc` keycap on the right. The field has **no border and no focus ring**; a hairline sits below it.
- Section labels are 13 medium secondary. Rows are 45 apart and inset 10, with an 18 outline icon, a 16 gap and a 15 label. The right side holds the context caption or real shortcut keycaps (22, R5, 1px border); never invent keycaps. A chevron marks drill-in (`22F3769A`). Hover and keyboard selection share one fill (`hover`, R10, inset row) that moves instantly.
- Footer: a hairline, then 48 high with hints `↑ ↓ Move`, `↵ Open`, keycaps as above (`3158C0B8`).
- Motion: about 100ms opacity with scale 0.97→1, backdrop dimming at the same time. The streaming answer mode in `0945F5E8` is out of scope while Ask is a placeholder.
- Evidence crops: external `manual-evidence/visual-parity-2026-11-24/frames/pal-*`.

## Shared input height rule

Single-line Kit inputs take their height from `Size`; Kit's inherent `h` sizes the multi-line variant only. Set a single-line input's height with `<Input as Styled>::h(input.small(), px(h))`, which refines the box itself and keeps Kit's 2pt vertical inset (`views/tasks/detail.rs` `line_input`). Shared helpers should follow the same rule.

## Runtime status

Everything above is a source/reference contract. Native rendering proof is **pending** the final batched capture. Results and the per-item checklist live in the external evidence matrix, not in this guide.

## Rulings on researcher questions

- **No 11pt floor for routine text.** Use `t_caption` 12 for the minimum readable meta (exit-rule line, counts, timestamps). 11 is `t_micro` only.
- Board tray 12, card 8, inset 8. Strict concentricity applies only to flush or ≤4 insets.
- Collapsed rail: as in the table. The label uses `vertical_label`. If SVG glyphs don't render in the correct system font, report back. Never fall back to a letter stack.
- Stage editor: there is no single reference. Compose it from the `630BC713` property grid (label column 100, rows 28), the `851174F7` diagram node boxes (R8 node, border; mono index `t_code_caption`), and the `B6BD5981` inspector panels (`section_card`). Keep its behaviour and don't invent new styling.
- Expanded project header: the title row uses `t_title`, the description uses `t_body` secondary, and progress uses `t_caption`. Put the view tabs and the Status/Stages segmented control (default Stages) on one 36 row using shared `segmented` sizing. Remove duplicated badges. Follow the exact references, not a blend.


## Reading and document roles (from `visual-docs-measurements.md`)

| Helper | Size/line | Weight | Use |
|---|---|---|---|
| `t_read_heading()` | 17/24 | semibold | Review chapter title, "Overview", "How the changes fit together" |
| `t_read()` | 14/22 | regular | Guide/overview prose |
| `t_doc_title()` | 28/34 | semibold | Document H1, read **and** edit |
| `t_doc_h2()` / `t_doc_h3()` | 19/26, 16/22 | semibold | Doc headings, read and edit |
| `t_prose()` | 15/24 | regular | Doc paragraphs/lists, measure ≤ 680 |
| `t_display()` | 28/32 | semibold, tabular | Large figures ("1,705", "$20") |
| `t_stat()` / `t_stat_sm()` | 22/28 semibold, 20/26 medium | tabular | Stat tile value / metric-strip value |
| `t_code(cx)` | 12.5/20 mono | regular | Diff and code lines, gutters (B measured 12.5/20; there is no 12/18 role) |
| `t_code_caption(cx)` | 12/16 mono | regular | ids, chapter number boxes |

`t_headline` (17/22) and `t_read_heading` (17/24) are deliberately different roles. Use the compact one for drawers and dialogs, and the reading one for prose pages.

**Read↔edit parity (checked in source):** Kit `Input` sets its own `line_height(LINE_HEIGHT)`, `input_h` and `input_text_size` *before* `refine_style(&self.style)` (`input/input.rs:853-876`). Styling the wrapper is therefore **not** enough: apply the role **on the `Input` itself** (e.g. `Input::new(&state).appearance(false).t_doc_title()`), plus the same role on the read wrapper. Button: the size branch (`button.rs:709-728`) runs before `refine_style(&instance_style)` (`:779`), so `.h()`, `.px()` and `t_*` on a Button win. Markdown `TextView` sizes headings from `TextViewStyle::heading_font_size`, not from inherited style. Use `TextView::markdown(..).style(doc_text_view_style())` inside a `t_prose()`/`t_read()` wrapper. Heading line heights are Kit's. Both modes share one left edge: the doc column has no extra editor padding.

## Canonical radii (`theme::metrics`)

| Const | pt | Use |
|---|---|---|
| `R_NUMBER` | 5 | Review chapter number boxes (measured `851174F7`) |
| `R_CONTROL` | 6 | Buttons, inputs, chips, rows, segment items |
| `R_INNER` | 8 | Board cards, segmented track, tab top corners, tooltips, diagram nodes |
| `R_POPOVER` | 10 | Generic popovers, menus |
| `R_STAT` | 6 | Stat tiles (measured `DB2A4628`: 151×66, gap 8) |
| heatmap | cell 11, gap 3, `R_HEATMAP` 2 | `DB2A4628` |
| `R_CARD` | 12 | Cards, panels, board trays, rail, content card |
| `R_SECTION` | 16 | Grouped settings cards (`section_card`; Hexta refs) |
| `R_DIALOG` | 14 | Dialogs |

`metrics::inner_radius(outer, inset) = max(outer − inset, 4)`. A flush child inside a 1px border uses `outer − 1` (`edge_band`). Where a reference measures a component-specific value, that value wins. Add a named const for it rather than normalising.

## Shared API (canonical, in source: `apps/client/src/{theme.rs,components/**}`)

```rust
// theme.rs
pub mod metrics { TITLEBAR_H, TAB_H, SIDEBAR_W, SHELL_INSET, GUTTER_X, GUTTER_TOP, READ_W,
  ROW_NAV, ROW_LIST, ROW_PROP, CONTROL_MD, CONTROL_SM, CONTROL_XS, CHIP_H,
  R_CONTROL, R_INNER, R_NUMBER, R_POPOVER, R_STAT, R_CARD, R_SECTION, R_DIALOG, HEATMAP_CELL, HEATMAP_GAP, R_HEATMAP; pub fn inner_radius(outer: f32, inset: f32) -> f32 }
pub fn tokens(cx: &App) -> Tokens // + accent_fill, accent_text, section_fill, board_column, board_card

// components::typography (re-exported: components::Typography)
pub trait Typography: Styled { t_title, t_headline, t_read_heading, t_section, t_body, t_read,
  t_label, t_trace_label, t_caption, t_micro, t_doc_title, t_doc_h2, t_doc_h3, t_prose, t_display, t_stat,
  t_stat_sm, t_panel_title, t_read_title (all fn(self) -> Self); t_code(self, &App), t_code_caption(self, &App) }
pub fn tabular() -> FontFeatures
pub fn doc_text_view_style() -> TextViewStyle

// components (surfaces)
pub fn card(cx: &App) -> Div
pub fn section_card(cx: &App) -> Div
pub fn settings_row(title: impl Into<SharedString>, description: Option<SharedString>, control: impl IntoElement, cx: &App) -> Div
pub fn row_divider(cx: &App) -> Div
pub fn section_label(label: impl Into<SharedString>, cx: &App) -> Div   // page section label ("Startup")
pub fn group_label(label: impl Into<SharedString>, cx: &App) -> Div     // sidebar group label
pub fn page_header(title: impl Into<SharedString>, subtitle: Option<SharedString>, cx: &App) -> Div
pub fn page_body(reading: bool) -> Div
pub enum Edge { Top, Bottom }
pub fn edge_band(edge: Edge, outer: f32) -> Div
pub fn clip_inset(radius: f32) -> Pixels
pub fn popover_card(cx: &App) -> Div
pub fn popover_header(title: impl Into<SharedString>, detail: Option<SharedString>, cx: &App) -> Div
pub fn stat_tile(value: impl Into<SharedString>, label: impl Into<SharedString>, cx: &App) -> Div // R6, t_stat + t_caption

// components (controls)
pub fn button_md(Button) -> Button   // 28, 13pt, Kit radius 6 (generic outlined/compact, `769AF488`)
pub fn button_row_action(Button) -> Button // 28, radius 10: settings-row/banner actions ("Open →", black "Upgrade", `92D96623`)
pub fn button_pill(Button) -> Button // 32, fully rounded: dialog footer CTA ("Use template", `85F1DC37`)
pub fn button_sm(Button) -> Button   // 24, 13pt
pub fn button_xs(Button) -> Button   // 20, 12pt
pub fn icon_button(Button, size: f32) -> Button
pub fn input_md(Input) -> Input      // 28, 13pt
pub fn input_lg(Input) -> Input      // 32, 13pt
pub fn textarea_box(state: &Entity<TextareaState>, cx: &App) -> Div // bordered Textarea frame (R6, 10/6 padding, 13pt)
pub fn count_badge(count: Option<usize>, cx: &App) -> Option<Div>
pub fn chip(label: impl Into<SharedString>, tone: Tone, cx: &App) -> Div
pub fn status_pill(label: impl Into<SharedString>, tone: Tone, cx: &App) -> Div
pub fn list_row(id: impl Into<ElementId>, selected: bool, cx: &App) -> Stateful<Div>
pub struct Segment { label, count }  // Segment::new(label).count(n)
pub fn segmented(id: impl Into<SharedString>, items: Vec<Segment>, selected: usize,
  on_select: impl Fn(usize, &mut Window, &mut App) + 'static, cx: &App) -> Div
pub fn meter(segments: usize, filled: usize, tone: Tone, cx: &App) -> Div
pub fn file_row(id: impl Into<ElementId>, path: &str, adds: usize, dels: usize, selected: bool, cx: &App) -> Stateful<Div>
pub fn file_path_label(path: &str, suffix: Option<SharedString>, emphasized: bool, cx: &App) -> Div // dir secondary + basename + optional ":12-30"
pub fn change_counts(adds: usize, dels: usize, cx: &App) -> Div // +n −n mono caption
pub fn status_dot(tone: Tone, cx: &App) -> Div
impl Tone { pub fn color(self, cx: &App) -> Hsla; pub fn tint(self, cx: &App, alpha: f32) -> Hsla }
pub fn vertical_label(text: impl Into<SharedString>, color: Hsla, window: &mut Window) -> impl IntoElement
// unchanged: empty_state, error_state, loading_state, toast_warning, toast_error, feature_icon
```

**`vertical_label` font stack:** `'SF Pro Text', 'SF Pro', 'Helvetica Neue', Helvetica, sans-serif` at weight 600, 13pt. usvg reads the system SFNS only as a regular "System Font" with no weight axes, so the installed `/Library/Fonts/SF-Pro-Text-Semibold.otf` supplies the real 600 face. The SVG is authored at the window scale factor because GPUI rasterises SVG images at 1×. There is no letter-stack fallback. The glyph result is checked natively in the final batch.

## Heading tiers and variants (B, final)

- Tiers relative to body: doc H1 ≈1.85× (28 on 15 prose), review page title ≈1.4× (`t_read_title` 20/26 for the review tour title; `t_title` 22 for settings-style pages), chapter/section ≈1.2–1.3× (`t_read_heading` 17), **panel/popover title = row size, semibold** (`t_panel_title` 13/18).
- Doc headings: 24 above, 8 below (asymmetric). The owner applies this as the heading block's margin, with one owner per gap.
- Home (`DFCD…`): an outer widget surface plus an inset card layer. This is a distinct variant; don't flatten it into `card()`. Compose it from `section_fill` (outer, R12) and `card` (inner). Stats/home owners build it locally from these tokens.
- Trays and popovers are different variants. `popover_card` is the generic floating one. Run-state trays (`ADFBCD30`) are owned by the feature and use instant swaps.
- Stat meta: measured 11, but user legibility wins, so use `t_caption` 12.
- Motion module API: `motion::{FAST, TAB, POPOVER, STANDARD, fade(), tab(), popover(), standard()}`.

- Buttons: shape follows the reference per use. Don't flatten everything to R6. Helpers only set the radius when they name a shape, and callers may keep an explicit `.rounded(..)`.
- `theme.mono_font_size = 12.5`: Kit's code editor and diff read it directly.

## Tab join and strip mechanics (authoritative)

- GPUI's `Style::overflow_mask` clips y to the element bounds even when only x scrolls (`gpui-pre style.rs:672-676`). The tab strip therefore extends 1pt into the card row (`bottom(-1)` with `pb(1)`). This is intentional: it includes the 1pt join in the strip's paint area, and it is not a mask.
- Active-tab stroke: GPUI paints a div's border after its children, so the tab never carries its own side border. A child owns top/side strokes and stops at y 33 (7pt above the tab bottom), where each R8 flare (y 33–41, including the 1pt card-border overlap)'s curved stroke continues it into the card's top edge. No straight vertical tail beside the curve and no hairline under the tab.
- The shell lays out `flex_col_reverse` (body, then titlebar), so the titlebar paints above the card's top border. The active tab's 1pt canvas cover and its 8pt flares are ordinary children; `deferred` is not used.
- The reveal stays pending in `StripMemory` until the layout is measured and the tab is at full width. A frame is requested while it is pending or while the scroll extent changes.
- Overflow edge: arrow 24, then opaque 8, then fade 24 (`EDGE` 56). Reveal and the connected check keep the tab and its flares out of this zone.

## Icon roles

- 16: chrome and sidebar (titlebar buttons, nav rows). 14: controls and toolbars (24pt icon buttons, tab icons, file rows). 12: inline and disclosure (tab close, chevrons, 20pt buttons). Kubai feature art at 24–28 is a separate, intentional exception.
- `icon_button(button, hitbox)` sets the glyph from the hitbox (≥28 → Medium 16, ≥24 → Small 14, else XSmall 12) and the hitbox with `w`/`h`. Never use Kit `with_size(px)` on icon buttons: that renders the glyph at 0.75× the hitbox (`button.rs:659`).
- Alignment: `icon_button` gives a square hitbox with `p_0` and Kit's centred content, so glyph centres coincide. Titlebar leading/trailing controls, overflow arrows and the traffic lights (14, 13) centre on the full 40pt bar (y 20; measure against the whole bar, not icon to icon). Tab faces (pill, icon, label, close, hover, ghost, divider) share the same centre 20; the 8pt foot below the face never counts toward centring. Sidebar rows lead with 16pt icon boxes and 8pt gaps. Child rows indent 32, so their icon sits under the project dot and their text under the project name.
