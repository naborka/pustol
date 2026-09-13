//! Staff-written text: length limits, non-blank names and reasons.

/// Counts characters, not bytes.
#[must_use]
pub fn longer_than(text: &str, limit: usize) -> bool {
    text.chars().nth(limit).is_some()
}

/// Trimmed, never blank.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GuestName(String);

#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
#[error("a booking needs a name to call out")]
pub struct BlankGuestName;

impl GuestName {
    /// # Errors
    ///
    /// [`BlankGuestName`] when `text` is blank after trim.
    pub fn new(text: &str) -> Result<Self, BlankGuestName> {
        trimmed(text).map(Self).ok_or(BlankGuestName)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why table shut for shift. Trimmed, never blank: storage holds no closure staff cannot explain.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BlockReason(String);

#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
#[error("closing a table needs a reason staff can read later")]
pub struct MissingBlockReason;

impl BlockReason {
    /// # Errors
    ///
    /// [`MissingBlockReason`] when `text` is blank after trim.
    pub fn new(text: &str) -> Result<Self, MissingBlockReason> {
        trimmed(text).map(Self).ok_or(MissingBlockReason)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn trimmed(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_is_longer_than_a_limit_only_past_its_last_character() {
        assert!(!longer_than("", 0));
        assert!(longer_than("я", 0));
        assert!(!longer_than("яяя", 3));
        assert!(longer_than("яяяя", 3));
    }

    #[test]
    fn a_guest_name_and_a_block_reason_are_trimmed_and_never_blank() {
        assert_eq!(
            GuestName::new("  Полина ").map(|name| name.as_str().to_owned()),
            Ok("Полина".to_owned())
        );
        assert_eq!(GuestName::new(" \u{3000}\t"), Err(BlankGuestName));
        assert_eq!(
            BlockReason::new(" Дождь ").map(|reason| reason.as_str().to_owned()),
            Ok("Дождь".to_owned())
        );
        assert_eq!(BlockReason::new("   "), Err(MissingBlockReason));
    }
}
