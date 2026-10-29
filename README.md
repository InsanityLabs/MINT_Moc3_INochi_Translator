# MINT

**Moc3 INochi Translator** — a recovery and translation tool for compiled
Live2D models. MINT reads `.moc3` files and turns them back into editable
projects: layered **PSD** (Krita/Photoshop), **OpenRaster (.ora)**, Cubism
Editor **.cmo3** (plus .can3 animation files), and Inochi2D **.inp/.inx** —
with meshes, masks, hierarchy, parameters, keyforms and draw order preserved
as far as the source format allows.

MINT is a clean-room implementation. **It does not use any code from Live2D.**

## Please read before using

This tool is intended for recovering your own work — models you own or have
explicit **permission** to decompile. Do not use it to rip apart published or
commercial models that are not yours. We are not lawyers, and we are not
responsible for what you do with it.

## Outputs

| Format | Opens in | Notes |
|---|---|---|
| `.psd` | Photoshop, Krita | Real layered PSD v1; part hierarchy as layer groups, render-order z-stacking |
| `.ora` | Krita, GIMP | OpenRaster with the same group hierarchy |
| `.cmo3` | Cubism Editor 5.3 | Reconstructed project: parts, warps, rotations, parameters, keyforms, masks, textures |
| `.can3` | Cubism Editor 5.3 | Animation file generated when motion data is present |
| `.inp` | Inochi2D runtimes | Runtime puppet with baked parameter bindings |
| `.inx` | Inochi2D Creator | Editable project (node tree, meshes, bindings, sidecar files embedded) |

## GUI

```
cargo run --release --bin mint-gui
```

Open or drop a `.moc3` onto the window. The app shows a preview of the model
at its default pose, runs sanity checks (missing textures and sidecar files,
degenerate meshes, hidden parts, version warnings), and exports the formats
you tick into `<model>_export/` (configurable). Loading and exporting run in
the background so the window stays responsive.

## CLI

```
mint-cli export <model.moc3> <out-stem> --all     # psd + ora + cmo3 + can3 + inp + inx
mint-cli render <model.moc3> <out.png>            # default-pose render
mint-cli info|parts|meshes|params|deformers|dumpverts|order|bindings|maskdump <moc3>
mint-cli cmo3info <file.cmo3>                     # version/metadata info
mint-cli cmo3dump <file.cmo3> <dir>               # unpack a cmo3 archive
```

Options:

- `--all` — include hidden / animation-only layers in PSD/ORA.
- `--cmo3-visible` — force everything visible in the `.cmo3` (breaks
  hair/clothing toggles; off by default).
- `export <moc3> <stem> --ora|--psd|--cmo3|--inp` — pick formats individually.

## Supported input

A `.moc3` alone is enough, but place the whole model set next to it for best
results: `model3.json`, textures, `cdi3.json` (restores parameter/part display
names), `physics3.json`, `pose3.json` (restores default-hidden parts like
alternate arm sets), expressions, motions and `vtube.json` — they get embedded
into the `.cmo3`/`.inp`/`.inx` where possible.

Verified on the four official Live2D sample models (Mark, Haru, Hiyori, Ren —
moc3 versions 1, 3, 3 and 6) and on custom models.

## Known limitations

- **Cubism 5.3 (v6) models:** blend modes and offscreen drawing are
  approximated. Parts using the "Out" alpha blend and pose-hidden parts are
  exported hidden by default; some mask lists stored in the new sections are
  not yet reconstructed.
- **Physics:** `physics3.json` is embedded into outputs, but editor-side
  physics objects in `.cmo3` are not fully reconstructed (schema undocumented).
- **Masks:** standard clipping masks are preserved; masks that live on
  offscreen parts (Cubism 5.3) are not yet.
- Expressions and motions are embedded as resources; they are not rebuilt
  into editor-native objects.

## Layout

- `crates/mint-core` — moc3 parser, pose evaluation, rasterizer, emitters
- `crates/mint-cli` — command line tool
- `crates/mint-gui` — egui app
- `vendor/moc2cmo` — cmo3 archive/XML builder (MIT, by Eatgrapes; modified)

## License

[PolyForm Noncommercial 1.0.0](LICENSE) — free to use, modify and share for
**noncommercial purposes only**. Commercial use requires a separate license.
