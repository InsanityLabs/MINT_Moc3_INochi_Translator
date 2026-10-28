# model-reaper

It is a live2d extractor to rip the psd out of moc3 files IT IS NOT USING ANY CODE FROM LIVE2D

Goal: public, open-source tool that distills compiled VTuber models back into
editable files — layered **PSD**, **.cmo3** (Cubism Editor), and **.inp/.inx**
(Inochi2D). Prior art (Lincubator, Eviler.Izabel's extraction core) was never
fully released; this project is the public implementation.

we are not lawyers and we dont want to get sued. we are just trying to make a tool that works and is free for everyone to use. please do not rip apart public live2d models or try to use this tool to extract commercial models. please use this on models you own or have <underline>PERMISSION</underline> to decompile!!!!!

we do not condone the use of this tool for any illegal or unethical purposes. we are not responsible for any damage caused by the use of this tool. if you use this tool illegally or unethically, you are on your own and we are not responsible for any damage caused by your actions. you are solely responsible for your actions if you get sued.

this is a work in progress and is not yet ready for public use. please bear with us while we work on it. 

## Status (2026-10-27) (these files are on the development machines right now for testing. please bear with us while we talk to the lawyers for a real release. it is a joke but really though we have to make sure we make no mistakes here for this shit to actually be released. we are not lawyers and we dont want to get sued. we are just trying to make a tool that works and is free for everyone to use.)

Working and verified on all four official Live2D sample models + 8 custom models made by us!!!!
(`testdata/{Mark,Haru,Hiyori,Ren}` — moc3 format versions 1, 3, 3, 6):

- `crates/reaper-core/src/moc3.rs` — clean-room moc3 parser. Section offset
  table = one u32 pointer per *field array* (not per element). Layout verified:
  `0x40` → countInfo ptr, `0x44` → canvasInfo ptr, `0x48` → field pointers.
  `runtimeSpace` fields are N*8-byte blocks. Indices like `uv_begin` and
  `position_sources_begin` are in **float units** (divide by 2 for element
  index). Keyform position arrays are 64-byte aligned individually.
- `crates/reaper-core/src/model.rs` — default-pose evaluation: parameter→
  keyform weights (mixed-radix, first bound param varies fastest), warp
  (normalized [0,1] grid coords, bilinear if quad_source else triangular,
  parallelogram extrapolation outside) and rotation (`origin+R(base+angle)·scale·v`)
  deformers, deformer-opacity chains (zero opacity is meaningful — do NOT
  guard it), draw-order-group sort (objects sorted by interpolated draw order
  within group, part objects reserve space for their owned group).
- `crates/reaper-core/src/raster.rs` + `render.rs` — software triangle
  rasterizer w/ UV bilinear sampling, clipping masks, add/multiply/normal
  blends. `render` produces pixel-correct composites of all 4 samples.
- `crates/reaper-core/src/emit.rs` — per-mesh canvas-space layers grouped by
  part tree, ordered by render position.
- `crates/reaper-core/src/emit/psd.rs` — real layered PSD v1 writer (PackBits
  RLE, luni names, lsct dividers). Group encoding: file order topmost-first,
  group = `[bounding divider][children top-first][folder record]`.
- `crates/reaper-core/src/emit/ora.rs` — OpenRaster export.
- `reaper-cli`: `info|parts|meshes|params|deformers|dumpverts|render|export`.

## Binding model (verified 198/198 objects on Ren)

`object.keyform_binding_source` → `keyform_bindings[k]` → `(begin,count)`
range of `parameter_binding_indices` → `parameter_bindings` → keys range.
Parameter of a pb = inverse of `param.binding_sources_begin/count` (which
directly indexes parameterBindings). Object keyform count = product of bound
pbs' key counts.

## Known issues / TODO

- PSD: a few stray `</Layer group>`/0x0 records on some branches (group
  open/close pairing needs a final polish pass); masked meshes (EyeL1/EyeR1
  in Mark) currently skipped — likely produce empty layer via mask path.
- Release-profile rasterizer needed (debug build renders Ren in ~2min).
- Not yet: cmo3 output (dep `moc2cmo` crate, MIT), .inp transpile, egui GUI
  (`reaper-gui` crate is a stub), expressions/motions, v5+ keyform colors,
  glue deformers, VTS-encrypted models (out of scope).

## Prior art / references

- OpenL2D moc3ingbird `moc3.hexpat` — format spec (kept in `testdata/`)
- MahouTechnologies/moc3-rs — deformer + draw-order reference implementation
- Eatgrapes/moc2cmo (MIT) — moc3→cmo3 (CAFF archive) reference
- Inochi2D/inpunpack + INP spec — trivial container, official tooling
- goldimpact/Eviler — published only psd.py (packing stage); extraction core
  was never public. UlyssesWu/Lincubator — private paid moc3→cmo3 service.

## Legal

Clean-room: parses the documented binary format, uses no Live2D SDK code or
decompiled material (unlike py-moc3/moc3-reader-re). Intended for recovering
your own models / interop. Model copyrights belong to their owners.
