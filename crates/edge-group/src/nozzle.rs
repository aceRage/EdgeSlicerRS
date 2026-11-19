//! The nozzles filaments are grouped onto, and a grouping's answer
//! (MultiNozzleUtils.hpp `NozzleInfo`, `NozzleGroupInfo`,
//! `LayeredNozzleGroupResult`).

/// A nozzle's kind (PrintConfig.hpp `NozzleVolumeType`), by the
/// reference's numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VolumeType(pub i32);

impl VolumeType {
    pub const STANDARD: Self = Self(0);
    pub const HIGH_FLOW: Self = Self(1);
    /// "Any kind": what a filament asks for when it does not care.
    pub const HYBRID: Self = Self(2);
    pub const TPU_HIGH_FLOW: Self = Self(3);
    pub const TPU_285: Self = Self(4);
    pub const E3D_HIGH_FLOW: Self = Self(5);

    /// From the setting's text (`s_keys_map_NozzleVolumeType`).
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "Standard" => Self::STANDARD,
            "High Flow" => Self::HIGH_FLOW,
            "Hybrid" => Self::HYBRID,
            "TPU High Flow" => Self::TPU_HIGH_FLOW,
            "TPU285" => Self::TPU_285,
            "E3D High Flow" => Self::E3D_HIGH_FLOW,
            _ => return None,
        })
    }
}

/// One nozzle: its diameter as the reference writes it ("0.4"), its kind,
/// the extruder it is on and its number among all the printer's nozzles
/// (`group_id`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NozzleInfo {
    pub diameter: String,
    pub volume_type: VolumeType,
    pub extruder_id: i32,
    pub group_id: i32,
}

/// The nozzles of `groups`, each a diameter, a kind, an extruder and how
/// many such nozzles that extruder has (MultiNozzleUtils.cpp
/// `build_nozzle_list`): the groups sorted by extruder, then diameter as
/// text, kind and count, and numbered in that order.
pub fn nozzle_list(groups: &[(String, VolumeType, i32, i32)]) -> Vec<NozzleInfo> {
    let mut groups = groups.to_vec();
    // `NozzleGroupInfo::operator<`: extruder, diameter, kind, count.
    groups.sort_by(|a, b| {
        (a.2.cmp(&b.2))
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.3.cmp(&b.3))
    });
    let mut list = Vec::new();
    for (diameter, volume_type, extruder_id, count) in groups {
        for _ in 0..count {
            let group_id = list.len() as i32;
            list.push(NozzleInfo {
                diameter: diameter.clone(),
                volume_type,
                extruder_id,
                group_id,
            });
        }
    }
    list
}

/// Which nozzle each filament prints from, the same on every layer: a
/// printer without a filament switcher (`LayeredNozzleGroupResult` made by
/// its first `create`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupResult {
    /// For each filament, its nozzle's place in [`Self::nozzles`].
    pub map: Vec<i32>,
    pub nozzles: Vec<NozzleInfo>,
    /// The filaments the plate uses, ascending.
    pub used: Vec<u32>,
}

impl GroupResult {
    /// None where there is no map or no nozzle, as the reference's.
    pub fn create(map: Vec<i32>, nozzles: Vec<NozzleInfo>, used: Vec<u32>) -> Option<Self> {
        (!map.is_empty() && !nozzles.is_empty()).then_some(Self { map, nozzles, used })
    }

    pub fn nozzle_from_id(&self, id: i32) -> Option<&NozzleInfo> {
        usize::try_from(id).ok().and_then(|id| self.nozzles.get(id))
    }

    /// The nozzle `filament` prints from (`get_nozzle_for_filament`, and
    /// `get_first_nozzle_for_filament`, the same here).
    pub fn nozzle_for(&self, filament: i64) -> Option<&NozzleInfo> {
        let filament = usize::try_from(filament).ok()?;
        self.nozzle_from_id(*self.map.get(filament)?)
    }

    /// Each filament's extruder, from 0 or from 1, -1 (0 from 1) for one
    /// whose nozzle is not on the list (`get_extruder_map`).
    pub fn extruder_map(&self, zero_based: bool) -> Vec<i32> {
        let offset = i32::from(!zero_based);
        (self.map.iter())
            .map(|nozzle| self.nozzle_from_id(*nozzle).map_or(-1, |n| n.extruder_id) + offset)
            .collect()
    }

    /// Each filament's nozzle number, -1 for one not on the list
    /// (`get_nozzle_map`).
    pub fn nozzle_map(&self) -> Vec<i32> {
        (self.map.iter())
            .map(|nozzle| self.nozzle_from_id(*nozzle).map_or(-1, |n| n.group_id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nozzles_are_numbered_by_extruder_then_kind() {
        let list = nozzle_list(&[
            ("0.4".into(), VolumeType::HIGH_FLOW, 1, 1),
            ("0.4".into(), VolumeType::STANDARD, 1, 2),
            ("0.4".into(), VolumeType::STANDARD, 0, 1),
        ]);
        let shown: Vec<(i32, i32, i32)> = (list.iter())
            .map(|n| (n.group_id, n.extruder_id, n.volume_type.0))
            .collect();
        assert_eq!(shown, [(0, 0, 0), (1, 1, 0), (2, 1, 0), (3, 1, 1)]);
    }

    #[test]
    fn a_map_off_the_list_reads_as_no_extruder() {
        let nozzles = nozzle_list(&[
            ("0.4".into(), VolumeType::STANDARD, 0, 1),
            ("0.4".into(), VolumeType::STANDARD, 1, 1),
        ]);
        let result = GroupResult::create(vec![1, 0, 2], nozzles, vec![0, 1]).unwrap();
        assert_eq!(result.extruder_map(false), [2, 1, 0]);
        assert_eq!(result.extruder_map(true), [1, 0, -1]);
        assert_eq!(result.nozzle_map(), [1, 0, -1]);
        assert!(GroupResult::create(Vec::new(), Vec::new(), Vec::new()).is_none());
    }
}
