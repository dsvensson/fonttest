# MSDF Font Explorer

A compact Rust sample that shapes a romanized Klingon adaptation of Lorem Ipsum with
[HarfRust](https://crates.io/crates/harfrust), generates a multi-channel signed
distance field atlas with
[bymsdfgen](https://crates.io/crates/bymsdfgen-core), and renders every glyph in
custom `wgpu` shaders.

The default face is Playfair Display. On desktop, `--font` accepts an installed
family, a Google Fonts family, or a font-file path. Installed families win;
otherwise the app downloads the regular face from Google Fonts and keeps a copy
in the operating system's application cache. Font collections use face zero
when selected by path.

## Run

Rust 1.95 or newer and a graphics adapter supported by `wgpu` are required.

```powershell
cargo run --release
cargo run --release -- --font "Segoe UI"
cargo run --release -- --font "Roboto Slab"
cargo run --release -- --font "C:\Windows\Fonts\arial.ttf"
```

Use `cargo run -- --help` to see the command-line interface. A Google font needs
network access only on its first desktop load; later runs use the cached font.

## WebAssembly

The web build uses the browser's WebGPU implementation and fetches its font from
Google Fonts. Install [Trunk](https://trunkrs.dev/) and the Rust WASM target, then
serve the included `index.html`:

```powershell
rustup target add wasm32-unknown-unknown
cargo install --locked trunk --version 0.21.14
trunk serve --release --open
```

Build a deployable `dist` directory with `trunk build --release`. Select another
Google Fonts family through the query string, for example
`http://127.0.0.1:8080/?font=Roboto%20Slab`. Local file paths and installed system
fonts are intentionally unavailable inside the browser sandbox. The initial
page load needs network access to Google Fonts; the browser may cache subsequent
requests.

### GitHub Pages

The `Deploy WASM to GitHub Pages` workflow builds the Trunk site and deploys its
HTML, JavaScript, and WASM files whenever `master` is pushed. Before its first
run, open the repository's **Settings → Pages** and select **GitHub Actions** as
the publishing source. The workflow derives the repository's Pages base path at
build time, so project pages such as `https://OWNER.github.io/REPOSITORY/` load
their generated assets from the correct location. It can also be started
manually from the Actions tab.

## Controls

| Input | Action |
| --- | --- |
| Mouse wheel | Continuously zoom at the mouse cursor |
| Left-button drag | Pan an axis when the zoomed page is larger than the window |
| `0` or numpad `0` | Reset zoom and pan |

Zoom is logarithmic and has no discrete levels. The camera uses `f64` world
coordinates and only stops at distant numerical-safety limits. Visible glyph
tiles are clipped and translated to screen-relative coordinates before upload,
keeping GPU coordinates stable during deep zooms.

## Rendering pipeline

- `font.rs` resolves a desktop system family or path, downloads/caches Google
  fonts, and decodes browser-delivered WOFF2 fonts for the WASM build.
- `text.rs` shapes every line with HarfRust and performs Unicode-aware wrapping.
- `atlas.rs` extracts outlines and builds a padded RGBA MSDF atlas with
  `bymsdfgen-core` and `bymsdfgen-io` once at startup.
- `gpu_text.rs` streams visible instanced quads to `wgpu`; `msdf.wgsl` reconstructs
  coverage from the median RGB distance.
- `camera.rs` owns cursor-anchored zoom, DPI-aware reset, resize behavior, and
  edge-constrained panning.

The shader uses the same distance value to produce crisp antialiasing, gradient
fills, independent outlines, colored glow, and offset soft shadows. These
effects remain resolution-independent because they are thresholds over the
MSDF rather than pre-baked bitmap decoration.

## Verify

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
cargo check --target wasm32-unknown-unknown --lib
```
