#![no_std]

/// Maximum encoded path accepted by the shared matcher.
pub const MAX_PATTERN_BYTES: usize = 192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PatternError {
    Empty,
    TooLong,
    InvalidPath,
    TrailingEscape,
    UnterminatedClass,
    EmptyClass,
    InvalidRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pattern<'a> {
    value: &'a str,
    magic: bool,
}

impl<'a> Pattern<'a> {
    pub fn parse(value: &'a str) -> Result<Self, PatternError> {
        let mut magic = false;
        // C borrows this string and writes only the supplied boolean.
        pattern_result(unsafe {
            ghostos_pattern_parse(value.as_ptr(), value.len(), &mut magic)
        })?;
        Ok(Self { value, magic })
    }

    pub const fn as_str(self) -> &'a str {
        self.value
    }

    pub const fn has_magic(self) -> bool {
        self.magic
    }

    /// Match one complete path. Star and question mark never cross a path separator.
    /// Matching is byte-exact except that question mark consumes one UTF-8 character.
    pub fn matches(self, candidate: &str) -> bool {
        // C only reads the two borrowed UTF-8 slices during this call.
        unsafe {
            ghostos_pattern_matches(self.value.as_ptr(), self.value.len(),
                candidate.as_ptr(), candidate.len())
        }
    }

    /// Match one directory component.
    pub fn matches_component(self, candidate: &str) -> bool {
        self.matches(candidate)
    }
}

/// Copy a pattern into a path buffer while removing escape markers.
pub fn unescape(pattern: &str, output: &mut [u8]) -> Result<usize, PatternError> {
    let mut written = 0;
    // C writes at most output.len() bytes and retains no pointers.
    pattern_result(unsafe {
        ghostos_pattern_unescape(pattern.as_ptr(), pattern.len(),
            output.as_mut_ptr(), output.len(), &mut written)
    })?;
    Ok(written)
}

unsafe extern "C" {
    fn ghostos_pattern_parse(pattern: *const u8, length: usize, magic: *mut bool) -> u32;
    fn ghostos_pattern_matches(pattern: *const u8, length: usize,
        candidate: *const u8, candidate_length: usize) -> bool;
    fn ghostos_pattern_unescape(pattern: *const u8, length: usize,
        output: *mut u8, capacity: usize, written: *mut usize) -> u32;
}

fn pattern_result(code: u32) -> Result<(), PatternError> {
    match code {
        0 => Ok(()),
        1 => Err(PatternError::Empty),
        2 => Err(PatternError::TooLong),
        3 => Err(PatternError::InvalidPath),
        4 => Err(PatternError::TrailingEscape),
        5 => Err(PatternError::UnterminatedClass),
        6 => Err(PatternError::EmptyClass),
        _ => Err(PatternError::InvalidRange),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_components_without_crossing_separator() {
        assert!(Pattern::parse("/data/*.txt").unwrap().matches("/data/a.txt"));
        assert!(!Pattern::parse("/data/*.txt").unwrap().matches("/data/x/a.txt"));
        assert!(Pattern::parse("/data/file?.txt").unwrap().matches("/data/file1.txt"));
    }

    #[test]
    fn matches_classes_and_escapes() {
        assert!(Pattern::parse("/data/[a-c].txt").unwrap().matches("/data/b.txt"));
        assert!(Pattern::parse("/data/[!a].txt").unwrap().matches("/data/b.txt"));
        assert!(Pattern::parse(r#"/data/\*.txt"#).unwrap().matches("/data/*.txt"));
    }

    #[test]
    fn rejects_malformed_patterns() {
        assert_eq!(Pattern::parse("/data/[abc"), Err(PatternError::UnterminatedClass));
        assert_eq!(Pattern::parse("/data/foo\\"), Err(PatternError::TrailingEscape));
        assert_eq!(Pattern::parse("/data/[z-a]"), Err(PatternError::InvalidRange));
        assert_eq!(Pattern::parse("/data/[]"), Err(PatternError::EmptyClass));
        assert_eq!(Pattern::parse("/data/foo/"), Err(PatternError::InvalidPath));
    }

    #[test]
    fn matcher_handles_boundaries_and_utf8_without_crossing_components() {
        assert!(Pattern::parse("*").unwrap().matches(".hidden"));
        assert!(Pattern::parse("?").unwrap().matches("é"));
        assert!(!Pattern::parse("?").unwrap().matches("éé"));
        assert!(!Pattern::parse("*").unwrap().matches("nested/name"));
        assert!(Pattern::parse(r#"/data/\*.txt"#).unwrap().matches("/data/*.txt"));
        assert!(!Pattern::parse(r#"/data/\*.txt"#).unwrap().matches("/data/a.txt"));
    }

    #[test]
    fn matcher_rejects_patterns_at_the_encoded_path_boundary() {
        let value = [b'a'; MAX_PATTERN_BYTES];
        let too_long = [b'a'; MAX_PATTERN_BYTES + 1];
        assert!(Pattern::parse(core::str::from_utf8(&value).unwrap()).is_ok());
        assert_eq!(
            Pattern::parse(core::str::from_utf8(&too_long).unwrap()),
            Err(PatternError::InvalidPath)
        );
    }

    #[test]
    fn property_wildcards_never_cross_a_separator() {
        use ghostos_test_support::property::{ascii, run_assert, Config};

        run_assert("path-pattern.wildcard-separator", Config::new(0x59_3, 128), |_, _, entropy| {
            let name = ascii(entropy, 32);
            let Ok(pattern) = Pattern::parse("*") else { return false };
            pattern.matches(&name) && !pattern.matches("a/b")
        })
        .expect("generated path cases satisfy wildcard contract");
    }

    #[test]
    fn property_unescape_preserves_literal_bytes() {
        use ghostos_test_support::property::{ascii, run_assert, Config};

        run_assert("path-pattern.unescape", Config::new(0x59_3, 128), |_, _, entropy| {
            let value = ascii(entropy, 32);
            let mut output = [0; MAX_PATTERN_BYTES];
            let Ok(length) = unescape(&value, &mut output) else { return false };
            &output[..length] == value.as_bytes()
        })
        .expect("generated literal paths round-trip through unescape");
    }
}
