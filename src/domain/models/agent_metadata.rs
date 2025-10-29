///! Agent Metadata Parser
///!
///! Parses agent markdown files from .claude/agents directory to extract
///! metadata like model configuration, tools, MCP servers, etc.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Agent metadata extracted from markdown frontmatter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    /// Agent name/identifier
    pub name: String,

    /// Description of the agent's purpose
    pub description: Option<String>,

    /// Model to use (opus, sonnet, haiku, or full model ID)
    pub model: String,

    /// Display color for the agent
    pub color: Option<String>,

    /// List of tools the agent can use
    pub tools: Vec<String>,

    /// List of MCP servers the agent can access
    pub mcp_servers: Vec<String>,
}

impl AgentMetadata {
    /// Parse agent metadata from a markdown file
    ///
    /// Extracts YAML frontmatter from between `---` delimiters at the start of the file.
    ///
    /// # Arguments
    /// * `path` - Path to the agent markdown file
    ///
    /// # Returns
    /// * `Ok(AgentMetadata)` - Successfully parsed metadata
    /// * `Err` - Failed to read or parse the file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read agent file: {}", path.as_ref().display()))?;

        Self::from_markdown(&content)
    }

    /// Parse agent metadata from markdown content
    ///
    /// Extracts YAML frontmatter from between `---` delimiters.
    ///
    /// # Arguments
    /// * `content` - Markdown file content
    ///
    /// # Returns
    /// * `Ok(AgentMetadata)` - Successfully parsed metadata
    /// * `Err` - Failed to parse frontmatter
    pub fn from_markdown(content: &str) -> Result<Self> {
        // Extract frontmatter between --- delimiters
        let frontmatter = Self::extract_frontmatter(content)
            .context("Failed to extract frontmatter from markdown")?;

        // Parse YAML frontmatter
        let metadata: AgentMetadata = serde_yaml::from_str(&frontmatter)
            .context("Failed to parse YAML frontmatter")?;

        Ok(metadata)
    }

    /// Extract frontmatter from markdown content
    ///
    /// Looks for content between `---` delimiters at the start of the file.
    ///
    /// # Arguments
    /// * `content` - Markdown file content
    ///
    /// # Returns
    /// * `Ok(String)` - Extracted frontmatter
    /// * `Err` - No frontmatter found
    fn extract_frontmatter(content: &str) -> Result<String> {
        let lines: Vec<&str> = content.lines().collect();

        // Check if file starts with ---
        if lines.is_empty() || lines[0] != "---" {
            anyhow::bail!("Markdown file does not start with frontmatter delimiter (---)");
        }

        // Find the closing ---
        let end_index = lines.iter().skip(1).position(|&line| line == "---")
            .context("No closing frontmatter delimiter (---) found")?;

        // Extract frontmatter lines (excluding the --- delimiters)
        let frontmatter_lines = &lines[1..=end_index];
        Ok(frontmatter_lines.join("\n"))
    }

    /// Get the full Claude model ID from the model field
    ///
    /// Maps short names (opus, sonnet, haiku) to full model IDs.
    /// Returns the value as-is if it's already a full model ID.
    ///
    /// # Returns
    /// The full Claude model ID (e.g., "claude-opus-4-1-20250514")
    pub fn get_model_id(&self) -> String {
        match self.model.as_str() {
            "opus" => "claude-opus-4-1-20250514".to_string(),
            "sonnet" => "claude-sonnet-4-5-20250929".to_string(),
            "haiku" => "claude-haiku-4-5-20250929".to_string(),
            // If it's already a full model ID or unknown, return as-is
            _ => self.model.clone(),
        }
    }
}

/// Agent metadata registry
///
/// Loads and caches agent metadata from .claude/agents directory
pub struct AgentMetadataRegistry {
    /// Cached metadata by agent type
    metadata: HashMap<String, AgentMetadata>,

    /// Path to .claude/agents directory
    agents_dir: PathBuf,
}

impl AgentMetadataRegistry {
    /// Create a new registry
    ///
    /// # Arguments
    /// * `agents_dir` - Path to the .claude/agents directory
    ///
    /// # Returns
    /// A new `AgentMetadataRegistry`
    pub fn new<P: AsRef<Path>>(agents_dir: P) -> Self {
        Self {
            metadata: HashMap::new(),
            agents_dir: agents_dir.as_ref().to_path_buf(),
        }
    }

    /// Load all agent metadata from the directory
    ///
    /// Scans the .claude/agents directory recursively for .md files.
    ///
    /// # Returns
    /// * `Ok(())` - Successfully loaded all agent files
    /// * `Err` - Failed to read directory or parse some files
    pub fn load_all(&mut self) -> Result<()> {
        self.scan_directory(&self.agents_dir.clone())
    }

    /// Recursively scan a directory for agent markdown files
    fn scan_directory(&mut self, dir: &Path) -> Result<()> {
        for entry in fs::read_dir(dir)
            .with_context(|| format!("Failed to read directory: {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Recursively scan subdirectories
                self.scan_directory(&path)?;
            } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
                // Parse agent markdown file
                match AgentMetadata::from_file(&path) {
                    Ok(metadata) => {
                        self.metadata.insert(metadata.name.clone(), metadata);
                    }
                    Err(e) => {
                        // Log warning but continue with other files
                        tracing::warn!("Failed to parse agent file {}: {}", path.display(), e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Get metadata for a specific agent type
    ///
    /// If the metadata hasn't been loaded yet, attempts to load it on-demand.
    ///
    /// # Arguments
    /// * `agent_type` - The agent type (e.g., "requirements-gatherer")
    ///
    /// # Returns
    /// * `Some(AgentMetadata)` - Metadata found for this agent
    /// * `None` - No metadata file exists for this agent type
    pub fn get(&mut self, agent_type: &str) -> Option<&AgentMetadata> {
        // If already cached, return it
        if self.metadata.contains_key(agent_type) {
            return self.metadata.get(agent_type);
        }

        // Try to load on-demand
        if let Ok(()) = self.try_load_agent(agent_type) {
            return self.metadata.get(agent_type);
        }

        None
    }

    /// Try to load a specific agent file on-demand
    ///
    /// Searches for the agent file in known subdirectories.
    fn try_load_agent(&mut self, agent_type: &str) -> Result<()> {
        // Try common subdirectories
        let subdirs = ["abathur", "workers", ""];

        for subdir in &subdirs {
            let path = if subdir.is_empty() {
                self.agents_dir.join(format!("{}.md", agent_type))
            } else {
                self.agents_dir.join(subdir).join(format!("{}.md", agent_type))
            };

            if path.exists() {
                let metadata = AgentMetadata::from_file(&path)?;
                self.metadata.insert(metadata.name.clone(), metadata);
                return Ok(());
            }
        }

        anyhow::bail!("Agent file not found for: {}", agent_type)
    }

    /// Get model ID for an agent type
    ///
    /// Returns the full Claude model ID for the agent, or a default if not found.
    ///
    /// # Arguments
    /// * `agent_type` - The agent type
    ///
    /// # Returns
    /// The full Claude model ID
    pub fn get_model_id(&mut self, agent_type: &str) -> String {
        self.get(agent_type)
            .map(|meta| meta.get_model_id())
            .unwrap_or_else(|| {
                tracing::warn!(
                    "No metadata found for agent type '{}', defaulting to sonnet",
                    agent_type
                );
                "claude-sonnet-4-5-20250929".to_string()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_frontmatter() {
        let markdown = r#"---
name: test-agent
model: opus
tools: [Read, Write]
mcp_servers: []
---

# Agent Instructions

This is the agent body.
"#;

        let frontmatter = AgentMetadata::extract_frontmatter(markdown).unwrap();
        assert!(frontmatter.contains("name: test-agent"));
        assert!(frontmatter.contains("model: opus"));
    }

    #[test]
    fn test_parse_metadata() {
        let markdown = r#"---
name: test-agent
description: "A test agent"
model: opus
color: Blue
tools:
  - Read
  - Write
mcp_servers:
  - abathur-memory
---

# Agent body
"#;

        let metadata = AgentMetadata::from_markdown(markdown).unwrap();
        assert_eq!(metadata.name, "test-agent");
        assert_eq!(metadata.model, "opus");
        assert_eq!(metadata.tools, vec!["Read", "Write"]);
        assert_eq!(metadata.mcp_servers, vec!["abathur-memory"]);
    }

    #[test]
    fn test_model_id_mapping() {
        let mut meta = AgentMetadata {
            name: "test".to_string(),
            description: None,
            model: "opus".to_string(),
            color: None,
            tools: vec![],
            mcp_servers: vec![],
        };

        assert_eq!(meta.get_model_id(), "claude-opus-4-1-20250514");

        meta.model = "sonnet".to_string();
        assert_eq!(meta.get_model_id(), "claude-sonnet-4-5-20250929");

        meta.model = "haiku".to_string();
        assert_eq!(meta.get_model_id(), "claude-haiku-4-5-20250929");

        // Full model ID should pass through
        meta.model = "claude-custom-model-123".to_string();
        assert_eq!(meta.get_model_id(), "claude-custom-model-123");
    }
}
