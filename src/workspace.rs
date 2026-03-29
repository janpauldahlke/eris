use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct FcpSeal {
    pub model: String,
}

pub const DEFAULT_IDENTITY: &str = "- **Role:** Senior Rust Engineer\n- **Tone:** Clinical and concise\n- **Logic:** Chain of Thought\n- **Constraint:** State 'Data unavailable' if unsure.\n- **Formatting:** High-scannability, no engagement loops";

pub async fn init_workspace(
    vault_root: &std::path::Path,
    workspace: &str,
    model: &str,
) -> crate::executive::error::Result<()> {
    let workspace_dir = vault_root.join(workspace);
    let seal_file = workspace_dir.join(".fcp_seal.json");

    if workspace_dir.exists() {
        if !seal_file.exists() {
            return Err(crate::executive::error::FcpError::Config(
                format!("Workspace '{}' exists but missing .fcp_seal.json", workspace)
            ));
        }

        let seal_content = tokio::fs::read_to_string(&seal_file).await?;
        let seal: FcpSeal = serde_json::from_str(&seal_content)?;

        if seal.model != model {
            return Err(crate::executive::error::FcpError::Config(
                format!("Workspace seal model mismatch: got '{}', requested '{}'", seal.model, model)
            ));
        }

        return Ok(());
    }

    tokio::fs::create_dir_all(&workspace_dir).await?;

    let core_dir = workspace_dir.join("00_Core");
    tokio::fs::create_dir_all(&core_dir).await?;

    let identity_file = core_dir.join("Identity.md");
    tokio::fs::write(&identity_file, DEFAULT_IDENTITY).await?;

    let seal = FcpSeal {
        model: model.to_string(),
    };
    tokio::fs::write(&seal_file, serde_json::to_string(&seal)?).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_boot_creates_identity_and_seal() {
        let temp_dir = tempdir().unwrap();
        let vault_root = temp_dir.path();
        
        let workspace = "test_workspace";
        let model = "test_model";
        
        init_workspace(vault_root, workspace, model).await.unwrap();
        
        let workspace_dir = vault_root.join(workspace);
        
        // Assert 00_Core/ is created
        let core_dir = workspace_dir.join("00_Core");
        assert!(core_dir.exists(), "00_Core directory should exist");
        
        // Assert Identity.md exists
        let identity_file = core_dir.join("Identity.md");
        assert!(identity_file.exists(), "Identity.md should exist");
        
        let identity_content = tokio::fs::read_to_string(&identity_file).await.unwrap();
        assert_eq!(identity_content, DEFAULT_IDENTITY);
        
        // Assert .fcp_seal.json contains the requested model
        let seal_file = workspace_dir.join(".fcp_seal.json");
        assert!(seal_file.exists(), ".fcp_seal.json should exist");
        
        let seal_content = tokio::fs::read_to_string(&seal_file).await.unwrap();
        let seal: FcpSeal = serde_json::from_str(&seal_content).unwrap();
        assert_eq!(seal.model, model);
    }

    #[tokio::test]
    async fn test_boot_seal_mismatch_fails() {
        let temp_dir = tempdir().unwrap();
        let vault_root = temp_dir.path();
        let workspace = "test_workspace";
        let workspace_dir = vault_root.join(workspace);
        
        tokio::fs::create_dir_all(&workspace_dir).await.unwrap();
        
        let seal = FcpSeal {
            model: "old_model".to_string(),
        };
        let seal_file = workspace_dir.join(".fcp_seal.json");
        tokio::fs::write(&seal_file, serde_json::to_string(&seal).unwrap()).await.unwrap();
        
        let err = init_workspace(vault_root, workspace, "new_model").await.unwrap_err();
        match err {
            crate::executive::error::FcpError::Config(_) => {}
            _ => panic!("Expected FcpError::Config, got {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_missing_seal_on_existing_workspace_fails() {
        let temp_dir = tempdir().unwrap();
        let vault_root = temp_dir.path();
        let workspace = "test_workspace";
        let workspace_dir = vault_root.join(workspace);
        
        tokio::fs::create_dir_all(&workspace_dir).await.unwrap();
        
        let err = init_workspace(vault_root, workspace, "test_model").await.unwrap_err();
        match err {
            crate::executive::error::FcpError::Config(_) => {}
            _ => panic!("Expected FcpError::Config, got {:?}", err),
        }
    }
}
