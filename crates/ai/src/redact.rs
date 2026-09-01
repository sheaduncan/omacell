//! Pattern detectors and `[REDACTED:kind]` placeholders.

use regex::Regex;
use serde_json::{Map, Value};
use std::sync::OnceLock;

/// Detector kind, stable in placeholders.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Kind {
    /// Email address.
    Email,
    /// Telephone.
    Phone,
    /// Payment-card-like number (Luhn).
    Card,
    /// National-ID shape (`000-00-0000`).
    NationalId,
    /// IBAN.
    Iban,
}

impl Kind {
    /// Placeholder token.
    #[must_use]
    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Email => "[REDACTED:email]",
            Self::Phone => "[REDACTED:phone]",
            Self::Card => "[REDACTED:card]",
            Self::NationalId => "[REDACTED:national-id]",
            Self::Iban => "[REDACTED:iban]",
        }
    }

    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Phone => "phone",
            Self::Card => "card",
            Self::NationalId => "national-id",
            Self::Iban => "iban",
        }
    }
}

/// One suggested (or applied) redaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// Detector.
    pub kind: Kind,
    /// Original snippet.
    pub sample: String,
}

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| compile_static(r"(?i)[A-Z0-9._%+\-]+@[A-Z0-9.\-]+\.[A-Z]{2,}"))
}

fn national_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| compile_static(r"\b\d{3}-\d{2}-\d{4}\b"))
}

fn iban_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| compile_static(r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b"))
}

fn card_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| compile_static(r"\b(?:\d[ \-]?){13,19}\b"))
}

fn phone_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        compile_static(r"(?:\+\d{1,3}[\s\-.]?)?(?:\(?\d{2,4}\)?[\s\-.]?)?\d{3,4}[\s\-.]?\d{4}")
    })
}

fn compile_static(pattern: &'static str) -> Regex {
    match Regex::new(pattern) {
        Ok(regex) => regex,
        Err(error) => unreachable!("invalid built-in redaction pattern {pattern:?}: {error}"),
    }
}

/// Redact detected secrets in `text`.
#[must_use]
pub fn redact_text(text: &str) -> (String, Vec<Suggestion>) {
    let mut suggestions = Vec::new();
    let mut out = email_re()
        .replace_all(text, |_caps: &regex::Captures<'_>| {
            suggestions.push(Suggestion {
                kind: Kind::Email,
                sample: Kind::Email.placeholder().to_string(),
            });
            Kind::Email.placeholder().to_string()
        })
        .into_owned();
    out = national_re()
        .replace_all(&out, |_caps: &regex::Captures<'_>| {
            suggestions.push(Suggestion {
                kind: Kind::NationalId,
                sample: Kind::NationalId.placeholder().to_string(),
            });
            Kind::NationalId.placeholder().to_string()
        })
        .into_owned();
    out = iban_re()
        .replace_all(&out, |_caps: &regex::Captures<'_>| {
            suggestions.push(Suggestion {
                kind: Kind::Iban,
                sample: Kind::Iban.placeholder().to_string(),
            });
            Kind::Iban.placeholder().to_string()
        })
        .into_owned();
    out = card_re()
        .replace_all(&out, |caps: &regex::Captures<'_>| {
            let digits: String = caps[0].chars().filter(|c| c.is_ascii_digit()).collect();
            if (13..=19).contains(&digits.len()) && luhn(&digits) {
                suggestions.push(Suggestion {
                    kind: Kind::Card,
                    sample: Kind::Card.placeholder().to_string(),
                });
                Kind::Card.placeholder().to_string()
            } else {
                caps[0].to_string()
            }
        })
        .into_owned();
    out = phone_re()
        .replace_all(&out, |caps: &regex::Captures<'_>| {
            let digits = caps[0].chars().filter(|c| c.is_ascii_digit()).count();
            if (10..=15).contains(&digits) {
                suggestions.push(Suggestion {
                    kind: Kind::Phone,
                    sample: Kind::Phone.placeholder().to_string(),
                });
                Kind::Phone.placeholder().to_string()
            } else {
                caps[0].to_string()
            }
        })
        .into_owned();
    (out, suggestions)
}

fn luhn(digits: &str) -> bool {
    let mut sum = 0u32;
    let mut alt = false;
    for c in digits.chars().rev() {
        let mut n = u32::from(c) - u32::from('0');
        if alt {
            n *= 2;
            if n > 9 {
                n -= 9;
            }
        }
        sum += n;
        alt = !alt;
    }
    sum.is_multiple_of(10)
}

/// Walk a JSON document and redact every string, including literal text in formulas.
pub fn redact_json(value: &mut Value) -> Vec<Suggestion> {
    let mut suggestions = Vec::new();
    redact_value(value, &mut suggestions);
    suggestions
}

fn redact_value(value: &mut Value, suggestions: &mut Vec<Suggestion>) {
    match value {
        Value::String(text) => {
            let (next, found) = redact_text(text);
            suggestions.extend(found);
            *text = next;
        }
        Value::Array(items) => {
            for item in items {
                redact_value(item, suggestions);
            }
        }
        Value::Object(map) => redact_object(map, suggestions),
        _ => {}
    }
}

fn redact_object(map: &mut Map<String, Value>, suggestions: &mut Vec<Suggestion>) {
    for value in map.values_mut() {
        redact_value(value, suggestions);
    }
}
