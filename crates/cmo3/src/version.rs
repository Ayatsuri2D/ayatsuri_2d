//! Schema generations and the Cubism SDK target-version ladder.

use std::fmt;

/// Known CMO3 schema generations, from the `<root fileFormatVersion="...">` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmo3Version {
    V3,
    V4,
    V5,
}

impl Cmo3Version {
    pub fn label(self) -> &'static str {
        match self {
            Cmo3Version::V3 => "cmo3 / Cubism 3.x",
            Cmo3Version::V4 => "cmo3 / Cubism 4.x",
            Cmo3Version::V5 => "cmo3 / Cubism 5.x",
        }
    }

    /// Maps the attribute value to a generation. The attribute is a packed decimal whose leading
    /// digit is the Cubism major generation (401010001 and 400050002 are 4.x, 501030000 is 5.x).
    pub fn from_file_format_version(raw: &str) -> Option<Self> {
        match raw.trim().parse::<i64>().ok()? / 100_000_000 {
            3 => Some(Cmo3Version::V3),
            4 => Some(Cmo3Version::V4),
            5 => Some(Cmo3Version::V5),
            _ => None,
        }
    }
}

impl fmt::Display for Cmo3Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Supported: {}", self.label())
    }
}

/// The SDK version ladder `CModelSource.targetVersionNo` denotes.
///
/// The on-disk value is an era-specific literal, not one formula: 3.x = major*1000 + minor*10,
/// 4.0 = major*100000, 4.2+ = major*1_000_000 + minor*10_000. Corpus evidence:
///
/// | Target | versionNo |
/// |--------|-----------|
/// | 3.0    | 3000      |
/// | 3.3    | 3030      |
/// | 4.0    | 400000    |
/// | 4.2    | 4020000   |
/// | 5.0    | 5000000   |
/// | 5.3    | 5030000   |
/// | SDK N/A| 9000000   |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmo3TargetVersion {
    V30,
    V33,
    V40,
    V42,
    V50,
    V53,
}

impl Cmo3TargetVersion {
    pub const ALL: [Cmo3TargetVersion; 6] = [
        Cmo3TargetVersion::V30,
        Cmo3TargetVersion::V33,
        Cmo3TargetVersion::V40,
        Cmo3TargetVersion::V42,
        Cmo3TargetVersion::V50,
        Cmo3TargetVersion::V53,
    ];

    /// The value the editor writes for "SDK(N/A)/Latest Cubism": no target ceiling. Not an enum
    /// entry because it names the absence of a target version.
    pub const LATEST_VERSION_NO: i32 = 9_000_000;

    pub fn major_minor(self) -> (i32, i32) {
        match self {
            Cmo3TargetVersion::V30 => (3, 0),
            Cmo3TargetVersion::V33 => (3, 3),
            Cmo3TargetVersion::V40 => (4, 0),
            Cmo3TargetVersion::V42 => (4, 2),
            Cmo3TargetVersion::V50 => (5, 0),
            Cmo3TargetVersion::V53 => (5, 3),
        }
    }

    /// The observed on-disk literal.
    pub fn version_no(self) -> i32 {
        match self {
            Cmo3TargetVersion::V30 => 3_000,
            Cmo3TargetVersion::V33 => 3_030,
            Cmo3TargetVersion::V40 => 400_000,
            Cmo3TargetVersion::V42 => 4_020_000,
            Cmo3TargetVersion::V50 => 5_000_000,
            Cmo3TargetVersion::V53 => 5_030_000,
        }
    }

    /// The value under the modern 4.2+ scheme, accepted as a tolerant-read alias.
    fn modern_version_no(self) -> i32 {
        let (major, minor) = self.major_minor();
        major * 1_000_000 + minor * 10_000
    }

    /// Decodes a raw `targetVersionNo`: observed literals first, then modern-scheme equivalents.
    /// None for an absent field, an unknown value, or [`Self::LATEST_VERSION_NO`].
    pub fn from_version_no(version_no: Option<i32>) -> Option<Self> {
        let v = version_no?;
        Self::ALL
            .iter()
            .copied()
            .find(|t| t.version_no() == v)
            .or_else(|| Self::ALL.iter().copied().find(|t| t.modern_version_no() == v))
    }
}

impl fmt::Display for Cmo3TargetVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (major, minor) = self.major_minor();
        write!(f, "Cubism {major}.{minor}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_from_attribute() {
        assert_eq!(Cmo3Version::from_file_format_version("401010001"), Some(Cmo3Version::V4));
        assert_eq!(Cmo3Version::from_file_format_version(" 504000000 "), Some(Cmo3Version::V5));
        assert_eq!(Cmo3Version::from_file_format_version("garbage"), None);
        assert_eq!(Cmo3Version::from_file_format_version("901000000"), None);
    }

    #[test]
    fn target_version_literals_and_aliases() {
        assert_eq!(Cmo3TargetVersion::from_version_no(Some(3030)), Some(Cmo3TargetVersion::V33));
        assert_eq!(Cmo3TargetVersion::from_version_no(Some(400_000)), Some(Cmo3TargetVersion::V40));
        assert_eq!(
            Cmo3TargetVersion::from_version_no(Some(4_000_000)),
            Some(Cmo3TargetVersion::V40)
        );
        assert_eq!(Cmo3TargetVersion::from_version_no(Some(9_000_000)), None);
        assert_eq!(Cmo3TargetVersion::from_version_no(None), None);
    }
}
