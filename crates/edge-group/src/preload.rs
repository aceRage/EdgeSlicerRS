//! Loading every filament on the first layer (`preload_all_filaments`,
//! ToolOrdering.cpp `preload_first_layer_tool_order`).

/// The first layer's order when every filament the plate uses is loaded
/// on it: those the layer does not print go first, grouped by extruder
/// (`extruder_of`, a negative one counted as 0) with the group of the
/// extruder the layer starts on last, ascending within a group; then the
/// layer's own order, unchanged. An empty layer stays empty.
pub fn preload_first_layer_tool_order(
    first_layer: &[u32],
    used: &[u32],
    extruder_of: impl Fn(u32) -> i32,
) -> Vec<u32> {
    let Some(first) = first_layer.first() else {
        return Vec::new();
    };
    let mut missing: Vec<u32> = (used.iter())
        .copied()
        .filter(|filament| !first_layer.contains(filament))
        .collect();
    missing.sort_unstable();
    missing.dedup();
    if missing.is_empty() {
        return first_layer.to_vec();
    }
    let extruder = |filament: u32| extruder_of(filament).max(0);
    let starting = extruder(*first);
    missing.sort_by_key(|filament| {
        let e = extruder(*filament);
        (e == starting, e, *filament)
    });
    missing.extend_from_slice(first_layer);
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_filaments_go_first_by_extruder() {
        // Filaments 0, 2 on extruder 0; 1, 3 on extruder 1. The layer
        // prints 3 then 0, so it starts on extruder 1: extruder 0's missing
        // filament first, then extruder 1's.
        let extruder = |filament: u32| (filament % 2) as i32;
        assert_eq!(
            preload_first_layer_tool_order(&[3, 0], &[0, 1, 2, 3], extruder),
            [2, 1, 3, 0]
        );
        assert_eq!(
            preload_first_layer_tool_order(&[0, 1], &[0, 1], extruder),
            [0, 1]
        );
        assert!(preload_first_layer_tool_order(&[], &[0, 1], extruder).is_empty());
    }
}
