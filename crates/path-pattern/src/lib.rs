#![no_std]
#![forbid(unsafe_code)]

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
        let bytes = value.as_bytes();
        if bytes.is_empty() {
            return Err(PatternError::Empty)
        }
        if bytes.len() > MAX_PATTERN_BYTES
            || bytes.contains(&0)
            || bytes.ends_with(b"/") && value != "/"
            || bytes.windows(2).any(|pair| pair == b"//")
        {
            return Err(PatternError::InvalidPath)
        }
        let mut magic = false;
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\\' => {
                    index += 1;
                    if index == bytes.len() {
                        return Err(PatternError::TrailingEscape)
                    }
                    index += 1;
                }
                b'*' | b'?' => {
                    magic = true;
                    index += 1;
                }
                b'[' => {
                    let end = class_end(bytes, index)?;
                    if match_class(&bytes[index..], &[0]).is_none() {
                        return Err(PatternError::InvalidRange)
                    }
                    magic = true;
                    index = end + 1;
                }
                _ => index += 1,
            }
        }
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
        match_bytes(self.value.as_bytes(), candidate.as_bytes())
    }

    /// Match one directory component.
    pub fn matches_component(self, candidate: &str) -> bool {
        self.matches(candidate)
    }
}

/// Copy a pattern into a path buffer while removing escape markers.
pub fn unescape(pattern: &str, output: &mut [u8]) -> Result<usize, PatternError> {
    let bytes = pattern.as_bytes();
    let mut written = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 1;
            if index == bytes.len() {
                return Err(PatternError::TrailingEscape)
            }
        }
        if written == output.len() {
            return Err(PatternError::TooLong)
        }
        output[written] = bytes[index];
        written += 1;
        index += 1;
    }
    Ok(written)
}

fn class_end(pattern: &[u8], start: usize) -> Result<usize, PatternError> {
    let mut index = start + 1;
    if index < pattern.len() && matches!(pattern[index], b'!' | b'^') {
        index += 1;
    }
    let mut members = 0;
    while index < pattern.len() {
        if pattern[index] == b'\\' {
            index += 1;
            if index == pattern.len() {
                return Err(PatternError::TrailingEscape)
            }
            members += 1;
            index += 1;
            continue
        }
        if pattern[index] == b']' {
            return if members == 0 {
                Err(PatternError::EmptyClass)
            } else {
                Ok(index)
            }
        }
        members += 1;
        index += 1;
    }
    Err(PatternError::UnterminatedClass)
}

fn match_bytes(pattern: &[u8], candidate: &[u8]) -> bool {
    if pattern.is_empty() {
        return candidate.is_empty()
    }
    match pattern[0] {
        b'\\' => pattern.len() > 1
            && !candidate.is_empty()
            && pattern[1] == candidate[0]
            && match_bytes(&pattern[2..], &candidate[1..]),
        b'*' => {
            match_bytes(&pattern[1..], candidate)
                || (!candidate.is_empty()
                    && candidate[0] != b'/'
                    && match_bytes(pattern, next_character(candidate)))
        }
        b'?' => !candidate.is_empty()
            && candidate[0] != b'/'
            && match_bytes(&pattern[1..], next_character(candidate)),
        b'[' => match_class(pattern, candidate).is_some_and(|(matched, consumed)| {
            matched && match_bytes(&pattern[consumed..], next_character(candidate))
        }),
        byte => {
            !candidate.is_empty()
                && byte == candidate[0]
                && match_bytes(&pattern[1..], &candidate[1..])
        }
    }
}

fn next_character(value: &[u8]) -> &[u8] {
    let width = utf8_width(value[0]).min(value.len());
    &value[width..]
}

fn utf8_width(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first < 0xe0 {
        2
    } else if first < 0xf0 {
        3
    } else {
        4
    }
}

fn match_class(pattern: &[u8], candidate: &[u8]) -> Option<(bool, usize)> {
    if pattern.first() != Some(&b'[') || candidate.is_empty() || candidate[0] == b'/' {
        return None
    }
    let end = class_end(pattern, 0).ok()?;
    let mut index = 1;
    let negated = if matches!(pattern[index], b'!' | b'^') {
        index += 1;
        true
    } else {
        false
    };
    let value = candidate[0];
    let mut matched = false;
    while index < end {
        let left = if pattern[index] == b'\\' {
            index += 1;
            let value = pattern[index];
            index += 1;
            value
        } else {
            let value = pattern[index];
            index += 1;
            value
        };
        if index + 1 < end && pattern[index] == b'-' && pattern[index + 1] != b']' {
            index += 1;
            let right = if pattern[index] == b'\\' {
                index += 1;
                let value = pattern[index];
                index += 1;
                value
            } else {
                let value = pattern[index];
                index += 1;
                value
            };
            if left > right {
                return None
            }
            matched |= (left..=right).contains(&value);
        } else {
            matched |= left == value;
        }
    }
    Some((if negated { !matched } else { matched }, end + 1))
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
    }
}
