//! Package versions: git tags read as `MAJOR.MINOR.PATCH`, and `^` ranges.
//!
//! A tag is a version when, after an optional leading `v`, it is one to three
//! numbers joined by dots (`v0.4` is 0.4.0, `1` is 1.0.0). A tag with a
//! suffix (`v1.0.0-rc1`) or anything else is not a version: a range never
//! picks it, but an exact `tag = "..."` can still name it.
//!
//! `^1.2.3` is any version from 1.2.3 up to, not including, 2.0.0. The
//! leftmost number that is not zero is the one that may not change: `^0.4`
//! is 0.4.0 up to 0.5.0, `^0.0.3` is exactly 0.0.3, `^0` is below 1.0.0.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The numbers of `text` (without `v`) and how many were written.
fn numbers(text: &str) -> Option<(Version, usize)> {
    let mut parts = [0u64; 3];
    let mut count = 0;
    for piece in text.split('.') {
        if count == 3 || piece.is_empty() || !piece.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        parts[count] = piece.parse().ok()?;
        count += 1;
    }
    Some((Version { major: parts[0], minor: parts[1], patch: parts[2] }, count))
}

impl Version {
    /// A tag as a version, if it is one.
    pub fn from_tag(tag: &str) -> Option<Version> {
        numbers(tag.strip_prefix('v').unwrap_or(tag)).map(|(v, _)| v)
    }
}

/// A `^` range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    low: Version,
    /// Exclusive upper bound.
    high: Version,
}

impl Range {
    /// Reads `^0.4`, `^1.2.3` (a leading `v` after the `^` is fine).
    pub fn parse(text: &str) -> Option<Range> {
        let rest = text.trim().strip_prefix('^')?;
        let (low, n) = numbers(rest.strip_prefix('v').unwrap_or(rest))?;
        let v = |major, minor, patch| Version { major, minor, patch };
        let high = if low.major > 0 || n == 1 {
            v(low.major + 1, 0, 0)
        } else if low.minor > 0 || n == 2 {
            v(0, low.minor + 1, 0)
        } else {
            v(0, 0, low.patch + 1)
        };
        Some(Range { low, high })
    }

    pub fn contains(&self, v: Version) -> bool {
        self.low <= v && v < self.high
    }

    /// Whether the tag is a version inside this range.
    pub fn allows_tag(&self, tag: &str) -> bool {
        Version::from_tag(tag).is_some_and(|v| self.contains(v))
    }
}

/// The newest tag of `tags` inside `range`.
pub fn newest_in<'a>(tags: impl IntoIterator<Item = &'a str>, range: &Range) -> Option<&'a str> {
    tags.into_iter().filter_map(|t| Version::from_tag(t).map(|v| (v, t))).filter(|(v, _)| range.contains(*v)).max().map(|(_, t)| t)
}

/// The newest tag that is a version at all.
pub fn newest<'a>(tags: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    tags.into_iter().filter_map(|t| Version::from_tag(t).map(|v| (v, t))).max().map(|(_, t)| t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(range: &str, tags: &[&str]) -> Option<String> {
        newest_in(tags.iter().copied(), &Range::parse(range).unwrap()).map(str::to_string)
    }

    #[test]
    fn tags_are_versions() {
        assert_eq!(Version::from_tag("v0.4"), Some(Version { major: 0, minor: 4, patch: 0 }));
        assert_eq!(Version::from_tag("1.2.3"), Some(Version { major: 1, minor: 2, patch: 3 }));
        assert_eq!(Version::from_tag("v1.0.0-rc1"), None);
        assert_eq!(Version::from_tag("latest"), None);
        assert_eq!(Version::from_tag("v1.2.3.4"), None);
    }

    #[test]
    fn caret_ranges() {
        let tags = ["v0.3.9", "v0.4.0", "v0.4.7", "v0.5.0", "v1.0.0", "v1.4.2", "v2.0.0", "nightly", "v1.5.0-rc1"];
        assert_eq!(pick("^0.4", &tags).as_deref(), Some("v0.4.7"));
        assert_eq!(pick("^1", &tags).as_deref(), Some("v1.4.2"));
        assert_eq!(pick("^1.0.0", &tags).as_deref(), Some("v1.4.2"));
        assert_eq!(pick("^0", &tags).as_deref(), Some("v0.5.0"));
        assert_eq!(pick("^0.0.3", &["v0.0.3", "v0.0.4"]).as_deref(), Some("v0.0.3"));
        assert_eq!(pick("^3", &tags), None);
        assert_eq!(pick("^0.4.8", &tags), None);
        assert!(Range::parse("0.4").is_none());
        assert!(Range::parse("^x").is_none());
    }

    #[test]
    fn newest_ignores_non_versions() {
        assert_eq!(newest(["v1.0.0", "v1.10.0", "v1.9.0", "junk"]), Some("v1.10.0"));
    }
}
