pub const DEFAULT_IDENTITY: &str = "- **Role:** Senior Rust Engineer\n- **Tone:** Clinical and concise\n- **Logic:** Chain of Thought\n- **Constraint:** State 'Data unavailable' if unsure.\n- **Formatting:** High-scannability, no engagement loops";

fn format_workspace_seal(model: &str) -> String {
    format!(
        "agent=FCP\nmodel={}\nsealed_at={}",
        model,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

fn model_from_seal_text(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("model=") {
            return Some(v.trim().to_string());
        }
    }
    None
}

pub async fn init_workspace(
    vault_root: &std::path::Path,
    workspace: &str,
    model: &str,
) -> crate::executive::error::Result<()> {
    let workspace_dir = vault_root.join(workspace);
    let seal_file = crate::vault_layout::seal(&workspace_dir);

    if workspace_dir.exists() {
        if !seal_file.exists() {
            return Err(crate::executive::error::FcpError::Config(format!(
                "Workspace '{}' exists but missing {}",
                workspace,
                seal_file.display()
            )));
        }

        let seal_content = tokio::fs::read_to_string(&seal_file).await?;
        let sealed_model = model_from_seal_text(&seal_content).ok_or_else(|| {
            crate::executive::error::FcpError::Config(format!(
                "Workspace '{}' seal missing model= line: {}",
                workspace,
                seal_file.display()
            ))
        })?;

        if sealed_model != model {
            return Err(crate::executive::error::FcpError::Config(format!(
                "Workspace seal model mismatch: got '{}', requested '{}'",
                sealed_model, model
            )));
        }

        return Ok(());
    }

    tokio::fs::create_dir_all(&workspace_dir).await?;

    let core_dir = workspace_dir.join("00_Invariants");
    tokio::fs::create_dir_all(&core_dir).await?;

    for sub in [
        "10_Topology",
        "20_Discourse",
        "30_Synthesis",
    ] {
        tokio::fs::create_dir_all(workspace_dir.join(sub)).await?;
    }

    let identity_file = core_dir.join("Identity.md");
    tokio::fs::write(&identity_file, DEFAULT_IDENTITY).await?;

    tokio::fs::metadata(&identity_file).await.map_err(|e| {
        crate::executive::error::FcpError::WorkspaceFault {
            workspace: workspace.to_string(),
            reason: format!(
                "Identity.md missing after init_workspace write: {}: {}",
                identity_file.display(),
                e
            ),
        }
    })?;

    let tools = crate::vault_layout::tools_dir(&workspace_dir);
    tokio::fs::create_dir_all(&tools).await?;

    if let Some(parent) = seal_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&seal_file, format_workspace_seal(model)).await?;

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

        let core_dir = workspace_dir.join("00_Invariants");
        assert!(core_dir.exists(), "00_Invariants directory should exist");

        assert!(
            workspace_dir.join("30_Synthesis").exists(),
            "30_Synthesis directory should exist"
        );

        let identity_file = core_dir.join("Identity.md");
        assert!(identity_file.exists(), "Identity.md should exist");

        let identity_content = tokio::fs::read_to_string(&identity_file).await.unwrap();
        assert_eq!(identity_content, DEFAULT_IDENTITY);

        let seal_file = crate::vault_layout::seal(&workspace_dir);
        assert!(seal_file.exists(), "seal file should exist");

        let seal_content = tokio::fs::read_to_string(&seal_file).await.unwrap();
        assert_eq!(
            model_from_seal_text(&seal_content).as_deref(),
            Some(model)
        );
    }

    #[tokio::test]
    async fn test_boot_seal_mismatch_fails() {
        let temp_dir = tempdir().unwrap();
        let vault_root = temp_dir.path();
        let workspace = "test_workspace";
        let workspace_dir = vault_root.join(workspace);

        tokio::fs::create_dir_all(&workspace_dir).await.unwrap();

        let seal_file = crate::vault_layout::seal(&workspace_dir);
        if let Some(p) = seal_file.parent() {
            tokio::fs::create_dir_all(p).await.unwrap();
        }
        tokio::fs::write(&seal_file, format_workspace_seal("old_model"))
            .await
            .unwrap();

        let err = init_workspace(vault_root, workspace, "new_model")
            .await
            .unwrap_err();
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

        let err = init_workspace(vault_root, workspace, "test_model")
            .await
            .unwrap_err();
        match err {
            crate::executive::error::FcpError::Config(_) => {}
            _ => panic!("Expected FcpError::Config, got {:?}", err),
        }
    }
}
