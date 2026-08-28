//! BOM detection, encoding sniff, and streaming decode/encode.

use std::io::{self, Read};

use encoding_rs::{Decoder, UTF_16BE, UTF_16LE};
use omacell_core::error::CoreError;

use super::plan::TextEncoding;
use crate::error;

/// How many prefix bytes to skip given a plan's encoding and `bom` flag.
#[must_use]
pub fn plan_bom_skip(encoding: TextEncoding, bom: bool) -> usize {
    if !bom {
        return 0;
    }
    match encoding {
        TextEncoding::Utf8 => 3,
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => 2,
        TextEncoding::Latin1 => 0,
    }
}

/// Byte length of a BOM for `encoding` at the start of `bytes`.
#[must_use]
pub fn bom_len(encoding: TextEncoding, bytes: &[u8]) -> usize {
    match encoding {
        TextEncoding::Utf8 if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) => 3,
        TextEncoding::Utf16Le if bytes.starts_with(&[0xFF, 0xFE]) => 2,
        TextEncoding::Utf16Be if bytes.starts_with(&[0xFE, 0xFF]) => 2,
        _ => 0,
    }
}

/// Encoding implied by a BOM, if any.
#[must_use]
pub fn detect_bom(bytes: &[u8]) -> Option<TextEncoding> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some(TextEncoding::Utf8)
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        Some(TextEncoding::Utf16Le)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some(TextEncoding::Utf16Be)
    } else {
        None
    }
}

/// Guess encoding from a sample. BOM wins; else valid UTF-8; else UTF-16
/// NUL heuristic; else Latin-1.
#[must_use]
pub fn sniff_encoding(bytes: &[u8]) -> (TextEncoding, bool) {
    if let Some(enc) = detect_bom(bytes) {
        return (enc, true);
    }
    if is_utf8(bytes) {
        return (TextEncoding::Utf8, false);
    }
    if let Some(enc) = sniff_utf16(bytes) {
        return (enc, false);
    }
    (TextEncoding::Latin1, false)
}

fn is_utf8(bytes: &[u8]) -> bool {
    let trimmed = trim_utf8_boundary(bytes);
    std::str::from_utf8(trimmed).is_ok()
}

fn trim_utf8_boundary(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] & 0xC0 == 0x80 {
        end -= 1;
    }
    if end > 0 && bytes[end - 1] & 0x80 != 0 {
        end -= 1;
    }
    &bytes[..end]
}

fn sniff_utf16(bytes: &[u8]) -> Option<TextEncoding> {
    if bytes.len() < 8 || bytes.len() % 2 != 0 {
        return None;
    }
    let mut even_nul = 0u32;
    let mut odd_nul = 0u32;
    for (i, b) in bytes.iter().enumerate() {
        if *b == 0 {
            if i % 2 == 0 {
                even_nul += 1;
            } else {
                odd_nul += 1;
            }
        }
    }
    let n = bytes.len() as u32 / 2;
    if odd_nul * 2 >= n && even_nul * 4 < n {
        Some(TextEncoding::Utf16Le)
    } else if even_nul * 2 >= n && odd_nul * 4 < n {
        Some(TextEncoding::Utf16Be)
    } else {
        None
    }
}

/// Decode `bytes` (BOM skipped when present for `encoding`).
pub fn decode_all(bytes: &[u8], encoding: TextEncoding) -> Result<String, CoreError> {
    let skip = bom_len(encoding, bytes);
    let body = &bytes[skip.min(bytes.len())..];
    match encoding {
        TextEncoding::Utf8 => std::str::from_utf8(body)
            .map(ToOwned::to_owned)
            .map_err(|_| error::encoding("input is not valid UTF-8")),
        TextEncoding::Latin1 => Ok(body.iter().map(|&b| b as char).collect()),
        TextEncoding::Utf16Le => decode_utf16(body, UTF_16LE),
        TextEncoding::Utf16Be => decode_utf16(body, UTF_16BE),
    }
}

fn decode_utf16(bytes: &[u8], enc: &'static encoding_rs::Encoding) -> Result<String, CoreError> {
    let (cow, _had_errors) = enc.decode_without_bom_handling(bytes);
    Ok(cow.into_owned())
}

/// Encode `text` in `encoding`. UTF-16 always writes a BOM when `bom` is true;
/// UTF-8 writes EF BB BF when `bom` is true. Latin-1 rejects non-Latin-1 chars.
pub fn encode_all(text: &str, encoding: TextEncoding, bom: bool) -> Result<Vec<u8>, CoreError> {
    match encoding {
        TextEncoding::Utf8 => {
            let mut out = Vec::with_capacity(text.len() + 3);
            if bom {
                out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            }
            out.extend_from_slice(text.as_bytes());
            Ok(out)
        }
        TextEncoding::Latin1 => {
            let mut out = Vec::with_capacity(text.len());
            for c in text.chars() {
                let u = u32::from(c);
                if u > 0xFF {
                    return Err(error::encoding(format!(
                        "character U+{u:04X} cannot be encoded as Latin-1"
                    )));
                }
                out.push(u as u8);
            }
            Ok(out)
        }
        TextEncoding::Utf16Le => Ok(encode_utf16(text, true, bom)),
        TextEncoding::Utf16Be => Ok(encode_utf16(text, false, bom)),
    }
}

fn encode_utf16(text: &str, little: bool, bom: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 2 + 2);
    if bom {
        if little {
            out.extend_from_slice(&[0xFF, 0xFE]);
        } else {
            out.extend_from_slice(&[0xFE, 0xFF]);
        }
    }
    for unit in text.encode_utf16() {
        let bytes = if little {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        };
        out.extend_from_slice(&bytes);
    }
    out
}

/// `Read` adapter that yields UTF-8 bytes.
pub struct DecodingReader<R: Read> {
    inner: R,
    encoding: TextEncoding,
    decoder: Option<Decoder>,
    pending: Vec<u8>,
    out: Vec<u8>,
    out_pos: usize,
    skip: usize,
    eof: bool,
}

impl<R: Read> DecodingReader<R> {
    /// Wrap `inner`. `skip` is BOM bytes still to consume from the stream.
    pub fn new(inner: R, encoding: TextEncoding, skip: usize) -> Self {
        let decoder = match encoding {
            TextEncoding::Utf16Le => Some(UTF_16LE.new_decoder_without_bom_handling()),
            TextEncoding::Utf16Be => Some(UTF_16BE.new_decoder_without_bom_handling()),
            TextEncoding::Utf8 | TextEncoding::Latin1 => None,
        };
        Self {
            inner,
            encoding,
            decoder,
            pending: Vec::new(),
            out: Vec::new(),
            out_pos: 0,
            skip,
            eof: false,
        }
    }
}

impl<R: Read> Read for DecodingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.out_pos < self.out.len() {
                let n = (self.out.len() - self.out_pos).min(buf.len());
                buf[..n].copy_from_slice(&self.out[self.out_pos..self.out_pos + n]);
                self.out_pos += n;
                if self.out_pos == self.out.len() {
                    self.out.clear();
                    self.out_pos = 0;
                }
                return Ok(n);
            }
            if self.eof && self.pending.is_empty() {
                return Ok(0);
            }
            self.fill()?;
            if self.out.is_empty() && self.eof {
                return Ok(0);
            }
        }
    }
}

impl<R: Read> DecodingReader<R> {
    fn fill(&mut self) -> io::Result<()> {
        while self.skip > 0 && !self.eof {
            let mut tmp = [0u8; 4];
            let want = self.skip.min(tmp.len());
            let n = self.inner.read(&mut tmp[..want])?;
            if n == 0 {
                self.eof = true;
                break;
            }
            self.skip -= n;
        }
        if self.skip > 0 && self.eof {
            return Ok(());
        }
        match self.encoding {
            TextEncoding::Utf8 => self.fill_utf8(),
            TextEncoding::Latin1 => self.fill_latin1(),
            TextEncoding::Utf16Le | TextEncoding::Utf16Be => self.fill_utf16(),
        }
    }

    fn fill_utf8(&mut self) -> io::Result<()> {
        let mut tmp = [0u8; 8192];
        let n = self.inner.read(&mut tmp)?;
        if n == 0 {
            self.eof = true;
            return Ok(());
        }
        self.out.extend_from_slice(&tmp[..n]);
        Ok(())
    }

    fn fill_latin1(&mut self) -> io::Result<()> {
        let mut tmp = [0u8; 4096];
        let n = self.inner.read(&mut tmp)?;
        if n == 0 {
            self.eof = true;
            return Ok(());
        }
        self.out.reserve(n * 2);
        for &b in &tmp[..n] {
            if b < 0x80 {
                self.out.push(b);
            } else {
                self.out.push(0xC0 | (b >> 6));
                self.out.push(0x80 | (b & 0x3F));
            }
        }
        Ok(())
    }

    fn fill_utf16(&mut self) -> io::Result<()> {
        let decoder = self
            .decoder
            .as_mut()
            .ok_or_else(|| io::Error::other("utf-16 decoder missing"))?;
        if !self.eof {
            let mut tmp = [0u8; 8192];
            let n = self.inner.read(&mut tmp)?;
            if n == 0 {
                self.eof = true;
            } else {
                self.pending.extend_from_slice(&tmp[..n]);
            }
        }
        if self.pending.is_empty() && !self.eof {
            return Ok(());
        }
        let mut dst = [0u8; 8192];
        let (result, read, written, _) = decoder.decode_to_utf8(&self.pending, &mut dst, self.eof);
        self.out.extend_from_slice(&dst[..written]);
        self.pending.drain(..read);
        match result {
            encoding_rs::CoderResult::InputEmpty | encoding_rs::CoderResult::OutputFull => Ok(()),
        }
    }
}

/// Counts bytes consumed from `inner`.
pub struct CountingReader<R: Read> {
    inner: R,
    /// Bytes read so far.
    pub bytes: u64,
}

impl<R: Read> CountingReader<R> {
    /// Wrap `inner`.
    pub fn new(inner: R) -> Self {
        Self { inner, bytes: 0 }
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes += n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16le_round_trip() {
        let src = "1,2\n3,4\n";
        let bytes = encode_all(src, TextEncoding::Utf16Le, true).unwrap();
        assert!(bytes.starts_with(&[0xFF, 0xFE]));
        let back = decode_all(&bytes, TextEncoding::Utf16Le).unwrap();
        assert_eq!(back, src);
        let mut r = DecodingReader::new(std::io::Cursor::new(bytes), TextEncoding::Utf16Le, 2);
        let mut out = String::new();
        r.read_to_string(&mut out).unwrap();
        assert_eq!(out, src);
    }
}
