# GPUI foundation (2026-11-24)

Status: the client shell compiles (`cargo build -p karabiner-client`). For the frozen feature interface see `client-interface.md`. Visuals follow `visual-contract.md` (`spatial-references-a.md`/`-b.md` are research notes only).

## Toolchain and versions
- `rust-toolchain.toml` pins **Rust 1.100.0**. `gpui-pre-macos 0.3.14` uses `Option::map_or_default`, which fails on 1.97.1 with E0658. Zed pins 1.100.0 too ([zed rust-toolchain.toml](https://raw.githubusercontent.com/zed-industries/zed/main/rust-toolchain.toml)). The pin is project-local; the global default stays stable 1.97.1.
- `gpui-kit = "=0.11.0"` pins GPUI to the `gpui-pre =0.3.14` snapshot ([v0.11.0 Cargo.toml](https://raw.githubusercontent.com/longbridge/gpui-kit/v0.11.0/Cargo.toml)). Use GPUI only through `gpui_kit::*`. Do not use the git-based snippet on the Base Getting Started page (it tracks `main`).
- HTTP: `reqwest_client = { package = "gpui-pre-reqwest-client", version = "=0.3.14" }` (`ReqwestClient`). No read timeout: reqwest's body timeout needs a Tokio reactor on the polling thread and panicked on GPUI's executor. The default native GPUI HttpClient fails every request.
- Xcode 27.2 with `metal`, and Bun 1.3.14, are present.

## Kit APIs in use / verified in source
- Boot: `application().with_assets(assets::Assets).run(|cx| { init(cx); open_window(opts, cx, |w, cx| cx.new(..)) })`; `WindowBounds::centered(None, size, cx)`.
- Tabs: `component::tab::{TabBar, Tab}`. `TabBar::max_width` (Tab's own is private), `.menu(true)`, `.selected_index`, `.on_click(Fn(&usize, &mut Window, &mut App))`, `Tab::suffix`. `Button::selected` needs `component::Selectable`.
- Motion (`gpui_kit::base::motion`): `transition`, `TransitionState`, `spring`, `Keyframes`, `Presence`, `Stagger`, `Sequence`, `MotionReveal` ([docs](https://gpui-kit.com/base/motion/)). Reduce Motion is read from macOS in `init`.
- Also available: `NavStack`, `InputState`/`Input`, `Editor` (syntect/tree-sitter features), `Popover`/`Dialog`/`Sheet`/`Toast`, `Table`/`DataTable`/`VirtualList`/`Tree`/`Plot`, GPUI `.on_drag/.drag_over/.on_drop`, `img()` for full-colour SVG (Kubai), `svg()` for mono icons.

## Transport (agreed)
Loopback HTTP JSON `/v1`, default `http://127.0.0.1:43173` (override with `KARABINER_SERVICE_URL`). `GET /v1/meta` must report `apiVersion == 1`, otherwise an "Incompatible service" banner shows. The service also offers SSE at `/v1/events`, but the client does not subscribe; refresh semantics are in `client-interface.md`. `crates/protocol` hand-mirrors `packages/contracts/src/index.ts`. Tabs persist through `/v1/workspace`; there is no client-local storage.

## Verification tools
- Headless: kit `test-support` (`VisualTestContext`, `Window::render_to_image`, `HeadlessAppContext::capture_screenshot` via Metal).
- Native: `screencapture -x -l <windowid>`, `osascript`, `cliclick`. `agent-device`, `peekaboo` and `orca` are absent. Driving the native window needs Screen Recording and Accessibility permission (not checked or changed).

## Commands
```sh
cargo build -p karabiner-client
cargo run -p karabiner-client          # runs without the service (unavailable banner)
zsh .groundwork/scripts/client-smoke.sh # native journey on an isolated service; evidence in .groundwork/tmp/client-smoke
cargo test -p karabiner-protocol
```
