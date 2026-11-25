# Kubai Icons (vendored subset)

- Source: npm `kubai-icons-static@0.15.0` (https://icons.kubai.design, https://github.com/kubai087/kubai-icons)
- License: MIT. See `LICENSE`. Keep it with these files.
- Subset: 35 icons, rest pose only (the hover motion ships only in the JS packages).
  - `svg/` has the colour variant (accent `#5393FF`).
  - `svg-soft/` has the neutral blue-grey variant, for the light-neutral UI.
- The files are full colour, with gradients and a drop-shadow filter. Render them with GPUI `img()`, not `svg()`. `svg()` paints a single-colour mask and would flatten the tones.
- Each viewBox includes margin for the shadow, so draw the files at 28px to match a 24px glyph. They are for feature-level icons (sidebar sections, empty states, template tiles, tab leading icons ≥20px). For tiny chrome glyphs such as plus, close and chevrons, use the UI kit's icons.
- To add an icon, copy it from the same pinned package version. Don't take files from the website, which is all rights reserved.

Icons: activity, book-open, bot, calendar-clock, chart-bar, circle-check, clock, code, file-diff, file-plus, file-text, folder, folder-open, git-branch, git-commit, git-compare, git-merge, git-pull-request, history, home, inbox, kanban, layout-dashboard, list-checks, message-square, pause, play, route, search, settings, sparkles, terminal, timer, users, workflow.
