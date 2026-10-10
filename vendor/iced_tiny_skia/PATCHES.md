# iced_tiny_skia 0.14.1: local changes

- Base: crates.io `iced_tiny_skia` 0.14.1, part of [Iced](https://github.com/iced-rs/iced).
- License: MIT, Copyright (c) 2019 Héctor Ramón, Iced contributors. The crates.io package does not
  ship a license file, so `LICENSE` here is Iced's MIT license.
- The root `Cargo.toml` uses this copy through `[patch.crates-io]`.

Everything not listed below is unchanged from the published crate.

## Changes

1. **Damage collapsing** (`src/window/compositor.rs`, `collapse_damage`).
   When the grouped damage has more than four rectangles, or the rectangles cover more than 40 % of
   the viewport, they are replaced by their single bounding rectangle. Each damage rectangle repeats
   the layer and clip passes, so a scroll that dirties many small regions is much cheaper to repaint
   in one pass.

2. **Text clipping** (`src/engine.rs`, text drawing).
   Cached text is always clipped to its own viewport intersected with the damaged region. Upstream
   skips the clip mask when the text's viewport lies inside the damage clip, but the viewport does
   not bound where the glyphs land: with change 1 the damage can cover a whole scrollable, and text
   scrolled outside it would otherwise be painted over the surrounding widgets.

3. **Pixel snapping** (`src/engine.rs`, `snap_quad` and `snap_bounds`).
   Quads with `snap` set (all quads when Iced's `crisp` feature is enabled) are rounded to whole
   physical pixels, and non-zero borders to at least one physical pixel; rounded corners are left
   as they are. Raster image bounds are rounded the same way. This matches what the wgpu quad shader
   does, so at fractional scale factors 1 px outlines stay sharp and images line up with the
   outlines that frame them.

4. **Image placement** (`src/raster.rs`).
   Images are positioned by translating before scaling. Upstream divides the origin by the scale
   and truncates it to whole source pixels, which can offset an image by up to one source pixel from
   its bounds.

5. **Render counters** (`src/stats.rs`, new; called from `src/window/compositor.rs`).
   When the `CRABYTUBE_RENDER_STATS` environment variable is set, each presented frame prints its
   damage rectangle count, layer count and draw time to stderr. Without the variable nothing is
   printed.

6. **Image measuring** (`src/raster.rs`, `dimensions`).
   Upstream measures an image by decoding it into the raster cache, and the cache is trimmed only at
   the end of a draw. An RGBA handle already carries its width and height, so it is returned as is.
   Without this, a window that is not drawn (minimized) keeps one decoded copy of every cover laid out.
