//! `Metadata/model_settings.config`: the Bambu/Orca part that names objects,
//! says what each part is for, and assigns instances to plates.

use std::collections::HashMap;

use quick_xml::{Reader, events::Event};

use crate::{
    Error, PartKind,
    xml::{attributes, malformed, number},
};

#[derive(Default)]
pub(crate) struct ModelSettings {
    pub objects: HashMap<u32, ObjectSettings>,
    pub plates: Vec<RawPlate>,
}

#[derive(Default)]
pub(crate) struct ObjectSettings {
    pub name: Option<String>,
    /// The filament the object is printed in, counting from 1.
    pub extruder: Option<u32>,
    /// Keyed by the id of the component the part comes from.
    pub parts: HashMap<u32, PartSettings>,
}

#[derive(Default)]
pub(crate) struct PartSettings {
    pub name: Option<String>,
    pub kind: PartKind,
    /// The filament this part is printed in, if not the object's.
    pub extruder: Option<u32>,
    /// Whether the part names one of Precise Seam's types
    /// (`precise_seam_type`), which a modifier part then is.
    pub precise_seam: bool,
    /// Whether the part is embossed text or an embossed picture: it
    /// carries a text or shape description (bbs_3mf.cpp `TEXT_TAG`,
    /// `SHAPE_TAG`; Model.hpp `ModelVolume::is_text`, `is_svg`).
    pub decoration: bool,
}

impl PartSettings {
    /// What the part is: a modifier that names a Precise Seam type is
    /// one, whichever order its metadata comes in; on any other part the
    /// name is passed over (bbs_3mf.cpp, "Apply the seam mode after all
    /// base-type metadata").
    pub fn kind(&self) -> PartKind {
        if self.kind == PartKind::Modifier && self.precise_seam {
            PartKind::PreciseSeam
        } else {
            self.kind
        }
    }
}

#[derive(Default)]
pub(crate) struct RawPlate {
    pub id: u32,
    pub name: String,
    /// The plate's own order of filaments on the first layer.
    pub first_layer_sequence: Vec<u32>,
    /// `(object id, index among that object's instances)`.
    pub instances: Vec<(u32, usize)>,
}

enum Scope {
    Object(u32),
    Part(u32, u32),
    Plate,
    Instance {
        object: Option<u32>,
        instance: Option<usize>,
    },
    Other,
}

pub(crate) fn parse(text: &str, part: &str) -> Result<ModelSettings, Error> {
    let mut reader = Reader::from_str(text);
    let mut settings = ModelSettings::default();
    let mut scopes: Vec<Scope> = Vec::new();

    loop {
        let (element, is_empty) = match reader
            .read_event()
            .map_err(|e| malformed(part, e.to_string()))?
        {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => {
                if let Some(Scope::Instance {
                    object: Some(object),
                    instance: Some(instance),
                }) = scopes.pop()
                    && let Some(plate) = settings.plates.last_mut()
                {
                    plate.instances.push((object, instance));
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };

        let mut id = None;
        let (mut key, mut value, mut subtype) = (String::new(), String::new(), String::new());
        attributes(&element, part, |name, text| {
            match name {
                "id" => id = Some(number::<u32>(part, "id", text)?),
                "key" => key = text.to_owned(),
                "value" => value = text.to_owned(),
                "subtype" => subtype = text.to_owned(),
                _ => {}
            }
            Ok(())
        })?;

        let scope = match (element.local_name().as_ref(), scopes.last_mut()) {
            ("object", None | Some(Scope::Other)) => match id {
                Some(id) => {
                    settings.objects.entry(id).or_default();
                    Scope::Object(id)
                }
                None => Scope::Other,
            },
            ("part", Some(Scope::Object(object))) => match id {
                Some(id) => {
                    let object = *object;
                    let part = settings
                        .objects
                        .entry(object)
                        .or_default()
                        .parts
                        .entry(id)
                        .or_default();
                    part.kind = PartKind::from_subtype(&subtype);
                    Scope::Part(object, id)
                }
                None => Scope::Other,
            },
            // Embossed text or an embossed picture, described on the
            // part it is (`slic3rpe:text`, `slic3rpe:shape`).
            ("text" | "shape", Some(Scope::Part(object, id))) => {
                let object = settings.objects.entry(*object).or_default();
                object.parts.entry(*id).or_default().decoration = true;
                Scope::Other
            }
            ("plate", _) => {
                settings.plates.push(RawPlate::default());
                Scope::Plate
            }
            ("model_instance", Some(Scope::Plate)) => Scope::Instance {
                object: None,
                instance: None,
            },
            ("metadata", Some(scope)) => {
                match scope {
                    Scope::Object(object) if key == "name" => {
                        settings.objects.entry(*object).or_default().name = Some(value);
                    }
                    Scope::Part(object, id) if key == "name" => {
                        let object = settings.objects.entry(*object).or_default();
                        object.parts.entry(*id).or_default().name = Some(value);
                    }
                    Scope::Object(object) if key == "extruder" => {
                        settings.objects.entry(*object).or_default().extruder =
                            Some(number(part, "extruder", &value)?);
                    }
                    Scope::Part(object, id) if key == "precise_seam_type" => {
                        let object = settings.objects.entry(*object).or_default();
                        object.parts.entry(*id).or_default().precise_seam =
                            crate::is_precise_seam(&value);
                    }
                    Scope::Part(object, id) if key == "extruder" => {
                        let object = settings.objects.entry(*object).or_default();
                        object.parts.entry(*id).or_default().extruder =
                            Some(number(part, "extruder", &value)?);
                    }
                    Scope::Plate => {
                        if let Some(plate) = settings.plates.last_mut() {
                            match key.as_str() {
                                "plater_id" => plate.id = number(part, "plater_id", &value)?,
                                "plater_name" => plate.name = value,
                                // Filament numbers with spaces between;
                                // anything else among them is passed over.
                                "first_layer_print_sequence" => {
                                    plate.first_layer_sequence = value
                                        .split_whitespace()
                                        .filter_map(|number| number.parse().ok())
                                        .collect();
                                }
                                _ => {}
                            }
                        }
                    }
                    Scope::Instance { object, instance } => match key.as_str() {
                        "object_id" => *object = Some(number(part, "object_id", &value)?),
                        "instance_id" => *instance = Some(number(part, "instance_id", &value)?),
                        _ => {}
                    },
                    _ => {}
                }
                Scope::Other
            }
            _ => Scope::Other,
        };
        if !is_empty {
            scopes.push(scope);
        }
    }
    Ok(settings)
}
