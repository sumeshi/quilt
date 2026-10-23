//! Resolve a serde-style YAML path to the source position of its value.
//!
//! This walks the YAML with a small `DeserializeSeed` visitor, then asks the
//! YAML deserializer to report an intentional type error at the selected
//! value. The error contains a source location without exposing the value.

use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::cell::Cell;
use std::fmt;

#[derive(Debug, PartialEq, Eq)]
enum Segment {
    Key(String),
    Index(usize),
}

fn parse_path(path: &str) -> Option<Vec<Segment>> {
    let bytes = path.as_bytes();
    let mut parts = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'.' => i += 1,
            b'[' => {
                let end = path[i + 1..].find(']')? + i + 1;
                let item = &path[i + 1..end];
                if let Ok(index) = item.parse::<usize>() {
                    parts.push(Segment::Index(index));
                } else {
                    let key = item
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .or_else(|| item.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))?;
                    parts.push(Segment::Key(key.to_owned()));
                }
                i = end + 1;
            }
            _ => {
                let start = i;
                while i < bytes.len() && bytes[i] != b'.' && bytes[i] != b'[' {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                parts.push(Segment::Key(path[start..i].to_owned()));
            }
        }
    }
    Some(parts)
}

struct Locate<'a> {
    path: &'a [Segment],
    reached: &'a Cell<bool>,
}

impl<'de> DeserializeSeed<'de> for Locate<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        if self.path.is_empty() {
            self.reached.set(true);
            deserializer.deserialize_any(ProbeVisitor)
        } else {
            deserializer.deserialize_any(LocateVisitor {
                path: self.path,
                reached: self.reached,
            })
        }
    }
}

struct ProbeVisitor;

impl<'de> Visitor<'de> for ProbeVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a value at the requested YAML path")
    }
}

struct LocateVisitor<'a> {
    path: &'a [Segment],
    reached: &'a Cell<bool>,
}

impl<'de> Visitor<'de> for LocateVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the requested YAML value")
    }

    fn visit_map<A>(self, mut map: A) -> Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        let Some((Segment::Key(wanted), rest)) = self.path.split_first() else {
            return Err(de::Error::custom("path not found"));
        };
        while let Some(key) = map.next_key::<String>()? {
            if &key == wanted {
                return map.next_value_seed(Locate {
                    path: rest,
                    reached: self.reached,
                });
            }
            map.next_value::<IgnoredAny>()?;
        }
        Err(de::Error::custom("path not found"))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        let Some((Segment::Index(wanted), rest)) = self.path.split_first() else {
            return Err(de::Error::custom("path not found"));
        };
        for index in 0..=*wanted {
            if index == *wanted {
                return seq
                    .next_element_seed(Locate {
                        path: rest,
                        reached: self.reached,
                    })?
                    .ok_or_else(|| de::Error::custom("path not found"));
            }
            if seq.next_element::<IgnoredAny>()?.is_none() {
                return Err(de::Error::custom("path not found"));
            }
        }
        Err(de::Error::custom("path not found"))
    }
}

/// Return the one-based YAML line and column for `path`, if that path resolves.
///
/// The function only exposes coordinates. It never includes the selected YAML
/// value or a source excerpt in its result.
pub(super) fn source_location(raw: &str, path: &str) -> Option<(usize, usize)> {
    let parts = parse_path(path)?;
    let deserializer = yaml_serde::Deserializer::from_str(raw);
    let reached = Cell::new(false);
    let error = Locate {
        path: &parts,
        reached: &reached,
    }
    .deserialize(deserializer)
    .err()?;
    if !reached.get() {
        return None;
    }
    error
        .location()
        .map(|location| (location.line(), location.column()))
}

#[cfg(test)]
mod tests {
    use super::source_location;

    #[test]
    fn resolves_block_flow_unicode_quoted_alias_and_missing_paths() {
        let block = "run:\n  stages:\n    - name: café\n      steps:\n        - bucket:\n            column: when\n            interval: [bad]\n";
        assert_eq!(
            source_location(block, "run.stages[0].steps[0].bucket.interval"),
            Some((7, 23))
        );

        let flow = "run: {stages: [{steps: [{bucket: {interval: [bad]}}]}]}";
        assert_eq!(
            source_location(flow, "run.stages[0].steps[0].bucket.interval"),
            Some((1, 45))
        );

        let quoted = "'run': {'stages': [{'steps': [{'bucket': {'strict': nope}}]}]}";
        assert_eq!(
            source_location(quoted, "run.stages[0].steps[0].bucket.strict"),
            Some((1, 53))
        );

        let alias = "shared: &opts {strict: nope}\nrun: {stages: [{steps: [{bucket: *opts}]}]}\n";
        assert!(source_location(alias, "run.stages[0].steps[0].bucket.strict").is_some());
        assert_eq!(
            source_location(block, "run.stages[1].steps[0].bucket.interval"),
            None
        );
    }
}
