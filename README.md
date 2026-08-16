# MSDF Font Explorer

A compact Rust sample that shapes a curated Lorem Ipsum page with
[HarfRust](https://crates.io/crates/harfrust), generates a multi-channel signed
distance field atlas with
[bymsdfgen](https://crates.io/crates/bymsdfgen-core), and renders every glyph in
custom `wgpu` shaders.

The sample is Windows-first and defaults to Arial, while `--font` accepts either
an installed family name or a font-file path. Font collections use face zero
when selected by path.

## Run

Rust 1.95 or newer and a graphics adapter supported by `wgpu` are required.

```powershell
cargo run --release
cargo run --release -- --font "Segoe UI"
cargo run --release -- --font "C:\Windows\Fonts\arial.ttf"
```

Use `cargo run -- --help` to see the command-line interface. On a system without
Arial, pass an installed family or a `.ttf`, `.otf`, or `.ttc` file explicitly.

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

- `font.rs` resolves a system family through `fontdb` or loads a path directly.
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
```
