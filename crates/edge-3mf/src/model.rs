//! One `.model` part: mesh and component objects plus the build list.

use std::collections::BTreeSet;
use std::sync::Arc;

use edge_mesh::{Mesh, Transform};

use crate::paint::{self, Facet, SeamFacet};
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
    /// What is painted on triangles here other than colour and seams,
    /// none of which this reader takes in: named so that it can be said.
    pub unread_paint: BTreeSet<&'static str>,
}

pub(crate) struct RawObject {
    pub id: u32,
    pub name: Option<String>,
    pub body: Shape,
}

pub(crate) enum Shape {
    /// A mesh, its triangles as painted for filaments when any of them
    /// is, and the pieces of them painted for seams when any are.
    Mesh(
        Arc<Mesh>,
        Option<Arc<Vec<Facet>>>,
        Option<Arc<Vec<SeamFacet>>>,
    ),
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
            Shape::Components(components) => components.as_slice(),
            Shape::Mesh(..) => &[],
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
    /// Each triangle's `paint_color`, where it has one.
    paint: Vec<Option<Box<str>>>,
    /// Each triangle's `paint_seam`, where it has one.
    seam: Vec<Option<Box<str>>>,
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
                let (mut paint, mut seam) = (None, None);
                let unread = &mut file.unread_paint;
                attributes(&element, part, |key, value| {
                    // bbs_3mf.cpp: the other things a triangle may be
                    // painted with.
                    let other = match key {
                        "paint_fuzzy_skin" | "paint_fuzzy" => Some("fuzzy skin"),
                        "paint_supports" => Some("supports"),
                        _ => None,
                    };
                    if let Some(other) = other {
                        if !value.is_empty() {
                            unread.insert(other);
                        }
                        return Ok(());
                    }
                    let corner = match key {
                        "v1" => 0,
                        "v2" => 1,
                        "v3" => 2,
                        "paint_color" => {
                            paint = (!value.is_empty()).then(|| Box::from(value));
                            return Ok(());
                        }
                        "paint_seam" => {
                            seam = (!value.is_empty()).then(|| Box::from(value));
                            return Ok(());
                        }
                        _ => return Ok(()),
                    };
                    corners[corner] = number(part, key, value)?;
                    Ok(())
                })?;
                object.triangles.push(corners);
                object.paint.push(paint);
                object.seam.push(seam);
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
        let sound = |[a, b, c]: &[u32; 3]| a != b && b != c && c != a;
        // Painted, every triangle is given with its filament, the
        // unpainted ones with none.
        let paint = object.paint.iter().any(Option::is_some).then(|| {
            let mut facets = Vec::with_capacity(object.triangles.len());
            for (triangle, code) in object.triangles.iter().zip(&object.paint) {
                if !sound(triangle) {
                    continue;
                }
                let corners = triangle.map(|corner| object.vertices[corner as usize]);
                let read = code
                    .as_deref()
                    .is_some_and(|code| paint::decode(code, corners, &mut facets));
                if !read {
                    facets.push(Facet {
                        corners,
                        filament: 0,
                    });
                }
            }
            Arc::new(facets)
        });
        // Only the pieces painted for seams are kept.
        let seam = object.seam.iter().any(Option::is_some).then(|| {
            let mut facets = Vec::new();
            for (triangle, code) in object.triangles.iter().zip(&object.seam) {
                if let Some(code) = code.as_deref().filter(|_| sound(triangle)) {
                    let corners = triangle.map(|corner| object.vertices[corner as usize]);
                    paint::decode_seam(code, corners, &mut facets);
                }
            }
            Arc::new(facets)
        });
        let mut triangles = object.triangles;
        triangles.retain(sound);
        Shape::Mesh(
            Arc::new(Mesh {
                vertices: object.vertices,
                triangles,
            }),
            paint,
            seam,
        )
    } else {
        Shape::Components(object.components)
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::paint::SeamPaint;

    #[test]
    fn paint_that_is_not_colour_is_named_and_left_alone() {
        let model = |triangle: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
 <resources><object id="1" type="model"><mesh>
  <vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices>
  <triangles><triangle v1="0" v2="1" v3="2" {triangle}/></triangles>
 </mesh></object></resources>
 <build><item objectid="1"/></build>
</model>"#
            )
        };
        let unread = |triangle: &str| -> Vec<&'static str> {
            parse(&model(triangle), "3D/3dmodel.model")
                .unwrap()
                .unread_paint
                .into_iter()
                .collect()
        };
        assert!(unread("").is_empty());
        assert!(unread(r#"paint_color="4""#).is_empty());
        assert_eq!(unread(r#"paint_fuzzy_skin="4""#), ["fuzzy skin"]);
        assert_eq!(unread(r#"paint_fuzzy="8""#), ["fuzzy skin"]);
        // Seams painted on or off are read.
        assert_eq!(unread(r#"paint_seam="4" paint_supports="8""#), ["supports"]);
        // An empty attribute paints nothing.
        assert!(unread(r#"paint_seam="""#).is_empty());

        // Seams painted on and off are read: the triangle cut in four
        // (the code read backwards, the last piece first), the middle
        // piece and one corner wanted, one corner kept off, one left; what
        // cannot be read paints nothing.
        let seams = |triangle: &str| -> Vec<(SeamPaint, f32)> {
            let file = parse(&model(triangle), "3D/3dmodel.model").unwrap();
            match &file.objects[0].body {
                Shape::Mesh(_, _, Some(seam)) => (seam.iter())
                    .map(|facet| (facet.kind, facet.corners[0][0]))
                    .collect(),
                _ => Vec::new(),
            }
        };
        assert_eq!(seams(r#"paint_seam="4""#), [(SeamPaint::Enforcer, 0.0)]);
        assert_eq!(seams(r#"paint_seam="8""#), [(SeamPaint::Blocker, 0.0)]);
        assert_eq!(
            seams(r#"paint_seam="80443""#),
            [
                (SeamPaint::Enforcer, 0.5),
                (SeamPaint::Enforcer, 0.5),
                (SeamPaint::Blocker, 0.0)
            ]
        );
        assert!(seams(r#"paint_seam="0""#).is_empty());
        assert!(seams(r#"paint_seam="x4""#).is_empty());
    }
}
