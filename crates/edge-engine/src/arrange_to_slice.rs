//! Opening a model to slice it as the reference's command line does: a
//! model that is not a project of the reference's own kind (an STL or
//! OBJ, or a 3MF another program wrote, or one without printer settings)
//! has no plates the reference trusts, so before slicing it arranges
//! everything on the bed (Snapmaker_Orca.cpp l.2386 and l.2512 set
//! `need_arrange`; l.5394 arranges). A project of its own kind is sliced
//! where it stands, unless arranging is asked for.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use edge_gcode::PrintSettings;

use crate::{
    Bed, Chosen, Failure, Loaded, Model, Opened, Overrides,
    load::{self, Held},
};

/// Whether to arrange before slicing, as the reference's `--arrange`
/// takes it ("0-disable, 1-enable, others-auto", PrintConfig.cpp).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Arrange {
    /// What the reference does by itself: arrange a model that is not a
    /// project of its own kind.
    #[default]
    Auto,
    /// Never: everything stands where the file has it.
    Off,
    /// Always. Only a model that is arranged anyway is taken: arranging a
    /// project's plate before slicing it (the reference's "only arrange
    /// current plate", l.5583) is not done here.
    On,
}

impl Arrange {
    /// From the number the reference's `--arrange` takes.
    pub fn from_option(value: i64) -> Self {
        match value {
            0 => Self::Off,
            1 => Self::On,
            _ => Self::Auto,
        }
    }
}

/// A model opened to be sliced.
pub struct ToSlice {
    /// What to slice.
    pub loaded: Loaded,
    /// The number of the plate taken and how many there are; none for a
    /// model sliced whole.
    pub plate: Option<(u32, usize)>,
    /// What arranging did, when it was done.
    pub arranged: Option<ArrangedToSlice>,
    /// The model as arranged, when it was: what a sliced file of the
    /// plate holds.
    pub model: Option<Model>,
}

/// What arranging a model before slicing it did.
#[derive(Clone, Debug)]
pub struct ArrangedToSlice {
    /// How long arranging took, settling the settings it uses included.
    pub took: Duration,
    /// How many plates the objects are on now.
    pub plates: usize,
    /// The objects no plate could hold, put beside the plates: not sliced.
    pub unplaced: Vec<String>,
    /// The objects with no size, left where they were.
    pub zero_size: Vec<String>,
}

/// Opens a model to slice plate `plate` of it (the first when left out),
/// arranging it first as the reference's command line would, with the
/// settings it will be sliced with: the presets `chosen` and `overrides`
/// over its own. A model that is arranged has plates afterwards, as the
/// reference's has, and `plate` picks among them.
pub fn open_to_slice(
    path: &Path,
    plate: Option<u32>,
    arrange: Arrange,
    chosen: &Chosen,
    overrides: &Overrides,
) -> Result<ToSlice, Failure> {
    let opened = crate::open(path, None)?;
    let own_kind = match &opened.held {
        Held::Model(_) => false,
        Held::Project { project, .. } => project.is_reference_project(),
    };
    let arranging = match arrange {
        Arrange::Off => false,
        Arrange::Auto => !own_kind,
        Arrange::On if own_kind => {
            return Err(format!(
                "{}: arranging a project's plate before slicing it (--arrange 1 on a project with \
                 its own plates) is not done yet; no G-code was written",
                path.display()
            )
            .into());
        }
        Arrange::On => true,
    };
    let started = Instant::now();
    // The settings it will be sliced with, and the bed they give.
    let settled = if arranging {
        let filaments = opened.filaments().into_iter().max().unwrap_or(1);
        let block = crate::followed::block(opened.settings(), filaments);
        let settings = crate::prepare(block, chosen, overrides)?.settings;
        PrintSettings::from_settings(&settings)?
            .bed
            .map(|bed| (settings, Bed::of(bed)))
    } else {
        None
    };
    let Some((settings, bed)) = settled else {
        // Not arranged (or with no bed to arrange on): as the file has it.
        let opened = take_plate(opened, plate)?;
        let taken = opened.plate();
        return Ok(ToSlice {
            loaded: opened.place()?,
            plate: taken,
            arranged: None,
            model: None,
        });
    };
    let mut model = Model::from(opened);
    let arranged = model.arrange_as_command_line(&bed, &settings)?;
    let took = started.elapsed();
    let id = plate.unwrap_or(1);
    let loaded = model.place(Some(id))?;
    Ok(ToSlice {
        loaded,
        plate: Some((id, model.plates().len())),
        arranged: Some(ArrangedToSlice {
            took,
            plates: model.plates().len(),
            unplaced: arranged.unplaced,
            zero_size: arranged.zero_size,
        }),
        model: Some(model),
    })
}

/// The file opened for plate `plate`, as [`crate::open`] takes it.
fn take_plate(opened: Opened, plate: Option<u32>) -> Result<Opened, Failure> {
    match opened.held {
        Held::Project { project, .. } => {
            let index = load::plate_of(&opened.path, &project, plate)?;
            Ok(Opened {
                path: opened.path,
                held: Held::Project {
                    project,
                    plate: index,
                },
            })
        }
        Held::Model(_) if plate.is_some() => Err("--plate only applies to 3MF files".into()),
        Held::Model(_) => Ok(opened),
    }
}
