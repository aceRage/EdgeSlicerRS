# CLAUDE.md

EdgeSlicerRS is a clean-slate Rust slicing engine for EdgeSlicer. Performance
is the goal; a Tauri + web UI comes second. See README.md for layout and status.

## Commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run --release -p edge-cli -- slice <model.stl|project.3mf> [--plate N] [--svg out.svg]
cargo run --release -p edge-cli -- inspect <project.3mf>
cargo run --release -p edge-cli -- preset <vendor> <kind> <name> --profiles reference/EdgeSlicer/resources/profiles
```

Three test files (`edge-config/tests/reference_profiles.rs`,
`edge-3mf/tests/reference_projects.rs`,
`edge-perimeter/tests/reference_projects.rs`) run against everything in the
submodule; a new stage should add its own and keep all of them at zero failures.

On the Mac mini cargo comes from Homebrew rustup: `/opt/homebrew/opt/rustup/bin`.

## Rules

- `reference/EdgeSlicer` is a pinned submodule of the C++ slicer. Never edit
  it from here; C++ changes go through the EdgeSlicer repo. Bump the pin
  deliberately, in its own commit.
- The reference slicer defines correct output. Where this engine differs on
  purpose, say so in the code and in the PR.
- Output must be deterministic: same input gives byte-identical results at
  any thread count. Parity testing depends on it.
- Only `edge-clip` may depend on the clipping library; every other crate
  goes through its API, so the library can be replaced in one place.
- Engine crates stay UI-free and take no dependency on Tauri; the desktop
  app calls into them.
- `unsafe` is denied workspace-wide. A crate that needs it opts in locally
  with the reason and a benchmark showing the gain.
- Tests also run on Windows: no hard-coded path separators or Unix-only APIs.
- No hosted CI; run tests and clippy locally and quote the counts in PRs.
- Licence is AGPL-3.0, so porting from libslic3r is allowed; note the source
  file in a comment when you do.
