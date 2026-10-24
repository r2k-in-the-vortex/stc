//! Stable parser diagnostic codes (Grammar §13 and D-047).

/// An unsupported body language (IL/LD/FBD) was recognized.
pub const UNSUPPORTED_BODY: &str = "STC-E-GRAM-101";
/// SFC declarations or statements are not in the profile.
pub const SFC_UNSUPPORTED: &str = "STC-E-GRAM-102";
/// Preprocessor directives are not in the profile.
pub const NO_PREPROCESSOR: &str = "STC-E-GRAM-103";
/// A known out-of-profile construct (properties, mutexes, `ASSERT`) was found.
pub const NOT_IN_PROFILE: &str = "STC-E-GRAM-104";
/// A token does not fit any grammar position.
pub const UNEXPECTED_TOKEN: &str = "STC-E-GRAM-105";
/// A specific token or construct was expected.
pub const EXPECTED_TOKEN: &str = "STC-E-GRAM-106";
/// A statement or declaration is missing its semicolon.
pub const MISSING_SEMICOLON: &str = "STC-E-GRAM-107";
/// A block ended with a terminator belonging to another construct.
pub const MISMATCHED_END: &str = "STC-E-GRAM-108";
/// Nesting exceeded the parser depth limit.
pub const DEPTH_LIMIT: &str = "STC-E-GRAM-109";
/// A pragma appeared where the grammar does not allow it.
pub const PRAGMA_PLACEMENT: &str = "STC-E-GRAM-110";
/// A block terminator is missing.
pub const MISSING_END: &str = "STC-E-GRAM-111";
