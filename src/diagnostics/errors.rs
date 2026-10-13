use std::{borrow::Cow, fmt};

/// Omit authority-like user information, including malformed whitespace and separators.
pub(crate) fn redact_url_credentials(text: &str) -> Cow<'_, str> {
    let Some(last_at) = text.rfind('@') else {
        return Cow::Borrowed(text);
    };
    let mut redacted = String::new();
    let mut copied = 0;
    for (scheme_end, _) in text.match_indices("://") {
        let start = scheme_end + 3;
        let authority = &text[start..];
        let end = authority.find('/').unwrap_or(authority.len());
        // A colon can introduce a password containing raw path separators.
        // Such malformed text has no trustworthy URL boundary; prefer hiding
        // ambiguous text to exposing part of a credential. One final-@ lookup
        // also bounds this conservative path to a single pass over the input.
        let at = if authority[..end].contains(':') {
            (last_at >= start).then_some(last_at)
        } else {
            authority[..end].rfind('@').map(|at| start + at)
        };
        if let Some(at) = at {
            redacted.push_str(&text[copied..start]);
            redacted.push_str("[redacted]");
            copied = at;
            if at == last_at {
                break;
            }
        }
    }
    if copied == 0 {
        Cow::Borrowed(text)
    } else {
        redacted.push_str(&text[copied..]);
        Cow::Owned(redacted)
    }
}

/// Marks an operation whose failure terminates the command with status 2.
/// Context retains the underlying diagnostic without unwinding worker threads.
#[derive(Debug)]
pub(crate) struct Fatal;

impl fmt::Display for Fatal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ERROR")
    }
}

pub(crate) fn status(error: &anyhow::Error) -> u8 {
    if error.is::<Fatal>() { 2 } else { 1 }
}
