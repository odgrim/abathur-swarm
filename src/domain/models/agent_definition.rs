//! Agent definition parser for `.claude/agents/*.md` files.
//!
//! Agent definitions use YAML frontmatter + markdown body format:
//! ```markdown
//! ---
//! name: my-agent
//! tier: worker
//! description: Does things
//! tools:
//!   - read
//!   - write
//! constraints:
//!   - Always run tests
//! max_turns: 30
//! ---
//!
//! # System Prompt
//!
//! You are an agent that does things...
//! ```

use std::path::Path;

use crate::domain::models::agent::{
    AgentConstraint, AgentTemplate, AgentTier, ToolCapability,
};

/// Parsed representation of a `.claude/agents/*.md` file.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub name: String,
    pub tier: String,
    pub version: Option<String>,
    pub description: String,
    pub tools: Vec<String>,
    pub constraints: Vec<String>,
    pub max_turns: u32,
    /// The markdown body after the closing `---`.
    pub system_prompt: String,
}

impl AgentDefinition {
    /// Parse a `.claude/agents/*.md` file content into an `AgentDefinition`.
    ///
    /// Expected format: YAML frontmatter between `---` markers, followed by
    /// the markdown body which becomes the system prompt.
    pub fn parse(content: &str) -> Result<AgentDefinition, String> {
        let trimmed = content.trim();

        // Must start with ---
        if !trimmed.starts_with("---") {
            return Err("Agent definition must start with YAML frontmatter (---)".to_string());
        }

        // Find the closing ---
        let after_first = &trimmed[3..];
        let closing_idx = after_first.find("\n---")
            .ok_or_else(|| "Missing closing --- for YAML frontmatter".to_string())?;

        let yaml_str = &after_first[..closing_idx].trim();
        let body_start = closing_idx + 4; // skip "\n---"
        let system_prompt = after_first[body_start..].trim().to_string();

        // Parse YAML frontmatter
        let yaml_value: serde_yaml::Value = serde_yaml::from_str(yaml_str)
            .map_err(|e| format!("Failed to parse YAML frontmatter: {}", e))?;

        let mapping = yaml_value.as_mapping()
            .ok_or_else(|| "YAML frontmatter must be a mapping".to_string())?;

        let name = mapping.get(serde_yaml::Value::String("name".to_string()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required field: name".to_string())?
            .to_lowercase();

        let tier = mapping.get(serde_yaml::Value::String("tier".to_string()))
            .and_then(|v| v.as_str())
            .unwrap_or("worker")
            .to_string();

        let version = mapping.get(serde_yaml::Value::String("version".to_string()))
            .and_then(|v| {
                v.as_str().map(|s| s.to_string())
                    .or_else(|| v.as_u64().map(|n| n.to_string()))
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
                    .or_else(|| v.as_f64().map(|n| n.to_string()))
            });

        let description = mapping.get(serde_yaml::Value::String("description".to_string()))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let tools = mapping.get(serde_yaml::Value::String("tools".to_string()))
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let constraints = mapping.get(serde_yaml::Value::String("constraints".to_string()))
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let max_turns = mapping.get(serde_yaml::Value::String("max_turns".to_string()))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(25);

        if system_prompt.is_empty() {
            return Err("Agent definition must have a markdown body (system prompt)".to_string());
        }

        Ok(AgentDefinition {
            name,
            tier,
            version,
            description,
            tools,
            constraints,
            max_turns,
            system_prompt,
        })
    }

    /// Convert this definition to an `AgentTemplate` for storage in the DB.
    pub fn to_agent_template(&self) -> AgentTemplate {
        let tier = self.parse_tier();

        let mut template = AgentTemplate::new(&self.name, tier)
            .with_description(&self.description)
            .with_prompt(&self.system_prompt)
            .with_max_turns(self.max_turns);

        for tool_name in &self.tools {
            let tool = ToolCapability::new(tool_name, format!("{} tool", tool_name));
            template = template.with_tool(tool);
        }

        for constraint_desc in &self.constraints {
            // Generate a kebab-case name from the constraint description
            let constraint_name: String = constraint_desc
                .to_lowercase()
                .split_whitespace()
                .take(4)
                .collect::<Vec<_>>()
                .join("-");
            let constraint = AgentConstraint::new(&constraint_name, constraint_desc);
            template = template.with_constraint(constraint);
        }

        template
    }

    /// Create an `AgentDefinition` from an `AgentTemplate`.
    pub fn from_template(template: &AgentTemplate) -> AgentDefinition {
        AgentDefinition {
            name: template.name.clone(),
            tier: template.tier.as_str().to_string(),
            version: Some(format!("{}", template.version)),
            description: template.description.clone(),
            tools: template.tools.iter().map(|t| t.name.clone()).collect(),
            constraints: template.constraints.iter().map(|c| c.description.clone()).collect(),
            max_turns: template.max_turns,
            system_prompt: template.system_prompt.clone(),
        }
    }

    /// Serialize back to `.md` format (YAML frontmatter + markdown body).
    pub fn to_markdown(&self) -> String {
        let mut yaml_parts = vec![
            format!("name: {}", self.name),
            format!("tier: {}", self.tier),
        ];

        if let Some(ref version) = self.version {
            yaml_parts.push(format!("version: {}", version));
        }

        yaml_parts.push(format!("description: {}", self.description));

        if !self.tools.is_empty() {
            yaml_parts.push("tools:".to_string());
            for tool in &self.tools {
                yaml_parts.push(format!("  - {}", tool));
            }
        }

        if !self.constraints.is_empty() {
            yaml_parts.push("constraints:".to_string());
            for constraint in &self.constraints {
                yaml_parts.push(format!("  - {}", constraint));
            }
        }

        yaml_parts.push(format!("max_turns: {}", self.max_turns));

        format!("---\n{}\n---\n\n{}\n", yaml_parts.join("\n"), self.system_prompt)
    }

    /// Scan a directory for `.md` agent definitions and parse them.
    pub fn load_from_directory(dir: &Path) -> Result<Vec<AgentDefinition>, String> {
        if !dir.exists() {
            return Ok(vec![]);
        }

        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("Failed to read directory {:?}: {}", dir, e))?;

        let mut definitions = Vec::new();

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
            let path = entry.path();

            if path.extension().is_some_and(|ext| ext == "md") {
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read {:?}: {}", path, e))?;

                match AgentDefinition::parse(&content) {
                    Ok(def) => {
                        tracing::debug!("Loaded agent definition '{}' from {:?}", def.name, path);
                        definitions.push(def);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse agent definition {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok(definitions)
    }

    /// Parse the tier string into an `AgentTier`.
    fn parse_tier(&self) -> AgentTier {
        match self.tier.to_lowercase().as_str() {
            "architect" | "meta" => AgentTier::Architect,
            "specialist" => AgentTier::Specialist,
            _ => AgentTier::Worker,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::specialist_templates::create_overmind;

    #[test]
    fn test_parse_basic() {
        let content = r#"---
name: test-agent
tier: worker
description: A test agent
tools:
  - read
  - write
constraints:
  - Always run tests
max_turns: 30
---

You are a test agent. Do test things."#;

        let def = AgentDefinition::parse(content).unwrap();
        assert_eq!(def.name, "test-agent");
        assert_eq!(def.tier, "worker");
        assert_eq!(def.description, "A test agent");
        assert_eq!(def.tools, vec!["read", "write"]);
        assert_eq!(def.constraints, vec!["Always run tests"]);
        assert_eq!(def.max_turns, 30);
        assert_eq!(def.system_prompt, "You are a test agent. Do test things.");
    }

    #[test]
    fn test_parse_overmind_style() {
        let content = r#"---
name: Overmind
tier: meta
version: 1.0.0
description: Orchestrates the swarm
tools:
  - read
  - write
  - edit
  - shell
max_turns: 50
---

# Overmind

You are the Overmind."#;

        let def = AgentDefinition::parse(content).unwrap();
        assert_eq!(def.name, "overmind");
        assert_eq!(def.tier, "meta");
        assert_eq!(def.version, Some("1.0.0".to_string()));
        assert!(def.system_prompt.contains("# Overmind"));
    }

    #[test]
    fn test_parse_missing_frontmatter() {
        let content = "Just some markdown without frontmatter.";
        assert!(AgentDefinition::parse(content).is_err());
    }

    #[test]
    fn test_parse_missing_body() {
        let content = "---\nname: test\ntier: worker\n---\n";
        assert!(AgentDefinition::parse(content).is_err());
    }

    #[test]
    fn test_to_markdown_roundtrip() {
        let original = r#"---
name: test-agent
tier: worker
description: A test agent
tools:
  - read
  - write
constraints:
  - Always run tests
max_turns: 30
---

You are a test agent."#;

        let def = AgentDefinition::parse(original).unwrap();
        let markdown = def.to_markdown();
        let reparsed = AgentDefinition::parse(&markdown).unwrap();

        assert_eq!(def.name, reparsed.name);
        assert_eq!(def.tier, reparsed.tier);
        assert_eq!(def.description, reparsed.description);
        assert_eq!(def.tools, reparsed.tools);
        assert_eq!(def.constraints, reparsed.constraints);
        assert_eq!(def.max_turns, reparsed.max_turns);
        assert_eq!(def.system_prompt, reparsed.system_prompt);
    }

    #[test]
    fn test_to_agent_template() {
        let def = AgentDefinition {
            name: "test-agent".to_string(),
            tier: "worker".to_string(),
            version: None,
            description: "A test agent".to_string(),
            tools: vec!["read".to_string(), "write".to_string()],
            constraints: vec!["Always run tests".to_string()],
            max_turns: 30,
            system_prompt: "You are a test agent.".to_string(),
        };

        let template = def.to_agent_template();
        assert_eq!(template.name, "test-agent");
        assert_eq!(template.tier, AgentTier::Worker);
        assert_eq!(template.max_turns, 30);
        assert!(template.has_tool("read"));
        assert!(template.has_tool("write"));
        assert_eq!(template.system_prompt, "You are a test agent.");
    }

    #[test]
    fn test_from_template_to_markdown() {
        let template = create_overmind();
        let def = AgentDefinition::from_template(&template);
        let markdown = def.to_markdown();

        // Should be parseable
        let reparsed = AgentDefinition::parse(&markdown).unwrap();
        assert_eq!(reparsed.name, "overmind");
        assert_eq!(reparsed.tier, "architect");
        assert!(!reparsed.system_prompt.is_empty());
    }

    #[test]
    fn test_tier_parsing() {
        let def = AgentDefinition {
            name: "test".to_string(),
            tier: "meta".to_string(),
            version: None,
            description: String::new(),
            tools: vec![],
            constraints: vec![],
            max_turns: 25,
            system_prompt: "test".to_string(),
        };
        assert_eq!(def.parse_tier(), AgentTier::Architect);

        let def2 = AgentDefinition { tier: "specialist".to_string(), ..def.clone() };
        assert_eq!(def2.parse_tier(), AgentTier::Specialist);

        let def3 = AgentDefinition { tier: "worker".to_string(), ..def };
        assert_eq!(def3.parse_tier(), AgentTier::Worker);
    }
}
