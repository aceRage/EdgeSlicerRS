//! The plate's layers shared out copy by copy, for printing object by
//! object (`print_sequence` "by object"; GCode.cpp `_do_export`, the loop
//! over `print_object_instances_ordering`, and `process_layers` for one
//! instance): each copy's layers in turn, every entry holding only what
//! of the plate's layer is that copy's, as the writer shares a layer out
//! among the copies it labels (`edge_gcode::CopyShares`).
//!
//! On each of a copy's layers its filaments go in the plate's order, but
//! the one in use goes first, as the reference's tool order for one
//! object starts from the tool the last one ended with
//! (ToolOrdering.cpp `reorder_extruders`). The skirt goes with the first
//! copy, which the reference prints it with (GCode.cpp
//! `m_skirt_group_done`). A copy's support and brim go with its first
//! filament on the layer, whichever filament began the plate's layer.

use std::ops::Range;

use edge_contour::LayerProfiles;
use edge_extrusion::{ExtrusionLoop, ExtrusionPath};
use edge_gcode::{CopyShares, Layer, ObjectLabel, ObjectRun};

/// One entry of a copy's layer: what of a plate entry is the copy's.
struct Kept {
    /// The plate's entry it is taken from.
    entry: usize,
    around: Vec<ExtrusionLoop>,
    support: Vec<ExtrusionPath>,
    loops: Vec<ExtrusionLoop>,
    islands: Vec<Range<usize>>,
    paths: Vec<ExtrusionPath>,
    profiles: LayerProfiles,
}

/// The plate's layers shared out copy by copy.
pub(crate) struct Split {
    kept: Vec<Kept>,
    /// Each copy's entries, in the order the copies are printed.
    pub runs: Vec<ObjectRun>,
    /// For each entry, whether its copy has support on its layer.
    pub support: Vec<bool>,
}

impl Split {
    /// Shares out `base`, the plate's entries, `layer_of` giving each
    /// one's layer, among `copies` printed in `order`. `profiles` are the
    /// contoured lines of each entry, or none.
    pub(crate) fn new(
        base: &[Layer<'_>],
        layer_of: &[usize],
        order: &[usize],
        copies: &[ObjectLabel],
        profiles: &[&LayerProfiles],
    ) -> Self {
        let shares: Vec<CopyShares> = base
            .iter()
            .map(|layer| CopyShares::of(layer, copies))
            .collect();
        // The plate's entries layer by layer.
        let mut layers: Vec<Range<usize>> = Vec::new();
        for (entry, layer) in layer_of.iter().enumerate() {
            match layers.last_mut() {
                Some(range) if layer_of[range.start] == *layer => range.end = entry + 1,
                _ => layers.push(entry..entry + 1),
            }
        }
        let mut kept: Vec<Kept> = Vec::new();
        let mut runs = Vec::with_capacity(order.len());
        let mut support = Vec::new();
        let mut tool: Option<usize> = None;
        for (run, &copy) in order.iter().enumerate() {
            let start = kept.len();
            for range in &layers {
                let mut on_layer: Vec<Kept> = Vec::new();
                let mut around = Vec::new();
                let mut lines = Vec::new();
                for entry in range.clone() {
                    let (layer, share) = (&base[entry], &shares[entry]);
                    for (lp, owner) in layer.around.iter().zip(&share.around) {
                        let ours = match owner {
                            Some(owner) => *owner == copy,
                            // The skirt, and a brim of several copies,
                            // with the first copy.
                            None => run == 0,
                        };
                        if ours {
                            around.push(lp.clone());
                        }
                    }
                    lines.extend(
                        (layer.support.iter().zip(&share.support))
                            .filter(|(_, owner)| **owner == copy)
                            .map(|(line, _)| line.clone()),
                    );
                    let mut loops = Vec::new();
                    let mut islands = Vec::new();
                    let mut profiled = LayerProfiles::default();
                    let profile = profiles.get(entry);
                    for (island, range) in layer.islands.iter().enumerate() {
                        if share.islands[island] != copy {
                            continue;
                        }
                        let from = loops.len();
                        loops.extend(layer.loops[range.clone()].iter().cloned());
                        islands.push(from..loops.len());
                        if let Some(profile) = profile {
                            for at in range.clone() {
                                profiled
                                    .loops
                                    .push(profile.loops.get(at).cloned().flatten());
                            }
                        }
                    }
                    let mut paths = Vec::new();
                    for (at, (path, owner)) in layer.paths.iter().zip(&share.paths).enumerate() {
                        if *owner != copy {
                            continue;
                        }
                        paths.push(path.clone());
                        if let Some(profile) = profile {
                            profiled
                                .paths
                                .push(profile.paths.get(at).cloned().flatten());
                        }
                    }
                    if !loops.is_empty() || !paths.is_empty() {
                        on_layer.push(Kept {
                            entry,
                            around: Vec::new(),
                            support: Vec::new(),
                            loops,
                            islands,
                            paths,
                            profiles: profiled,
                        });
                    }
                }
                if on_layer.is_empty() {
                    if around.is_empty() && lines.is_empty() {
                        continue;
                    }
                    // Only support or a brim: with the tool in use where
                    // it prints on the layer, else whichever began it.
                    let entry = range
                        .clone()
                        .find(|entry| Some(base[*entry].filament) == tool)
                        .unwrap_or(range.start);
                    on_layer.push(Kept {
                        entry,
                        around: Vec::new(),
                        support: Vec::new(),
                        loops: Vec::new(),
                        islands: Vec::new(),
                        paths: Vec::new(),
                        profiles: LayerProfiles::default(),
                    });
                }
                if let Some(first) = on_layer
                    .iter()
                    .position(|k| Some(base[k.entry].filament) == tool)
                {
                    on_layer.rotate_left(first);
                }
                let has_support = !lines.is_empty();
                on_layer[0].around = around;
                on_layer[0].support = lines;
                tool = on_layer.last().map(|k| base[k.entry].filament);
                support.extend(std::iter::repeat_n(has_support, on_layer.len()));
                kept.extend(on_layer);
            }
            runs.push(ObjectRun {
                copy,
                entries: start..kept.len(),
            });
        }
        Self {
            kept,
            runs,
            support,
        }
    }

    /// The plate's entry each of the copies' entries is taken from.
    pub(crate) fn from(&self) -> impl Iterator<Item = usize> + '_ {
        self.kept.iter().map(|kept| kept.entry)
    }

    /// The contoured lines of each of the copies' entries.
    pub(crate) fn profiles(&self) -> Vec<&LayerProfiles> {
        self.kept.iter().map(|kept| &kept.profiles).collect()
    }

    /// The copies' entries as the writer takes them, each like the
    /// plate's it is taken from but for its lines.
    pub(crate) fn plan<'a>(&'a self, base: &[Layer<'a>]) -> Vec<Layer<'a>> {
        self.kept
            .iter()
            .map(|kept| Layer {
                around: &kept.around,
                support: &kept.support,
                loops: &kept.loops,
                islands: &kept.islands,
                paths: &kept.paths,
                ..base[kept.entry]
            })
            .collect()
    }
}
