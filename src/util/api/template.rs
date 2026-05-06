use std::collections::HashMap;

use crate::executive::error::{FcpError, Result};

/// Replace `{key}` tokens using `params`. Fails on missing keys or malformed braces.
pub fn apply_template(input: &str, params: &HashMap<String, String>) -> Result<String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        rest = rest
            .get(start + 1..)
            .ok_or_else(|| FcpError::Config("invalid template: truncated after '{'".into()))?;
        let end = rest
            .find('}')
            .ok_or_else(|| FcpError::Config("unclosed template placeholder".into()))?;
        let key = rest[..end].trim();
        if key.is_empty() {
            return Err(FcpError::Config("empty template placeholder".into()));
        }
        let value = params
            .get(key)
            .ok_or_else(|| FcpError::Config(format!("missing template parameter: {key}")))?;
        out.push_str(value);
        rest = rest.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_template_replaces_placeholders() {
        let mut p = HashMap::new();
        p.insert("city".into(), "London".into());
        assert_eq!(
            apply_template("q={city}&x=1", &p).expect("ok"),
            "q=London&x=1"
        );
    }

    #[test]
    fn apply_template_missing_key_errors() {
        let p = HashMap::new();
        let r = apply_template("{missing}", &p);
        assert!(matches!(r, Err(FcpError::Config(_))));
    }
}
