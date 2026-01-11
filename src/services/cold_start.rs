//! Cold start context gathering service.
//!
//! Handles initial project analysis when the swarm starts with empty memory,
//! populating semantic memories with codebase structure, conventions, and dependencies.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::domain::errors::{DomainError, DomainResult};
use crate::services::MemoryService;

/// Configuration for cold start context gathering.
#[derive(Debug, Clone)]
pub struct ColdStartConfig {
    /// Root directory of the project.
    pub project_root: PathBuf,
    /// Maximum depth for directory scanning.
    pub max_scan_depth: usize,
    /// File extensions to analyze.
    pub analyzed_extensions: Vec<String>,
    /// Directories to ignore.
    pub ignore_dirs: Vec<String>,
    /// Whether to analyze dependencies.
    pub analyze_dependencies: bool,
    /// Whether to detect conventions.
    pub detect_conventions: bool,
}

impl Default for ColdStartConfig {
    fn default() -> Self {
        Self {
            project_root: PathBuf::from("."),
            max_scan_depth: 5,
            analyzed_extensions: vec![
                "rs".to_string(),
                "ts".to_string(),
                "tsx".to_string(),
                "js".to_string(),
                "jsx".to_string(),
                "py".to_string(),
                "go".to_string(),
                "java".to_string(),
                "md".to_string(),
                "toml".to_string(),
                "json".to_string(),
                "yaml".to_string(),
                "yml".to_string(),
            ],
            ignore_dirs: vec![
                ".git".to_string(),
                "node_modules".to_string(),
                "target".to_string(),
                "dist".to_string(),
                "build".to_string(),
                ".abathur".to_string(),
                "__pycache__".to_string(),
                "venv".to_string(),
                ".venv".to_string(),
            ],
            analyze_dependencies: true,
            detect_conventions: true,
        }
    }
}

/// Results from cold start context gathering.
#[derive(Debug, Clone)]
pub struct ColdStartReport {
    /// Detected project type.
    pub project_type: ProjectType,
    /// Codebase structure summary.
    pub structure_summary: String,
    /// Detected conventions.
    pub conventions: Vec<Convention>,
    /// Dependencies found.
    pub dependencies: Vec<Dependency>,
    /// Files analyzed.
    pub files_analyzed: usize,
    /// Directories scanned.
    pub directories_scanned: usize,
    /// Memories created.
    pub memories_created: usize,
}

/// Detected project type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectType {
    Rust,
    Node,
    Python,
    Go,
    Java,
    Mixed,
    Unknown,
}

impl std::fmt::Display for ProjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectType::Rust => write!(f, "Rust"),
            ProjectType::Node => write!(f, "Node.js"),
            ProjectType::Python => write!(f, "Python"),
            ProjectType::Go => write!(f, "Go"),
            ProjectType::Java => write!(f, "Java"),
            ProjectType::Mixed => write!(f, "Multi-language"),
            ProjectType::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Detected convention.
#[derive(Debug, Clone)]
pub struct Convention {
    /// Convention name.
    pub name: String,
    /// Convention description.
    pub description: String,
    /// Confidence level (0.0-1.0).
    pub confidence: f64,
    /// Category of convention.
    pub category: ConventionCategory,
}

/// Category of convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConventionCategory {
    FileStructure,
    Naming,
    Testing,
    Documentation,
    Build,
    Other,
}

/// Detected dependency.
#[derive(Debug, Clone)]
pub struct Dependency {
    /// Dependency name.
    pub name: String,
    /// Version (if available).
    pub version: Option<String>,
    /// Source file.
    pub source: String,
    /// Whether it's a dev dependency.
    pub is_dev: bool,
}

/// Directory entry for structure analysis.
#[derive(Debug, Clone)]
struct DirEntry {
    name: String,
    is_dir: bool,
    depth: usize,
    extension: Option<String>,
}

/// Cold start context gathering service.
pub struct ColdStartService<M>
where
    M: crate::domain::ports::MemoryRepository + 'static,
{
    memory_service: MemoryService<M>,
    config: ColdStartConfig,
}

impl<M> ColdStartService<M>
where
    M: crate::domain::ports::MemoryRepository + 'static,
{
    pub fn new(memory_service: MemoryService<M>, config: ColdStartConfig) -> Self {
        Self {
            memory_service,
            config,
        }
    }

    /// Check if cold start is needed (memory is empty).
    pub async fn needs_cold_start(&self) -> DomainResult<bool> {
        let stats = self.memory_service.get_stats().await?;
        let total = stats.working_count + stats.episodic_count + stats.semantic_count;
        Ok(total == 0)
    }

    /// Run cold start context gathering.
    pub async fn gather_context(&self) -> DomainResult<ColdStartReport> {
        let mut report = ColdStartReport {
            project_type: ProjectType::Unknown,
            structure_summary: String::new(),
            conventions: Vec::new(),
            dependencies: Vec::new(),
            files_analyzed: 0,
            directories_scanned: 0,
            memories_created: 0,
        };

        // Scan directory structure
        let entries = self.scan_directory(&self.config.project_root, 0).await?;
        report.directories_scanned = entries.iter().filter(|e| e.is_dir).count();
        report.files_analyzed = entries.iter().filter(|e| !e.is_dir).count();

        // Detect project type
        report.project_type = self.detect_project_type(&entries).await?;

        // Generate structure summary
        report.structure_summary = self.generate_structure_summary(&entries, &report.project_type);

        // Detect conventions
        if self.config.detect_conventions {
            report.conventions = self.detect_conventions(&entries).await?;
        }

        // Analyze dependencies
        if self.config.analyze_dependencies {
            report.dependencies = self.analyze_dependencies(&report.project_type).await?;
        }

        // Store memories
        report.memories_created = self.store_memories(&report).await?;

        Ok(report)
    }

    /// Scan directory structure recursively.
    async fn scan_directory(&self, path: &Path, depth: usize) -> DomainResult<Vec<DirEntry>> {
        if depth > self.config.max_scan_depth {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        let mut dir_entries = fs::read_dir(path).await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to read directory: {}", e)))?;

        while let Some(entry) = dir_entries.next_entry().await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to read entry: {}", e)))? {
            let name = entry.file_name().to_string_lossy().to_string();

            // Skip ignored directories
            if self.config.ignore_dirs.contains(&name) {
                continue;
            }

            // Skip hidden files (except .claude)
            if name.starts_with('.') && name != ".claude" {
                continue;
            }

            let file_type = entry.file_type().await
                .map_err(|e| DomainError::ValidationFailed(format!("Failed to get file type: {}", e)))?;

            let is_dir = file_type.is_dir();
            let extension = if !is_dir {
                Path::new(&name).extension().map(|e| e.to_string_lossy().to_string())
            } else {
                None
            };

            entries.push(DirEntry {
                name: name.clone(),
                is_dir,
                depth,
                extension,
            });

            // Recurse into subdirectories
            if is_dir {
                let sub_entries = Box::pin(self.scan_directory(&entry.path(), depth + 1)).await?;
                entries.extend(sub_entries);
            }
        }

        Ok(entries)
    }

    /// Detect project type from directory entries.
    async fn detect_project_type(&self, entries: &[DirEntry]) -> DomainResult<ProjectType> {
        let file_names: Vec<&str> = entries.iter()
            .filter(|e| !e.is_dir && e.depth == 0)
            .map(|e| e.name.as_str())
            .collect();

        let mut types = Vec::new();

        if file_names.iter().any(|f| *f == "Cargo.toml") {
            types.push(ProjectType::Rust);
        }
        if file_names.iter().any(|f| *f == "package.json") {
            types.push(ProjectType::Node);
        }
        if file_names.iter().any(|f| *f == "pyproject.toml" || *f == "setup.py" || *f == "requirements.txt") {
            types.push(ProjectType::Python);
        }
        if file_names.iter().any(|f| *f == "go.mod") {
            types.push(ProjectType::Go);
        }
        if file_names.iter().any(|f| *f == "pom.xml" || *f == "build.gradle") {
            types.push(ProjectType::Java);
        }

        Ok(match types.len() {
            0 => ProjectType::Unknown,
            1 => types[0].clone(),
            _ => ProjectType::Mixed,
        })
    }

    /// Generate a structure summary from directory entries.
    fn generate_structure_summary(&self, entries: &[DirEntry], project_type: &ProjectType) -> String {
        let mut summary = format!("Project Type: {}\n\n", project_type);

        // Get top-level directories
        let top_dirs: Vec<&str> = entries.iter()
            .filter(|e| e.is_dir && e.depth == 0)
            .map(|e| e.name.as_str())
            .collect();

        summary.push_str("Top-level directories:\n");
        for dir in &top_dirs {
            summary.push_str(&format!("  - {}/\n", dir));
        }

        // Count files by extension
        let mut ext_counts: HashMap<&str, usize> = HashMap::new();
        for entry in entries.iter().filter(|e| !e.is_dir) {
            if let Some(ref ext) = entry.extension {
                *ext_counts.entry(ext.as_str()).or_insert(0) += 1;
            }
        }

        summary.push_str("\nFile types:\n");
        let mut sorted_exts: Vec<_> = ext_counts.iter().collect();
        sorted_exts.sort_by(|a, b| b.1.cmp(a.1));
        for (ext, count) in sorted_exts.iter().take(10) {
            summary.push_str(&format!("  - .{}: {} files\n", ext, count));
        }

        // Project-specific structure notes
        if top_dirs.contains(&"src") {
            summary.push_str("\nSource code in src/ directory\n");
        }
        if top_dirs.contains(&"tests") || top_dirs.contains(&"test") {
            summary.push_str("Test files in dedicated test directory\n");
        }
        if top_dirs.contains(&"docs") || top_dirs.contains(&"doc") {
            summary.push_str("Documentation in dedicated docs directory\n");
        }

        summary
    }

    /// Detect coding conventions from directory entries.
    async fn detect_conventions(&self, entries: &[DirEntry]) -> DomainResult<Vec<Convention>> {
        let mut conventions = Vec::new();

        // Check for src/ directory
        if entries.iter().any(|e| e.is_dir && e.name == "src") {
            conventions.push(Convention {
                name: "source-dir".to_string(),
                description: "Source code organized in src/ directory".to_string(),
                confidence: 1.0,
                category: ConventionCategory::FileStructure,
            });
        }

        // Check for tests directory
        if entries.iter().any(|e| e.is_dir && (e.name == "tests" || e.name == "test")) {
            conventions.push(Convention {
                name: "separate-tests".to_string(),
                description: "Tests organized in separate directory".to_string(),
                confidence: 1.0,
                category: ConventionCategory::Testing,
            });
        }

        // Check for README
        if entries.iter().any(|e| !e.is_dir && e.name.to_lowercase().starts_with("readme")) {
            conventions.push(Convention {
                name: "readme".to_string(),
                description: "Project has README documentation".to_string(),
                confidence: 1.0,
                category: ConventionCategory::Documentation,
            });
        }

        // Check for CI configuration
        if entries.iter().any(|e| e.is_dir && e.name == ".github") {
            conventions.push(Convention {
                name: "github-actions".to_string(),
                description: "Uses GitHub Actions for CI/CD".to_string(),
                confidence: 0.9,
                category: ConventionCategory::Build,
            });
        }

        // Check for Rust-specific conventions
        let rust_files: Vec<_> = entries.iter()
            .filter(|e| e.extension.as_deref() == Some("rs"))
            .collect();

        if !rust_files.is_empty() {
            // Check for mod.rs usage
            if rust_files.iter().any(|e| e.name == "mod.rs") {
                conventions.push(Convention {
                    name: "mod-rs-pattern".to_string(),
                    description: "Uses mod.rs for module organization".to_string(),
                    confidence: 1.0,
                    category: ConventionCategory::FileStructure,
                });
            }

            // Check for lib.rs
            if rust_files.iter().any(|e| e.name == "lib.rs") {
                conventions.push(Convention {
                    name: "rust-library".to_string(),
                    description: "Rust library crate structure".to_string(),
                    confidence: 1.0,
                    category: ConventionCategory::FileStructure,
                });
            }
        }

        // Check for TypeScript
        if entries.iter().any(|e| !e.is_dir && e.name == "tsconfig.json") {
            conventions.push(Convention {
                name: "typescript".to_string(),
                description: "Uses TypeScript".to_string(),
                confidence: 1.0,
                category: ConventionCategory::Build,
            });
        }

        Ok(conventions)
    }

    /// Analyze project dependencies.
    async fn analyze_dependencies(&self, project_type: &ProjectType) -> DomainResult<Vec<Dependency>> {
        let mut dependencies = Vec::new();

        match project_type {
            ProjectType::Rust => {
                dependencies.extend(self.parse_cargo_toml().await?);
            }
            ProjectType::Node => {
                dependencies.extend(self.parse_package_json().await?);
            }
            ProjectType::Python => {
                dependencies.extend(self.parse_python_deps().await?);
            }
            ProjectType::Mixed => {
                // Try all parsers
                dependencies.extend(self.parse_cargo_toml().await.unwrap_or_default());
                dependencies.extend(self.parse_package_json().await.unwrap_or_default());
                dependencies.extend(self.parse_python_deps().await.unwrap_or_default());
            }
            _ => {}
        }

        Ok(dependencies)
    }

    /// Parse Cargo.toml for Rust dependencies.
    async fn parse_cargo_toml(&self) -> DomainResult<Vec<Dependency>> {
        let cargo_path = self.config.project_root.join("Cargo.toml");
        if !cargo_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&cargo_path).await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to read Cargo.toml: {}", e)))?;

        let mut dependencies = Vec::new();
        let mut in_deps = false;
        let mut in_dev_deps = false;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed == "[dependencies]" {
                in_deps = true;
                in_dev_deps = false;
            } else if trimmed == "[dev-dependencies]" {
                in_deps = false;
                in_dev_deps = true;
            } else if trimmed.starts_with('[') {
                in_deps = false;
                in_dev_deps = false;
            } else if (in_deps || in_dev_deps) && !trimmed.is_empty() && !trimmed.starts_with('#') {
                // Parse dependency line
                if let Some((name, version_part)) = trimmed.split_once('=') {
                    let name = name.trim().to_string();
                    let version = if version_part.contains('{') {
                        // Table format: dep = { version = "1.0" }
                        version_part
                            .split("version")
                            .nth(1)
                            .and_then(|s| s.split('"').nth(1))
                            .map(|s| s.to_string())
                    } else {
                        // Simple format: dep = "1.0"
                        version_part.trim().trim_matches('"').to_string().into()
                    };

                    dependencies.push(Dependency {
                        name,
                        version,
                        source: "Cargo.toml".to_string(),
                        is_dev: in_dev_deps,
                    });
                }
            }
        }

        Ok(dependencies)
    }

    /// Parse package.json for Node dependencies.
    async fn parse_package_json(&self) -> DomainResult<Vec<Dependency>> {
        let pkg_path = self.config.project_root.join("package.json");
        if !pkg_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&pkg_path).await
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to read package.json: {}", e)))?;

        let json: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| DomainError::ValidationFailed(format!("Failed to parse package.json: {}", e)))?;

        let mut dependencies = Vec::new();

        // Regular dependencies
        if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
            for (name, version) in deps {
                dependencies.push(Dependency {
                    name: name.clone(),
                    version: version.as_str().map(|s| s.to_string()),
                    source: "package.json".to_string(),
                    is_dev: false,
                });
            }
        }

        // Dev dependencies
        if let Some(deps) = json.get("devDependencies").and_then(|d| d.as_object()) {
            for (name, version) in deps {
                dependencies.push(Dependency {
                    name: name.clone(),
                    version: version.as_str().map(|s| s.to_string()),
                    source: "package.json".to_string(),
                    is_dev: true,
                });
            }
        }

        Ok(dependencies)
    }

    /// Parse Python dependency files.
    async fn parse_python_deps(&self) -> DomainResult<Vec<Dependency>> {
        let mut dependencies = Vec::new();

        // Try requirements.txt
        let req_path = self.config.project_root.join("requirements.txt");
        if req_path.exists() {
            let content = fs::read_to_string(&req_path).await
                .map_err(|e| DomainError::ValidationFailed(format!("Failed to read requirements.txt: {}", e)))?;

            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }

                // Parse requirement line (name==version or name>=version, etc.)
                let (name, version) = if let Some(idx) = trimmed.find(|c| c == '=' || c == '>' || c == '<') {
                    let name = trimmed[..idx].trim();
                    let version = trimmed[idx..].trim_start_matches(|c| c == '=' || c == '>' || c == '<');
                    (name.to_string(), Some(version.to_string()))
                } else {
                    (trimmed.to_string(), None)
                };

                dependencies.push(Dependency {
                    name,
                    version,
                    source: "requirements.txt".to_string(),
                    is_dev: false,
                });
            }
        }

        // Try pyproject.toml (basic parsing)
        let pyproject_path = self.config.project_root.join("pyproject.toml");
        if pyproject_path.exists() {
            let content = fs::read_to_string(&pyproject_path).await
                .map_err(|e| DomainError::ValidationFailed(format!("Failed to read pyproject.toml: {}", e)))?;

            let mut in_deps = false;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed == "dependencies = [" || trimmed.contains("[project]") {
                    in_deps = true;
                } else if in_deps && trimmed == "]" {
                    in_deps = false;
                } else if in_deps && trimmed.starts_with('"') {
                    let dep = trimmed.trim_matches(|c| c == '"' || c == ',' || c == ' ');
                    if !dep.is_empty() {
                        dependencies.push(Dependency {
                            name: dep.split(|c| c == '>' || c == '<' || c == '=' || c == '[')
                                .next()
                                .unwrap_or(dep)
                                .trim()
                                .to_string(),
                            version: None,
                            source: "pyproject.toml".to_string(),
                            is_dev: false,
                        });
                    }
                }
            }
        }

        Ok(dependencies)
    }

    /// Store gathered context as memories.
    async fn store_memories(&self, report: &ColdStartReport) -> DomainResult<usize> {
        let mut count = 0;
        let namespace = "project";

        // Store project type as semantic memory (long-term)
        self.memory_service.learn(
            "project.type".to_string(),
            format!("Project type: {}", report.project_type),
            namespace,
        ).await?;
        count += 1;

        // Store structure summary
        self.memory_service.learn(
            "project.structure".to_string(),
            report.structure_summary.clone(),
            namespace,
        ).await?;
        count += 1;

        // Store conventions
        for convention in &report.conventions {
            self.memory_service.learn(
                format!("project.convention.{}", convention.name),
                convention.description.clone(),
                namespace,
            ).await?;
            count += 1;
        }

        // Store key dependencies
        let key_deps: Vec<&Dependency> = report.dependencies
            .iter()
            .filter(|d| !d.is_dev)
            .take(20)
            .collect();

        if !key_deps.is_empty() {
            let deps_summary = key_deps.iter()
                .map(|d| {
                    if let Some(ref v) = d.version {
                        format!("  - {} ({})", d.name, v)
                    } else {
                        format!("  - {}", d.name)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            self.memory_service.learn(
                "project.dependencies".to_string(),
                format!("Key project dependencies:\n{}", deps_summary),
                namespace,
            ).await?;
            count += 1;
        }

        Ok(count)
    }

    /// Get configuration.
    pub fn config(&self) -> &ColdStartConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = ColdStartConfig::default();
        assert_eq!(config.max_scan_depth, 5);
        assert!(!config.analyzed_extensions.is_empty());
        assert!(!config.ignore_dirs.is_empty());
        assert!(config.analyze_dependencies);
        assert!(config.detect_conventions);
    }

    #[test]
    fn test_project_type_display() {
        assert_eq!(ProjectType::Rust.to_string(), "Rust");
        assert_eq!(ProjectType::Node.to_string(), "Node.js");
        assert_eq!(ProjectType::Python.to_string(), "Python");
        assert_eq!(ProjectType::Mixed.to_string(), "Multi-language");
    }

    #[test]
    fn test_convention_categories() {
        let conv = Convention {
            name: "test".to_string(),
            description: "Test convention".to_string(),
            confidence: 0.9,
            category: ConventionCategory::Testing,
        };
        assert_eq!(conv.category, ConventionCategory::Testing);
    }
}
