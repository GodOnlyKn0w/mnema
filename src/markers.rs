//! Marker vocabulary and parsing.
//!
//! This module is the single owner for bracket-marker spelling, classes, and
//! leading-marker parsing. Callers should ask this module for marker intent
//! instead of duplicating string rules.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkerClass {
    Judgment,
    Observation,
    Planning,
    Structure,
    Annotation,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MarkerSpec {
    pub(crate) spelling: &'static str,
    pub(crate) class: MarkerClass,
}

const MARKERS: &[MarkerSpec] = &[
    MarkerSpec {
        spelling: "[decision]",
        class: MarkerClass::Judgment,
    },
    MarkerSpec {
        spelling: "[constraint]",
        class: MarkerClass::Judgment,
    },
    MarkerSpec {
        spelling: "[friction]",
        class: MarkerClass::Judgment,
    },
    MarkerSpec {
        spelling: "[fixed]",
        class: MarkerClass::Judgment,
    },
    MarkerSpec {
        spelling: "[lesson]",
        class: MarkerClass::Judgment,
    },
    MarkerSpec {
        spelling: "[insight]",
        class: MarkerClass::Judgment,
    },
    MarkerSpec {
        spelling: "[observed]",
        class: MarkerClass::Observation,
    },
    MarkerSpec {
        spelling: "[check]",
        class: MarkerClass::Observation,
    },
    MarkerSpec {
        spelling: "[progress]",
        class: MarkerClass::Observation,
    },
    MarkerSpec {
        spelling: "[deliverable]",
        class: MarkerClass::Observation,
    },
    MarkerSpec {
        spelling: "[metric]",
        class: MarkerClass::Observation,
    },
    MarkerSpec {
        spelling: "[deadline]",
        class: MarkerClass::Planning,
    },
    MarkerSpec {
        spelling: "[covers]",
        class: MarkerClass::Structure,
    },
    MarkerSpec {
        spelling: "[guide]",
        class: MarkerClass::Structure,
    },
    MarkerSpec {
        spelling: "[skill]",
        class: MarkerClass::Structure,
    },
    MarkerSpec {
        spelling: "[task]",
        class: MarkerClass::Structure,
    },
    MarkerSpec {
        spelling: "[session]",
        class: MarkerClass::Structure,
    },
    MarkerSpec {
        spelling: "[done]",
        class: MarkerClass::Annotation,
    },
    MarkerSpec {
        spelling: "[verified]",
        class: MarkerClass::Annotation,
    },
    MarkerSpec {
        spelling: "[cancelled]",
        class: MarkerClass::Annotation,
    },
    MarkerSpec {
        spelling: "[failed]",
        class: MarkerClass::Annotation,
    },
    MarkerSpec {
        spelling: "[merged]",
        class: MarkerClass::Annotation,
    },
    MarkerSpec {
        spelling: "[ended]",
        class: MarkerClass::Annotation,
    },
    MarkerSpec {
        spelling: "[dispatched]",
        class: MarkerClass::Annotation,
    },
    MarkerSpec {
        spelling: "[registered]",
        class: MarkerClass::Annotation,
    },
    MarkerSpec {
        spelling: "[checkpoint]",
        class: MarkerClass::System,
    },
    MarkerSpec {
        spelling: "[hidden]",
        class: MarkerClass::System,
    },
    MarkerSpec {
        spelling: "[waiting:human]",
        class: MarkerClass::System,
    },
    MarkerSpec {
        spelling: "[grill]",
        class: MarkerClass::System,
    },
];

#[cfg(test)]
const MARKER_SPELLINGS: &[&str] = &[
    "[decision]",
    "[constraint]",
    "[friction]",
    "[fixed]",
    "[lesson]",
    "[insight]",
    "[observed]",
    "[check]",
    "[progress]",
    "[deliverable]",
    "[metric]",
    "[deadline]",
    "[covers]",
    "[guide]",
    "[skill]",
    "[task]",
    "[session]",
    "[done]",
    "[verified]",
    "[cancelled]",
    "[failed]",
    "[merged]",
    "[ended]",
    "[dispatched]",
    "[registered]",
    "[checkpoint]",
    "[hidden]",
    "[waiting:human]",
    "[grill]",
];

pub(crate) fn known() -> &'static [MarkerSpec] {
    MARKERS
}

#[cfg(test)]
pub(crate) fn known_marker_spellings() -> &'static [&'static str] {
    MARKER_SPELLINGS
}

pub(crate) fn is_known_marker_str(marker: &str) -> bool {
    classify(marker).is_some()
}

pub(crate) fn classify(marker: &str) -> Option<MarkerClass> {
    MARKERS
        .iter()
        .find(|m| m.spelling == marker)
        .map(|m| m.class)
}

pub(crate) fn validate_lifecycle_marker(_content: &str) -> Result<(), String> {
    // All bracket-prefixed content is accepted. Unknown markers are handled by
    // the W073 warning path in append after the write succeeds.
    Ok(())
}

pub(crate) fn leading_marker(content: &str) -> Option<&str> {
    let trimmed = content.trim_start();
    let rest = trimmed.strip_prefix('[')?;
    let end = rest.find(']')?;
    let token = &rest[..end];
    if token.is_empty() { None } else { Some(token) }
}

pub(crate) fn split_marker(content: &str) -> (&str, &str) {
    let trimmed = content.trim_start();
    let Some(rest) = trimmed.strip_prefix('[') else {
        return ("", content);
    };
    let Some(end) = rest.find(']') else {
        return ("", content);
    };
    if end == 0 {
        return ("", content);
    }
    let marker_end = end + 2;
    (&trimmed[..marker_end], trimmed[marker_end..].trim())
}

#[cfg(test)]
pub(crate) fn extract_from_text(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'[' {
            if i + 1 < len && bytes[i + 1].is_ascii_lowercase() {
                let start = i;
                let mut j = i + 1;
                while j < len {
                    let b = bytes[j];
                    if b.is_ascii_alphanumeric() || b == b'_' || b == b':' || b == b'-' {
                        j += 1;
                    } else {
                        break;
                    }
                }
                if j < len && bytes[j] == b']' {
                    out.push(s[start..=j].to_string());
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

pub(crate) fn is_closing_annotation_marker(content: &str) -> bool {
    let (marker, _) = split_marker(content);
    matches!(classify(marker), Some(MarkerClass::Annotation))
        && matches!(
            marker,
            "[done]" | "[failed]" | "[cancelled]" | "[merged]" | "[verified]"
        )
}

pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

pub(crate) fn suggest_marker(marker: &str) -> Option<&'static str> {
    let inner = marker.trim_start_matches('[').trim_end_matches(']');
    if inner.chars().any(|c| !c.is_alphabetic() && c != ':') {
        return None;
    }
    let mut best_dist = usize::MAX;
    let mut best_marker: Option<&'static str> = None;
    for spec in known() {
        let known = spec.spelling;
        let known_inner = known.trim_start_matches('[').trim_end_matches(']');
        let dist = levenshtein(inner, known_inner);
        if dist == 0 {
            return None;
        }
        if dist < best_dist {
            best_dist = dist;
            best_marker = Some(known);
        }
    }
    if best_dist <= 2 { best_marker } else { None }
}
