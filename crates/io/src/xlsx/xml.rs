//! Bounded XML reader: no DTD/entities, depth cap (spec F-9.6).

use omacell_core::error::CoreError;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error;

/// Maximum element nesting.
pub const MAX_XML_DEPTH: u32 = 64;

/// One XML event with local names (namespaces stripped).
#[derive(Clone, Debug)]
pub enum XmlEvent {
    /// Start tag.
    Start {
        /// Local name.
        name: String,
        /// Attribute local-name / value pairs, document order.
        attrs: Vec<(String, String)>,
    },
    /// End tag.
    End {
        /// Local name.
        name: String,
    },
    /// Character data (trimmed only when the caller asks).
    Text(String),
    /// Empty element (`<br/>`) as start+end.
    Empty {
        /// Local name.
        name: String,
        /// Attributes.
        attrs: Vec<(String, String)>,
    },
}

/// Streaming reader over a UTF-8 XML part.
pub struct XmlReader<'a> {
    inner: Reader<&'a [u8]>,
    buf: Vec<u8>,
    depth: u32,
}

impl<'a> XmlReader<'a> {
    /// Wrap `bytes`. Rejects a UTF-8 BOM by skipping it.
    pub fn new(mut bytes: &'a [u8]) -> Self {
        if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            bytes = &bytes[3..];
        }
        let mut inner = Reader::from_reader(bytes);
        inner.config_mut().trim_text(false);
        inner.config_mut().check_end_names = true;
        inner.config_mut().expand_empty_elements = false;
        Self {
            inner,
            buf: Vec::with_capacity(1024),
            depth: 0,
        }
    }

    /// Next event, or `None` at EOF.
    pub fn next(&mut self) -> Result<Option<XmlEvent>, CoreError> {
        loop {
            self.buf.clear();
            let ev = self
                .inner
                .read_event_into(&mut self.buf)
                .map_err(|e| error::xlsx_xml(e.to_string()))?;
            match ev {
                Event::Start(e) => {
                    self.depth += 1;
                    if self.depth > MAX_XML_DEPTH {
                        return Err(error::xlsx_limit(format!(
                            "XML nesting exceeds {MAX_XML_DEPTH}"
                        )));
                    }
                    return Ok(Some(XmlEvent::Start {
                        name: local_name(e.name().as_ref()),
                        attrs: collect_attrs(&e)?,
                    }));
                }
                Event::Empty(e) => {
                    return Ok(Some(XmlEvent::Empty {
                        name: local_name(e.name().as_ref()),
                        attrs: collect_attrs(&e)?,
                    }));
                }
                Event::End(e) => {
                    self.depth = self.depth.saturating_sub(1);
                    return Ok(Some(XmlEvent::End {
                        name: local_name(e.name().as_ref()),
                    }));
                }
                Event::Text(t) => {
                    let s = t
                        .xml10_content()
                        .map_err(|e| error::xlsx_xml(e.to_string()))?
                        .into_owned();
                    if s.is_empty() {
                        continue;
                    }
                    return Ok(Some(XmlEvent::Text(s)));
                }
                Event::CData(t) => {
                    let s = std::str::from_utf8(t.as_ref())
                        .map_err(|e| error::xlsx_xml(e.to_string()))?
                        .to_string();
                    if s.is_empty() {
                        continue;
                    }
                    return Ok(Some(XmlEvent::Text(s)));
                }
                Event::DocType(_) => {
                    return Err(error::xlsx_xml(
                        "DTD / DOCTYPE is not allowed in Office XML",
                    ));
                }
                Event::GeneralRef(_) => {
                    return Err(error::xlsx_xml(
                        "entity references are not allowed in Office XML",
                    ));
                }
                Event::Eof => return Ok(None),
                Event::Comment(_) | Event::PI(_) | Event::Decl(_) => continue,
            }
        }
    }
}

fn local_name(qname: &[u8]) -> String {
    let s = std::str::from_utf8(qname).unwrap_or("");
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.to_string(),
    }
}

fn collect_attrs(
    e: &quick_xml::events::BytesStart<'_>,
) -> Result<Vec<(String, String)>, CoreError> {
    let mut out = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|err| error::xlsx_xml(err.to_string()))?;
        let key = local_name(a.key.as_ref());
        let val = a
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|err| error::xlsx_xml(err.to_string()))?
            .into_owned();
        out.push((key, val));
    }
    Ok(out)
}

/// Attribute helper.
#[must_use]
pub fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}
