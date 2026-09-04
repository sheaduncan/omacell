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
    input_offset: usize,
    last_span: std::ops::Range<usize>,
}

impl<'a> XmlReader<'a> {
    /// Wrap `bytes`. Rejects a UTF-8 BOM by skipping it.
    pub fn new(mut bytes: &'a [u8]) -> Self {
        let mut input_offset = 0;
        if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            bytes = &bytes[3..];
            input_offset = 3;
        }
        let mut inner = Reader::from_reader(bytes);
        inner.config_mut().trim_text(false);
        inner.config_mut().check_end_names = true;
        inner.config_mut().expand_empty_elements = false;
        Self {
            inner,
            buf: Vec::with_capacity(1024),
            depth: 0,
            input_offset,
            last_span: input_offset..input_offset,
        }
    }

    /// Byte span of the most recently returned event in the original input.
    #[must_use]
    pub fn last_span(&self) -> std::ops::Range<usize> {
        self.last_span.clone()
    }

    /// Next event, or `None` at EOF.
    pub fn next(&mut self) -> Result<Option<XmlEvent>, CoreError> {
        loop {
            self.buf.clear();
            let event_start = self.inner.buffer_position() as usize + self.input_offset;
            let ev = self
                .inner
                .read_event_into(&mut self.buf)
                .map_err(|e| error::xlsx_xml(e.to_string()))?;
            let event_end = self.inner.buffer_position() as usize + self.input_offset;
            match ev {
                Event::Start(e) => {
                    self.depth += 1;
                    if self.depth > MAX_XML_DEPTH {
                        return Err(error::xlsx_limit(format!(
                            "XML nesting exceeds {MAX_XML_DEPTH}"
                        )));
                    }
                    self.last_span = event_start..event_end;
                    return Ok(Some(XmlEvent::Start {
                        name: local_name(e.name().as_ref())?,
                        attrs: collect_attrs(&e)?,
                    }));
                }
                Event::Empty(e) => {
                    self.last_span = event_start..event_end;
                    return Ok(Some(XmlEvent::Empty {
                        name: local_name(e.name().as_ref())?,
                        attrs: collect_attrs(&e)?,
                    }));
                }
                Event::End(e) => {
                    self.depth = self.depth.saturating_sub(1);
                    self.last_span = event_start..event_end;
                    return Ok(Some(XmlEvent::End {
                        name: local_name(e.name().as_ref())?,
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
                    self.last_span = event_start..event_end;
                    return Ok(Some(XmlEvent::Text(s)));
                }
                Event::CData(t) => {
                    let s = std::str::from_utf8(t.as_ref())
                        .map_err(|e| error::xlsx_xml(e.to_string()))?
                        .to_string();
                    if s.is_empty() {
                        continue;
                    }
                    self.last_span = event_start..event_end;
                    return Ok(Some(XmlEvent::Text(s)));
                }
                Event::DocType(_) => {
                    return Err(error::xlsx_xml(
                        "DTD / DOCTYPE is not allowed in Office XML",
                    ));
                }
                Event::GeneralRef(reference) => {
                    let decoded = reference
                        .decode()
                        .map_err(|e| error::xlsx_xml(e.to_string()))?;
                    let text = if let Some(ch) = reference
                        .resolve_char_ref()
                        .map_err(|e| error::xlsx_xml(e.to_string()))?
                    {
                        if !is_xml10_char(ch) {
                            return Err(error::xlsx_xml("illegal XML character reference"));
                        }
                        ch.to_string()
                    } else {
                        match decoded.as_ref() {
                            "lt" => "<".to_string(),
                            "gt" => ">".to_string(),
                            "amp" => "&".to_string(),
                            "apos" => "'".to_string(),
                            "quot" => "\"".to_string(),
                            _ => {
                                return Err(error::xlsx_xml(
                                    "custom entity references are not allowed in Office XML",
                                ));
                            }
                        }
                    };
                    self.last_span = event_start..event_end;
                    return Ok(Some(XmlEvent::Text(text)));
                }
                Event::Eof => return Ok(None),
                Event::Comment(_) | Event::PI(_) | Event::Decl(_) => continue,
            }
        }
    }
}

pub(crate) fn is_xml10_char(ch: char) -> bool {
    matches!(ch, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&ch)
        || ('\u{E000}'..='\u{FFFD}').contains(&ch)
        || ('\u{10000}'..='\u{10FFFF}').contains(&ch)
}

fn local_name(qname: &[u8]) -> Result<String, CoreError> {
    let s = std::str::from_utf8(qname).map_err(|e| error::xlsx_xml(e.to_string()))?;
    Ok(match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.to_string(),
    })
}

fn collect_attrs(
    e: &quick_xml::events::BytesStart<'_>,
) -> Result<Vec<(String, String)>, CoreError> {
    let mut out = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|err| error::xlsx_xml(err.to_string()))?;
        let key = local_name(a.key.as_ref())?;
        let val = a
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|err| error::xlsx_xml(err.to_string()))?
            .into_owned();
        out.push((key, val));
    }
    Ok(out)
}

/// Escape XML text / attribute values.
#[must_use]
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            '\t' => out.push_str("&#x9;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape SpreadsheetML text, including XML-forbidden control characters and
/// literal `_xHHHH_` sequences used by OOXML's character escape convention.
#[must_use]
pub fn escape_ooxml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (offset, ch) in s.char_indices() {
        if ch == '_' && looks_like_ooxml_escape(&s.as_bytes()[offset..]) {
            out.push_str("_x005F_");
            continue;
        }
        if !is_xml10_char(ch) {
            out.push_str(&format!("_x{:04X}_", u32::from(ch)));
            continue;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Decode SpreadsheetML `_xHHHH_` character escapes after XML parsing.
#[must_use]
pub fn decode_ooxml_text(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut offset = 0usize;
    while offset < bytes.len() {
        if looks_like_ooxml_escape(&bytes[offset..]) {
            let code = std::str::from_utf8(&bytes[offset + 2..offset + 6])
                .ok()
                .and_then(|hex| u32::from_str_radix(hex, 16).ok());
            if let Some(ch) = code.and_then(char::from_u32) {
                out.push(ch);
                offset += 7;
                continue;
            }
        }
        let Some(ch) = s[offset..].chars().next() else {
            break;
        };
        out.push(ch);
        offset += ch.len_utf8();
    }
    out
}

fn looks_like_ooxml_escape(bytes: &[u8]) -> bool {
    bytes.len() >= 7
        && bytes[0] == b'_'
        && matches!(bytes[1], b'x' | b'X')
        && bytes[2..6].iter().all(u8::is_ascii_hexdigit)
        && bytes[6] == b'_'
}

/// Attribute helper.
#[must_use]
pub fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predefined_and_character_references_are_text() {
        let mut reader = XmlReader::new(b"<a>&lt;&gt;&amp;&apos;&quot;&#65;&#x42;</a>");
        let mut text = String::new();
        while let Some(event) = reader.next().unwrap() {
            if let XmlEvent::Text(value) = event {
                text.push_str(&value);
            }
        }
        assert_eq!(text, "<>&'\"AB");
    }

    #[test]
    fn custom_entity_reference_is_rejected() {
        let mut reader = XmlReader::new(b"<a>&custom;</a>");
        assert!(matches!(reader.next(), Ok(Some(XmlEvent::Start { .. }))));
        assert!(reader.next().is_err());
    }

    #[test]
    fn ooxml_text_escapes_controls_and_literal_escape_tokens() {
        let input = "before\u{1}_x0002_after";
        let escaped = escape_ooxml_text(input);
        assert_eq!(escaped, "before_x0001__x005F_x0002_after");
        assert_eq!(decode_ooxml_text(&escaped), input);
    }

    #[test]
    fn attribute_whitespace_survives_xml_normalization() {
        let input = "line one\nline two\rline three\tend";
        let document = format!(r#"<a value="{}"/>"#, escape(input));
        assert!(document.contains("&#xA;"));
        assert!(document.contains("&#xD;"));
        assert!(document.contains("&#x9;"));

        let mut reader = XmlReader::new(document.as_bytes());
        let Some(XmlEvent::Empty { attrs, .. }) = reader.next().unwrap() else {
            panic!("expected one empty element");
        };
        assert_eq!(attr(&attrs, "value"), Some(input));
    }
}
