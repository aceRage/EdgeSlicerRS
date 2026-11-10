//! One `.model` part: mesh and component objects plus the build list.

use std::sync::Arc;

use edge_mesh::{Mesh, Transform};
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

use crate::{
    Error,
    xml::{attributes, malformed, number},
};

#[derive(Default)]
pub(crate) struct ModelFile {
    pub objects: Vec<RawObject>,
    pub items: Vec<Reference>,
}

pub(crate) struct RawObject {
    pub id: u32,
    pub name: Option<String>,
    pub body: Body,
}

pub(crate) enum Body {
    Mesh(Arc<Mesh>),
    Components(Vec<Reference>),
}

/// A component or build item: another object, placed.
pub(crate) struct Reference {
    /// The part holding the object when it is not this one (production extension).
    pub path: Option<String>,
    pub object: u32,
    pub transform: Transform,
    pub printable: bool,
}

impl ModelFile {
    /// Other parts this one's components and build items point into.
    pub fn referenced_parts(&self) -> impl Iterator<Item = &str> {
        let components = self.objects.iter().flat_map(|object| match &object.body {
            Body::Components(components) => components.as_slice(),
            Body::Mesh(_) => &[],
        });
        components
            .chain(&self.items)
            .filter_map(|reference| reference.path.as_deref())
    }
}

#[derive(Default)]
struct OpenObject {
    id: u32,
    name: Option<String>,
    has_mesh: bool,
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    components: Vec<Reference>,
}

pub(crate) fn parse(text: &str, part: &str) -> Result<ModelFile, Error> {
    let mut reader = Reader::from_str(text);
    let mut file = ModelFile::default();
    // Millimetres per model unit; applied to vertices and to translations.
    let mut scale = 1.0f64;
    let mut open: Option<OpenObject> = None;

    loop {
        let (element, is_empty) = match reader
            .read_event()
            .map_err(|e| malformed(part, e.to_string()))?
        {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(element) => {
                if element.local_name().as_ref() == "object"
                    && let Some(object) = open.take()
                {
                    file.objects.push(close(object, part)?);
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };

        match element.local_name().as_ref() {
            "vertex" => {
                let Some(object) = &mut open else { continue };
                let mut position = [f32::NAN; 3];
                attributes(&element, part, |key, value| {
                    let axis = match key {
                        "x" => 0,
                        "y" => 1,
                        "z" => 2,
                        _ => return Ok(()),
                    };
                    position[axis] = number::<f32>(part, key, value)? * scale as f32;
                    Ok(())
                })?;
                if !position.iter().all(|c| c.is_finite()) {
                    return Err(malformed(
                        part,
                        format!(
                            "object {} has a vertex without finite x, y and z",
                            object.id
                        ),
                    ));
                }
                object.vertices.push(position);
            }
            "triangle" => {
                let Some(object) = &mut open else { continue };
                let mut corners = [u32::MAX; 3];
                attributes(&element, part, |key, value| {
                    let corner = match key {
                        "v1" => 0,
                        "v2" => 1,
                        "v3" => 2,
                        _ => return Ok(()),
                    };
                    corners[corner] = number(part, key, value)?;
                    Ok(())
                })?;
                object.triangles.push(corners);
            }
            "object" => {
                let mut object = OpenObject::default();
                let mut id = None;
                attributes(&element, part, |key, value| {
                    match key {
                        "id" => id = Some(number(part, "object id", value)?),
                        "name" => object.name = Some(value.to_owned()),
                        _ => {}
                    }
                    Ok(())
                })?;
                object.id = id.ok_or_else(|| malformed(part, "an object has no id"))?;
                if is_empty {
                    file.objects.push(close(object, part)?);
                } else {
                    open = Some(object);
                }
            }
            "mesh" => {
                if let Some(object) = &mut open {
                    object.has_mesh = true;
                }
            }
            "component" => {
                if let Some(object) = &mut open {
                    object.components.push(reference(&element, part, scale)?);
                }
            }
            "item" => file.items.push(reference(&element, part, scale)?),
            "model" => attributes(&element, part, |key, value| {
                if key == "unit" {
                    scale = match value {
                        "micron" => 0.001,
                        "millimeter" => 1.0,
                        "centimeter" => 10.0,
                        "inch" => 25.4,
                        "foot" => 304.8,
                        "meter" => 1000.0,
                        other => return Err(malformed(part, format!("unknown unit \"{other}\""))),
                    };
                }
                Ok(())
            })?,
            _ => {}
        }
    }
    Ok(file)
}

fn close(object: OpenObject, part: &str) -> Result<RawObject, Error> {
    let body = if object.has_mesh {
        let count = object.vertices.len();
        if let Some(bad) = object
            .triangles
            .iter()
            .flatten()
            .find(|&&corner| corner as usize >= count)
        {
            let reason = if *bad == u32::MAX {
                format!("object {} has a triangle without v1, v2 and v3", object.id)
            } else {
                format!(
                    "object {} has a triangle using vertex {bad}, but only {count} vertices",
                    object.id
                )
            };
            return Err(malformed(part, reason));
        }
        let mut triangles = object.triangles;
        triangles.retain(|[a, b, c]| a != b && b != c && c != a);
        Body::Mesh(Arc::new(Mesh {
            vertices: object.vertices,
            triangles,
        }))
    } else {
        Body::Components(object.components)
    };
    Ok(RawObject {
        id: object.id,
        name: object.name,
        body,
    })
}

fn reference(element: &BytesStart<'_>, part: &str, scale: f64) -> Result<Reference, Error> {
    let mut reference = Reference {
        path: None,
        object: 0,
        transform: Transform::IDENTITY,
        printable: true,
    };
    let mut object = None;
    attributes(element, part, |key, value| {
        match key {
            "objectid" => object = Some(number(part, "objectid", value)?),
            "path" => reference.path = Some(value.to_owned()),
            "printable" => reference.printable = value != "0",
            "transform" => reference.transform = transform(part, value, scale)?,
            _ => {}
        }
        Ok(())
    })?;
    reference.object =
        object.ok_or_else(|| malformed(part, "a component or build item has no objectid"))?;
    Ok(reference)
}

/// Twelve numbers, the 4x3 matrix row by row, translation last.
fn transform(part: &str, text: &str, scale: f64) -> Result<Transform, Error> {
    let mut rows = [[0.0f64; 3]; 4];
    let mut values = text.split_ascii_whitespace();
    for value in rows.iter_mut().flatten() {
        let text = values.next().ok_or_else(|| {
            malformed(
                part,
                format!("transform \"{text}\" has fewer than 12 numbers"),
            )
        })?;
        *value = number(part, "transform", text)?;
    }
    if values.next().is_some() {
        return Err(malformed(
            part,
            format!("transform \"{text}\" has more than 12 numbers"),
        ));
    }
    for offset in &mut rows[3] {
        *offset *= scale;
    }
    Ok(Transform { rows })
}
