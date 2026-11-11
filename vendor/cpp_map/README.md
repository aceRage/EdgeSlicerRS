> **This is EdgeSlicerRS's copy of `cpp_map` 0.2.0** (crates.io; upstream
> commit `1912fd11818b599948ecafdfa6f9af3bc56eec7e` of
> <https://codeberg.org/eadf/cpp_map_rs>), used in place of the published
> crate through `[patch.crates-io]` in `Cargo.toml` and
> `apps/desktop/Cargo.toml`.
>
> **What was changed:** the skip list took the levels of its nodes from a
> thread-local `rand::rng()`. Each list now has a small generator of its
> own (SplitMix64, `LevelRng` in `src/skiplist/skiplist_impl.rs`), started
> from the same constant when the list is made and again when it is
> cleared. The library's dependency on `rand` went with it; the tests and
> the example still use `rand`. The files touched are `Cargo.toml`,
> `src/skiplist.rs` and `skiplist_impl.rs`, `insert.rs` and `extend.rs`
> under `src/skiplist/`, each marked where it was changed. Left out of the
> copy: upstream's CI and coverage scripts and cargo aliases.
>
> **Why:** EdgeSlicerRS must give byte-identical output for the same input
> on every run and at any thread count. The Voronoi library
> (`boostvoronoi`) keeps its beach line in this list. On a well-formed
> sweep the list's shape only decides which comparisons a search makes;
> but where the sweep's ordering has gone wrong (Boost.Polygon's own does
> on the same outlines), the node a search lands on depends on the shape,
> and the diagram came out differently from run to run.
>
> Licence unchanged: MIT or Apache-2.0, at your option; both texts are
> beside this file.

[![Latest version](https://img.shields.io/crates/v/cpp_map.svg)](https://crates.io/crates/cpp_map)
[![Documentation](https://docs.rs/cpp_map/badge.svg)](https://docs.rs/cpp_map)
[![workflow](https://ci.codeberg.org/api/badges/14523/status.svg)](https://ci.codeberg.org/repos/14523)
[![dependency status](https://deps.rs/crate/cpp_map/0.2.0/status.svg)](https://deps.rs/crate/cpp_map/0.2.0)
![license](https://img.shields.io/crates/l/cpp_map)

# cpp_map.rs
# C++ `std::map` Emulator for Rust

A simple C++ `std::map` emulator for Rust.

This library provides a data structure that emulates C++'s `std::map`, particularly its pointer-based cursors/iterators.

## Key Features

- Replicates C++ behavior where `insert(key, value)` is a no-op if the key exists (the new value isn't used)
- Maintains pointer stability like C++'s std::map
- Provides familiar C++-style iterator interface

## Implementations

### Skip List
- O(log n) search and insert
- O(1) sequential access

### Linked List
- O(n) search and insert
- O(1) sequential access

### Performance Note

For primarily position/hint-based operations, the linked list implementation will typically be faster.

### Minimum Supported Rust Version (MSRV)

The minimum supported version of Rust for `cpp_map` is `1.87.0`.

## License

Licensed under either of

* [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0)
* [MIT license](http://opensource.org/licenses/MIT)

at your option.
