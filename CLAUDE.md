# CLAUDE.md

EdgeSlicerRS is a clean-slate Rust slicing engine for EdgeSlicer. Performance
is the goal; a Tauri + web UI comes second. See README.md for layout and status.

## Commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run --release -p edge-cli -- slice <model.stl|project.3mf> [--plate N] [--svg out.svg] [--gcode out.gcode]
cargo run --release -p edge-cli -- inspect <project.3mf>
cargo run --release -p edge-cli -- preset <vendor> <kind> <name> --profiles reference/EdgeSlicer/resources/profiles
```

Five test files (`edge-config/tests/reference_profiles.rs` and a
`tests/reference_projects.rs` in each of `edge-3mf`, `edge-perimeter`,
`edge-infill` and `edge-gcode`) run against everything in the
submodule; a new stage should add its own and keep all of them at zero failures.

On the Mac mini cargo comes from Homebrew rustup: `/opt/homebrew/opt/rustup/bin`.

`edge-parity <project.3mf> --reference <EdgeSlicer executable>` compares this
engine with the C++ slicer (or set `EDGE_REFERENCE_BIN`). On the Mac mini a
build lives at
`~/Dev/EdgeSlicer/build/arm64/Snapmaker_Orca/EdgeSlicer.app/Contents/MacOS/EdgeSlicer`.
`tools/make_fixture.py` writes small parts to compare on (supports, brim).
The tool gives the reference its own empty data directory and `--no-hub`, so
it does not touch the user's settings or a running hub. The build it runs
must be of the commit `reference/EdgeSlicer` is pinned to: check with
`git -C ~/Dev/EdgeSlicer log -1` against `git -C reference/EdgeSlicer log -1`
before trusting a difference, and read the pinned source when they disagree. Run it after a change
to a stage and say in the PR what moved.

## Rules

- `reference/EdgeSlicer` is a pinned submodule of the C++ slicer. Never edit
  it from here; C++ changes go through the EdgeSlicer repo. Bump the pin
  deliberately, in its own commit. After a bump run
  `tools/make_defaults.py <EdgeSlicer executable>`, which rewrites the
  table of the reference's defaults in `edge-config`.
- The reference slicer defines correct output. Where this engine differs on
  purpose, say so in the code and in the PR.
- Output must be deterministic: same input gives byte-identical results at
  any thread count. Parity testing depends on it.
  The G-code's first line carries the time it was made; set
  `SOURCE_DATE_EPOCH` when comparing two runs.
- Only `edge-clip` may depend on the clipping library; every other crate
  goes through its API, so the library can be replaced in one place.
  The same holds for `edge-medial` and the Voronoi library.
- G-code is headed for a real printer. Anything a profile asks for that the
  engine cannot do yet (an unknown template variable, an unsupported
  setting) is an error or a listed note, never a silent default.
  A G-code file is written whole or not at all, and never with toolpaths
  off the bed.
- Engine crates stay UI-free and take no dependency on Tauri; the desktop
  app calls into them.
- `unsafe` is denied workspace-wide. A crate that needs it opts in locally
  with the reason and a benchmark showing the gain.
- Tests also run on Windows: no hard-coded path separators or Unix-only APIs.
- No hosted CI; run tests and clippy locally and quote the counts in PRs.
- Licence is AGPL-3.0, so porting from libslic3r is allowed; note the source
  file in a comment when you do.
