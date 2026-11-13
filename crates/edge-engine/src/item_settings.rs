//! Settings of an object, of a part and of a height range of an object, as
//! the model keeps them: what the reference's "Objects" pages set
//! (Tab.cpp `TabPrintModel::on_value_change`, `reset_model_config`) and its
//! "Height range Modifier" adds (GUI_ObjectList.cpp `layers_editing`,
//! `add_layer_range_after_current`, `edit_layer_range`, `del_layer_range`).
//!
//! They are kept and saved with the object ([`edge_3mf::ObjectKept`],
//! [`edge_3mf::PartKept`]) as the reference writes them. Nothing here says
//! what a slice does with them; an edit gives the object a new
//! [`Shape::stamp`], so the slices of the plates it is on are let go.

use std::{path::Path, sync::Arc};

use edge_3mf::{LayerRange, ObjectKept, PartKept, PartKind};

use super::{Model, ModelObject, Shape, refused};
use crate::{Bed, Failure};

/// One setting to set (`Some`, as the reference writes it) or to give back
/// to what is over it (`None`).
pub type SettingChange = (String, Option<String>);

impl ModelObject {
    /// The object's own settings, key and value as the reference writes
    /// them, in the order they are kept.
    pub fn settings(&self) -> Vec<(String, String)> {
        self.kept
            .as_deref()
            .map(ObjectKept::settings)
            .unwrap_or_default()
    }

    /// The object's variable layer height profile, when it has one
    /// ([`ObjectKept::layer_height_profile`]).
    pub fn layer_height_profile(&self) -> Option<Vec<f64>> {
        self.kept.as_deref()?.layer_height_profile()
    }

    /// The object's height ranges, lowest first.
    pub fn layer_ranges(&self) -> Vec<LayerRange> {
        self.kept
            .as_deref()
            .map(ObjectKept::layer_ranges)
            .unwrap_or_default()
    }
}

impl Model {
    /// The object's kept part, made for an object no file held. Such an
    /// object is saved under its name; one without (a bare STL) is given
    /// the name the save would give it (its file's), which it is shown by.
    fn object_kept(&mut self, at: usize) -> ObjectKept {
        if self.objects[at].kept.is_none() && self.objects[at].name.is_empty() {
            let name = self
                .path
                .as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.objects[at].name = name;
        }
        self.objects[at]
            .kept
            .as_deref()
            .cloned()
            .unwrap_or_else(ObjectKept::made_here)
    }

    /// A new stamp for the object's shape, so that what is sliced of it is
    /// let go.
    fn restamp(&mut self, at: usize) {
        let shape = self.objects[at].shape.clone();
        self.objects[at].shape = Shape::keeping(shape.parts.clone(), shape.ids.clone());
    }

    /// Sets or takes away settings of an object's own (Tab.cpp
    /// `TabPrintModel::on_value_change`, `reset_model_config`).
    pub fn set_object_settings(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        changes: &[SettingChange],
    ) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        self.settle(bed);
        let mut kept = self.object_kept(at);
        for (key, value) in changes {
            kept = kept.with_own_setting(key, value.as_deref());
        }
        self.objects[at].kept = Some(Arc::new(kept));
        self.restamp(at);
        Ok(())
    }

    /// Sets or takes away settings of a part's own. A part or a modifier
    /// has settings of its own; the reference shows none for a negative
    /// part, a support blocker or enforcer or a Precise Seam part
    /// (GUI_ObjectList.cpp `add_settings_item`).
    pub fn set_part_settings(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        part: u64,
        changes: &[SettingChange],
    ) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        let index = self.part_at(at, part)?;
        let shape = self.objects[at].shape.clone();
        if !matches!(
            shape.parts[index].kind,
            PartKind::Normal | PartKind::Modifier
        ) {
            return Err(refused("only a part or a modifier has settings of its own"));
        }
        self.settle(bed);
        let mut parts = shape.parts.clone();
        let mut kept = parts[index]
            .kept
            .as_deref()
            .cloned()
            .unwrap_or_else(PartKept::made_here);
        for (key, value) in changes {
            kept = kept.with_own_setting(key, value.as_deref());
        }
        parts[index].kept = Some(Arc::new(kept));
        self.objects[at].shape = Shape::keeping(parts, shape.ids.clone());
        Ok(())
    }

    /// Gives an object a variable layer height profile, or takes it away
    /// (GLCanvas3D.cpp `LayersEditing`, which sets the model object's
    /// `layer_height_profile`).
    pub fn set_layer_height_profile(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        profile: Option<Vec<f64>>,
    ) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        if let Some(profile) = &profile
            && (profile.len() < 4
                || !profile.len().is_multiple_of(2)
                || profile.iter().any(|value| !value.is_finite()))
        {
            return Err(refused("Invalid numeric."));
        }
        self.settle(bed);
        let kept = self
            .object_kept(at)
            .with_layer_height_profile(profile.as_deref());
        self.objects[at].kept = Some(Arc::new(kept));
        self.restamp(at);
        Ok(())
    }

    /// Gives an object these height ranges, put in order of their heights
    /// as the reference's map keeps them. A range is above the bed and has
    /// a top above its bottom, and none overlaps another: the reference
    /// takes a range typed over another's place in its stead and lets
    /// ranges overlap (GUI_ObjectList.cpp `edit_layer_range`), which this
    /// refuses. A project whose file has the reference's height colour
    /// patterns keeps its ranges as the file has them: those patterns'
    /// bands are ranges this app does not tell apart.
    pub fn set_height_ranges(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        mut ranges: Vec<LayerRange>,
    ) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        if self
            .kept
            .as_deref()
            .is_some_and(edge_3mf::Kept::has_height_patterns)
        {
            return Err(refused(
                "the project has height colour patterns (\"Color Pattern...\"), whose bands are height ranges: its height ranges are kept as the file has them",
            ));
        }
        for range in &ranges {
            if !(range.min_z.is_finite() && range.max_z.is_finite()) || range.min_z < 0.0 {
                return Err(refused("Invalid numeric."));
            }
            if range.max_z <= range.min_z {
                return Err(refused(format!(
                    "a height range ends above where it starts: {:.2}-{:.2} mm",
                    range.min_z, range.max_z
                )));
            }
        }
        ranges.sort_by(|a, b| {
            a.min_z
                .total_cmp(&b.min_z)
                .then(a.max_z.total_cmp(&b.max_z))
        });
        if let Some(pair) = ranges.windows(2).find(|pair| pair[1].min_z < pair[0].max_z) {
            return Err(refused(format!(
                "height ranges cannot overlap: {:.2}-{:.2} mm and {:.2}-{:.2} mm",
                pair[0].min_z, pair[0].max_z, pair[1].min_z, pair[1].max_z
            )));
        }
        self.settle(bed);
        let kept = self.object_kept(at).with_height_ranges(&ranges);
        self.objects[at].kept = Some(Arc::new(kept));
        self.restamp(at);
        Ok(())
    }
}
