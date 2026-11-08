# Architecture

How the native Codex Desktop in Rust is split into crates.

| Crate | Role |
|---|---|
| `bridge` | JSON-RPC client for `codex app-server` over stdio or durable cloud WebSocket, protocol types, recorded sessions, and the ACP backend. |
| `ui` | The window shell (titlebar, navigation rail, sidebar, main panel), the page modules in `screens/`, the composer, the thread view, the diff view and the icons. |
| `shapes` | Box decorations that land on Chromium's pixels: rounded and squircle corners, borders, `box-shadow`, backdrop blur, SVG icon paths and circles. It links Skia. |
| `textsys` | A GPUI text system that puts glyphs on the same pixels as Chromium on macOS. It also holds the vendored `gpui` and `gpui_apple`. |
| `theme` | Color tokens, metrics and the bundled fonts, extracted from the original renderer CSS. |
| `parity` | The pixel-parity harness: it renders each screen headless and compares it with the reference screenshot. |
| `app` | The `codex-gpui` binary: the root view, the menu bar and the app-server session. |

App routing lives in `crates/app/src/routes/`.
Home, Thread, Settings, Automations, Mobile, and Archive each own their action registration and page activation there.
`routes/mod.rs` keeps the route enum, family registration, navigation history, and shared shell updates.
`app::Route` and its saved values stay unchanged.
Shared RPC handlers and state remain on `Root` in `lib.rs`.
`Root::first_app_connected` lets app-list refresh callers pass the previous list and the new list.
It refreshes default and science ambient suggestions only when a known list gains its first enabled, accessible app.
The host requests run off the UI thread; a failed request does not stop the other domain.
The plugins slice owns the caller; this hook does not change the plugins page.
Run `cargo test -p app --test first_app_connected -- --test-threads=1` on macOS for the native hook checks.

Opening an unloaded chat reads its history and resumes the thread before showing it.
The composer restores that thread's returned model, reasoning effort, and permissions instead of using startup defaults.
An unset effort stays unset in requests; the picker shows Medium.
The composer labels modified workspace policies as Custom.
A titled thread shows its header identity, also in a single pane.
An untitled thread shows it only when a right pane is open; the toolbar actions stay available.

In a loaded chat with an open content tab, `Cmd+Shift+F` switches between full and split view.
`Cmd+Shift+B` changes full view to split view, hides the tabs, or restores the retained tabs beside Chat.
Hiding tabs keeps their selected content; opening the same file, terminal, or Changes tab shows it again.
Full view focuses the content pane and does not send typed text to the hidden composer.
Find in full view searches the visible Changes panel, not the hidden chat.
These commands do not create a browser tab when no content tab is open.
Run `cargo test -p app --test workspace_layout -- --test-threads=1` for the native interaction checks.

`LocalHost` serves `native-desktop-apps` and `native-desktop-app-by-bundle-id` through the desktop fetch boundary.
On macOS, the adapter reads Spotlight metadata, applies the original app exclusions, and supports usage or display-name ordering.
It returns each app's path, bundle ID, and display name without launching the app.
Linux returns an empty native app list, as the original host does.

On macOS and Linux, `workspace-directory-entries` lists workspace children with relative paths and directories first.
`includeHidden` and `directoriesOnly` filter the reply.
Directory requests reject lexical and symlink paths outside the workspace.
Enumeration uses the validated directory handle, so replacing a symlink cannot redirect a listing.

`add-context-file` sends a file reference to the host's bound view.
The composer shows a file card without changing the draft text or selection.
Repeated context files stay attached once; replacing the host disconnects its previous context channel.
Submission lists these paths in the original `Files mentioned by the user` section, separate from image uploads.
Editing a queued message restores its file cards and request text.
Failed thread submissions restore the request and file cards.

`open-in-targets` returns the original editor and terminal catalog, installed targets, and the preferred target.
It reads `open-in-target-preferences` from the same shared host settings store as the UI.
Saving one project's preference preserves entries for other projects.
It reads `custom_file_handlers` from enabled raw app-server configuration layers because the typed configuration omits this desktop setting.
Set `deferEnrichment` without a path to read the catalog without application detection.
Path queries include native file handlers and their icons; discovery does not launch an application.
Alacritty is a Linux fallback for the `terminal` target, not a separate target ID in this original build.
Cloned `host_settings::Settings` handles share updates; release `read_all()` guards before writing.

`open-in` launches a selected target and returns its success state.
Local paths must resolve inside the project, including symlink targets.
Existing relative paths take precedence over Git diff prefixes.
Editor locations and terminal commands follow the original platform rules.
Custom handlers receive a path argument, a JSON argument, or JSON on standard input.
The native UI reports failed launches and keeps the current diff.
Command text sent to an interactive terminal rejects control characters.
ACP connections use built-in targets without requesting Codex-only configuration.

On macOS, `textsys::show_definition(window, selection, baseline)` opens AppKit's native Look Up popover.
The baseline uses window-local logical pixels from the top left.
The adapter applies window magnification and handles both view orientations.
Call it on the main thread with the unchanged selection.
Headless windows return their native-handle error; the adapter has no URL or clipboard fallback.

On macOS, `TextInputConfiguration::text_substitutions` enables native text substitutions for an editable field.
The composer enables this option only for non-secret text.
Edit > Substitutions controls smart quotes, smart dashes, and text replacement for the window.
`NSSpellChecker` supplies the edits; the adapter preserves UTF-16 ranges and the caret.
Each check requests up to 512 UTF-16 units of context on either side of the accepted edit.
`cargo run -p app --example native_substitutions` checks the input path in a hidden native window.
The check uses process-local replacement options without changing shared OS settings or activating the app.

On macOS, `SystemNotification::reply_placeholder` adds a text-input notification action.
Register `App::on_system_notification_reply` before posting; it receives the notification identifier and reply text on the main thread.
Without a registered handler, the notification omits the reply field.
The local `vendor/gpui_macos` patch tests native request and category fields without posting a notification.

With `test-support`, `HeadlessAppContext::did_prompt_for_paths` reports a pending path dialog.
Use `simulate_path_prompt_response` to answer it with selected paths or `None` for cancellation.
These hooks use `TestPlatform` without opening an OS dialog; normal app behavior does not change.

`gpui` and `gpui_apple` come from Zed at a fixed revision and are vendored in `crates/textsys`.
The workspace `[patch]` replaces the upstream crates with these copies.
Stock GPUI cannot match Chromium's pixels. The local copies include these changes:

- Blink snapping: layout in Blink `LayoutUnit`s (1/64 device px), device-pixel rounding half up, and text at fractional x positions.
- Skia blends: glyph and sprite blending as Skia's 8-bit raster does it, and `SkPaint::setAlphaf` alpha rounded to a byte.
- Layer compositing: opacity layers and composited layers (`will-change`, scrollers) that blend as Chromium's `saveLayerAlpha` does.

RGB-to-HSL conversion keeps saturated sRGB colors at saturation 1.0.
This keeps their edges on Skia's legacy raster path instead of the float path.
Comment cards clip their contents to the rounded card shape while their shadows remain outside the clip.
Clipped deferred draws retain their ancestor's raster recording origin without creating a new opacity layer.
Comment attachments use Skia for the rounded border and fill, including the hover state.

`Window::layout_origin_residual_y` returns the fractional vertical origin removed by layout rounding.
Use it when placing multiline field glyphs to retain Blink's baseline positions.

`Window::set_ui_scale` scales layout, painting, pointer input, and IME geometry without resizing the native window.
Text layouts use each window's effective device scale and discard stale cached metrics when that scale changes.

Inline-code backgrounds use both snapped label edges, so fractional text widths do not expand their right edge.
Their height and position follow the active code font's metrics.
The thread's top fade preserves content that scrolls below it.

Monochrome glyphs and SVGs keep their paint alpha.
Element opacity is applied once when the enclosing layer is restored.

Native sprite state retains four nested layer devices within Metal's eight color targets.
Each device restores into its parent before a parent opacity mask is applied.
Deeper alpha is combined in the innermost device.

Character-unit widths use the raw glyph advance at the requested font size.
They exclude tracking; text layout still includes it.
Use `TextSystem::ch_advance` for CSS `ch` columns, not glyph ink bounds.

Canvas callbacks honor their `opacity`, `layer`, and `top_layer` styles.
These styles use the same compositor boundary as other elements.

Linear gradients interpolate premultiplied colors in the selected color space.
Their ordered RGB dither leaves alpha unchanged, so transparent fades do not add opacity noise.
Gradients inside compositor layers apply dithering after blending and retain element opacity and the unsnapped gradient origin.

Home composer permission and model labels paint directly into their parent.
Docked composers retain the composited label path. The animated effort label keeps its filter layer.
An extra layer changes glyph edge bytes even when its opacity is one.

`shapes` links Skia binaries built with Chromium's `SkUserConfig.h` legacy defines.
`.cargo/config.toml` points `SKIA_BINARIES_URL` to these binaries.
The rasterizer reuses identical inverse-blend probe results across icon positions without changing their pixels.
`skia-hairline-sys` links a narrow C scanner wrapper for the Skia revision in `skia.toml`.
It records the ordered legacy hairline operations that the sprite renderer replays.
`.cargo/config.toml` points `SKIA_HAIRLINE_BINARIES_URL` and `SKIA_HAIRLINE_PACKAGE_SHA256` to its versioned package.

Raster output buffers stop at the visible viewport.
Compositor tiles keep their original size, so long scrollers preserve edge rasterization.

On macOS, popup backdrops capture the current frame's scene before the popup.
The capture includes text, images, clipping, and opacity.
Nested filters use the parent layer's content, not its already-filtered backdrop.

`shapes::raster_images` decodes to the smallest mip that covers the rounded target before bilinear sampling.
Callers choose direct composition, a saved image group, or a transparent compositor tile to match the original layer.
An image group keeps its save-layer even when it contains one image, so clipped sampling retains the original interpolation.
Transparent image groups retain straight-alpha pixels for GPUI, so the background and shadow remain visible through rounded corners.
Use `ImageComposition::Composited` with an opaque background for compositor tiles.
Rasterize these images during paint, when the layer context is available.

Wrap a sparse viewport layer's decorated subtree in `ui::recorded_bounds`.
It collects painted bounds, including shadows, without adding an opacity layer.
The Toaster uses this scope to match Chromium's shifted tile grid and local corner geometry.
The thread titlebar reports its icon and divider bounds so the tile grid follows painted content, including untitled states.

Wrap a tightly bounded compositor layer in `ui::composited_bounds`.
It keeps the exact shadow bounds and uses Chromium's dense tile sizes.
The tooltip uses this scope around its own `top_layer`.
`focus::shadow_ring` paints outer spread shadows. `focus::ring` retains the outline path.
Use `focus::inset_outline` for CSS outlines with negative offsets.
Pass the CSS offset directly; `px(-2.)` keeps a 2px outline inside the control.
The helper keeps a bounded local recording while keyboard focus is visible.

For a composited layer recorded in window coordinates, add `.window_space_recording()` to its container.
Keep `.cc_tiles()` when the original layer rectangle sets the tile size.
Paths retain their window coordinates; the tile grid remains relative to the layer.
Nested composited layers keep their own recording space. Saved layers inherit it; sparse recordings retain their local origin.


