# MINT

**Moc3 INochi Translator** — a recovery and translation tool for compiled
Live2D models. MINT reads `.moc3` files and turns them back into editable
projects: layered **PSD** (Krita/Photoshop), **OpenRaster (.ora)**, Cubism
Editor **.cmo3** (plus .can3 animation files), and Inochi2D **.inp/.inx** —
with meshes, masks, hierarchy, parameters, keyforms and draw order preserved
as far as the source format allows.

MINT is a clean-room implementation. **It does not use any code from Live2D.**

[![Release](https://github.com/InsanityLabs/MINT_Moc3_INochi_Translator/actions/workflows/release.yml/badge.svg)](https://github.com/InsanityLabs/MINT_Moc3_INochi_Translator/actions/workflows/release.yml) [![Nightly](https://github.com/InsanityLabs/MINT_Moc3_INochi_Translator/actions/workflows/nightly.yml/badge.svg)](https://github.com/InsanityLabs/MINT_Moc3_INochi_Translator/actions/workflows/nightly.yml) [![CI](https://github.com/InsanityLabs/MINT_Moc3_INochi_Translator/actions/workflows/ci.yml/badge.svg)](https://github.com/InsanityLabs/MINT_Moc3_INochi_Translator/actions/workflows/ci.yml)

## Please read before using

This tool is intended for recovering your own work — models you own or have
explicit **permission** to decompile. Do not use it to rip apart published or
commercial models that are not yours. We are not lawyers, and we are not
responsible for what you do with it.

## Outputs

| Format | Opens in | Notes |
|---|---|---|
| `.psd` | Photoshop, Krita | Real layered PSD v1; part hierarchy as groups, render-order stacking, full unmasked art plus real layer masks, default-hidden layers/groups switched off |
| `.ora` | Krita, GIMP | OpenRaster with the same hierarchy and visibility; masks ship as hidden `<layer>.mask` layers (ORA has no layer masks) |
| `.cmo3` | Cubism Editor 5.3 | Reconstructed project: parts, warps, rotations, parameters, keyforms, masks, textures |
| `.can3` | Cubism Editor 5.3 | Animation file generated when motion data is present |
| `.inp` | Inochi2D runtimes | Runtime puppet with baked parameter bindings |
| `.inx` | Inochi2D Creator | Editable project (node tree, meshes, bindings, physics, tints, sidecar files embedded) |

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
- `--guess-pairs` — also combine non-standard X/Y parameter pairs (a guess; off
  by default).
- `export <moc3> <stem> --ora|--psd|--cmo3|--inp` — pick formats individually.

## Supported input

A `.moc3` alone is enough, but place the whole model set next to it for best
results: `model3.json`, textures, `cdi3.json` (restores parameter/part display
names), `physics3.json`, `pose3.json` (restores default-hidden parts like
alternate arm sets), expressions, motions and `vtube.json` — they get embedded
into the `.cmo3`/`.inp`/`.inx` where possible.

Verified on the four official Live2D sample models (Mark, Haru, Hiyori, Ren —
moc3 versions 1, 3, 3 and 6) and on custom models.

## Cubism 5.3 (v6) support

v6 models are rendered with their real data: per-mesh colour blends (linear
burn, multiply, screen, overlay ...), multiply/screen keyform colours,
offscreen parts (group buffers with the "Out" cut-out and the other alpha
blends), part-level offscreen masks and opacity. PSD/ORA layers carry the
blend modes (PSD blend keys / ORA composite-ops) and Inochi gets the nearest
Inochi blend mode.

## Known limitations

These come from what the target formats can express:

- **Offscreen / alpha blends in editable files:** PSD, ORA, Inochi and the
  `.cmo3` have no equivalent of Cubism's offscreen drawing or its Atop / Out /
  Conjoint / Disjoint alpha blends, so editable layers are exported as plain
  layers and "Out" cut-out parts are exported *hidden*. The merged preview and
  composite image do apply them.
- **Interleaved parts in PSD/ORA:** when one part's meshes are drawn both below and above another part's (e.g. neck skin under a jacket, scarf over it), the part is split into several groups (`Part (2)`, ...) so the layer stacking matches the model's draw order exactly.
- **Offscreen opacity** is applied per mesh rather than to the group buffer
  (identical unless meshes inside one group overlap while translucent).
- **Inochi physics** is an approximation: Live2D physics is mapped to Inochi
  `SimplePhysics` nodes (an anchor moved by the input parameters swings a
  pendulum that writes the output parameter). Inochi's solver differs from
  Live2D's, so the sway needs tuning per model.
- **Expressions** are separate `.exp3.json` files in Cubism (not part of the
  `.cmo3`), so they are copied/embedded as-is.
- **Parameters that do nothing** are a property of the source model: some have
  no keyforms in the moc3 (editor dividers, unused standard params, params only
  read by VTube Studio, physics outputs nothing is bound to). The GUI lists
  how many per model; dividers are dropped from the Inochi export.
- **2D (combined) parameters:** the moc3 does not record which parameters the
  author combined, so MINT only combines Cubism's standard-template pairs when
  they sit next to each other (Angle X/Y, Eyeball X/Y, Mouth Form/Open Y).
  `--guess-pairs` (GUI: "Guess extra 2D parameter pairs") also combines other
  `...X`/`...Y` pairs by name; usually right but can be wrong.

Not yet verified against Cubism itself: output types other than "Angle" in the
`.cmo3` physics settings, and the `.can3` motion files (laid out like a
Cubism-authored `.can3`, one scene per motion).

## Updating

The GUI checks the project's GitHub releases at startup and offers **Update
now**, which replaces `mint-gui.exe` (and `mint-cli.exe` next to it) and offers
a restart. In About you can switch the **update channel** (Stable / Nightly) or
turn the startup check off.

## Platforms

MINT builds for Windows (x64), Linux (x64) and macOS (Apple Silicon and Intel).
Each release has one zip per platform (`MINT-windows-x64-...`, `MINT-linux-x64-...`,
`MINT-macos-arm64-...`, `MINT-macos-x64-...`) holding `mint-gui` and `mint-cli`,
and the in-app updater downloads the matching one.

- **Linux:** needs a desktop with GTK 3 (file dialogs) and OpenGL. To build from
  source: `libgtk-3-dev libxkbcommon-dev libwayland-dev libxcb-render0-dev
  libxcb-shape0-dev libxcb-xfixes0-dev libx11-dev`.
- **macOS:** the binaries are not code-signed, so Gatekeeper blocks the first run.
  Allow it in System Settings -> Privacy & Security, or run
  `xattr -d com.apple.quarantine mint-gui mint-cli`.
- **Windows:** SmartScreen may warn about the unsigned exe (More info -> Run anyway).

## Builds and releases (GitHub Actions)

| Channel | Trigger | Release | Who gets it |
|---|---|---|---|
| CI | every push / PR | none (build + tests only) | - |
| Nightly | daily 06:00 UTC if `main` changed, or run manually | a new pre-release per build (`nightly-<date>-<time>-<commit>`); the newest 10 are kept | users on the Nightly channel |
| Stable | push a tag like `v1.2.0` | a normal release (`v1.2.0-rc1` = pre-release), kept forever | everyone on the Stable channel |

Cut a stable release:

```
git tag v1.2.0
git push origin v1.2.0
```

The workflow stamps the version from the tag, builds every platform, packs a small zip per platform
(`python scripts/package_release.py`, run it locally too), and creates the release with
generated notes (or `RELEASE_NOTES.md` if present). The release title always ends
with the short commit hash (`MINT - 1.2.0 - 2026-11-02 - 56a329a`); the updater
compares that hash with the one built into the app, so keep that format if you
publish by hand. Pre-releases are invisible to the Stable channel.

The workflows publish to the repository they live in, and the app reads
`InsanityLabs/MINT_Moc3_INochi_Translator` (`REPO` in
`crates/mint-gui/src/update.rs`); keep the two the same.

## Layout

- `crates/mint-core` — moc3 parser, pose evaluation, rasterizer, emitters
- `crates/mint-cli` — command line tool
- `crates/mint-gui` — egui app
- `vendor/moc2cmo` — cmo3 archive/XML builder (MIT, by Eatgrapes; modified)

## License

[PolyForm Noncommercial 1.0.0](LICENSE) — free to use, modify and share for
**noncommercial purposes only**. Commercial use requires a separate license.
