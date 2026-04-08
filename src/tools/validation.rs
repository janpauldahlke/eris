use std::path::{Component, Path};
use crate::executive::error::{FcpError, Result};

pub fn validate_path_is_mutable(path_str: &str) -> Result<()> {
    let path = Path::new(path_str);
    for component in path.components() {
        match component {
            Component::ParentDir => return Err(FcpError::ToolFault { tool_name: "gatekeeper".to_string(), reason: "Path Traversal Denied".to_string() }),
            Component::Normal(p) if p == "00_Core" => return Err(FcpError::ToolFault { tool_name: "gatekeeper".to_string(), reason: "00_Core is Immutable".to_string() }),
            Component::Normal(p) if p == "00_Invariants" => return Err(FcpError::ToolFault { tool_name: "gatekeeper".to_string(), reason: "00_Invariants is Immutable — user-maintained only".to_string() }),
            Component::RootDir | Component::Prefix(_) => return Err(FcpError::ToolFault { tool_name: "gatekeeper".to_string(), reason: "Absolute paths outside workspace denied".to_string() }),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_path_allows_drops() {
        let res = validate_path_is_mutable("90_Drops/new_note.md");
        assert!(res.is_ok());
    }

    #[test]
    fn test_validate_path_rejects_core_root() {
        let res = validate_path_is_mutable("00_Core/Identity.md");
        assert!(res.is_err());
    }

    #[test]
    fn test_validate_path_rejects_core_traversal() {
        let res = validate_path_is_mutable("10_Projects/../00_Core/Identity.md");
        assert!(res.is_err());
    }

    #[test]
    fn test_validate_path_rejects_invariants() {
        let res = validate_path_is_mutable("00_Invariants/Identity.md");
        assert!(res.is_err());
        let err = format!("{}", res.unwrap_err());
        assert!(err.contains("Immutable"));
    }

    #[test]
    fn test_validate_path_rejects_parent_dir() {
        let res = validate_path_is_mutable("../outside.md");
        assert!(res.is_err());
    }

    #[test]
    fn test_validate_path_rejects_absolute() {
        let res = validate_path_is_mutable("/etc/passwd");
        assert!(res.is_err());
    }
}
