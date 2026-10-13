//! Interpretation of supported native credential encodings.

use anyhow::{Result, bail};
use base64::Engine;

pub(crate) fn decode_keychain(value: &str) -> Result<String> {
    let value = value.trim();
    if let Some(encoded) = value.strip_prefix("go-keyring-base64:") {
        return Ok(String::from_utf8(
            base64::engine::general_purpose::STANDARD.decode(encoded)?,
        )?);
    }
    if let Some(encoded) = value.strip_prefix("go-keyring-encoded:") {
        if encoded.len() % 2 != 0 {
            bail!("invalid hexadecimal credential");
        }
        let bytes = encoded
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                let text = std::str::from_utf8(b)?;
                Ok(u8::from_str_radix(text, 16)?)
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(String::from_utf8(bytes)?);
    }
    Ok(value.into())
}
