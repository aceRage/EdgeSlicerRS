//! The small slice of XML handling every part of a 3MF needs.

use quick_xml::events::BytesStart;

use crate::Error;

pub(crate) fn malformed(part: &str, reason: impl Into<String>) -> Error {
    Error::Malformed {
        part: part.to_owned(),
        reason: reason.into(),
    }
}

/// Calls `visit` with each attribute's name (namespace prefix removed) and
/// its value with entities such as `&amp;` decoded.
pub(crate) fn attributes(
    element: &BytesStart<'_>,
    part: &str,
    mut visit: impl FnMut(&str, &str) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut attributes = element.attributes();
    // Duplicate detection costs a pass per attribute and meshes have millions.
    attributes.with_checks(false);
    for attribute in attributes {
        let attribute = attribute.map_err(|e| malformed(part, e.to_string()))?;
        let key = attribute.key.local_name();
        let raw: &str = &attribute.value;
        if raw.contains('&') {
            let value =
                quick_xml::escape::unescape(raw).map_err(|e| malformed(part, e.to_string()))?;
            visit(key.as_ref(), &value)?;
        } else {
            visit(key.as_ref(), raw)?;
        }
    }
    Ok(())
}

pub(crate) fn number<T: std::str::FromStr>(part: &str, name: &str, text: &str) -> Result<T, Error> {
    text.trim().parse().map_err(|_| {
        malformed(
            part,
            format!("{name} is \"{text}\", which is not a valid number"),
        )
    })
}
