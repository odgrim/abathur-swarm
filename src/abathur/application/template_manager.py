"""Template management for cloning and validating agent templates."""

import asyncio
import json
import shutil
from dataclasses import dataclass
from pathlib import Path

from abathur.infrastructure.logger import get_logger

logger = get_logger(__name__)


@dataclass
class Template:
    """Represents an agent template."""

    name: str
    version: str
    path: Path
    agents: list[str]  # List of agent definition files
    config_path: Path | None = None
    mcp_config_path: Path | None = None


@dataclass
class ValidationResult:
    """Result of template validation."""

    valid: bool
    errors: list[str]
    warnings: list[str]

    @property
    def is_valid(self) -> bool:
        """Alias for valid attribute for backwards compatibility."""
        return self.valid


class TemplateManager:
    """Manages agent templates - cloning, caching, and validation."""

    def __init__(self, cache_dir: Path | None = None, project_root: Path | None = None):
        """Initialize template manager.

        Args:
            cache_dir: Directory for caching templates (default: ~/.abathur/cache/templates)
            project_root: Project root directory (default: current directory)
        """
        self.cache_dir = cache_dir or (Path.home() / ".abathur" / "cache" / "templates")
        self.project_root = project_root or Path.cwd()
        self.cache_dir.mkdir(parents=True, exist_ok=True)

    async def clone_template(self, repo_url: str, version: str = "main") -> Template:
        """Clone a template repository.

        Args:
            repo_url: Git repository URL or short name (e.g., "owner/repo")
            version: Git tag, branch, or commit

        Returns:
            Template object

        Raises:
            RuntimeError: If cloning fails
        """
        # Convert short name to full URL if needed
        if "/" in repo_url and not repo_url.startswith("http"):
            repo_url = f"https://github.com/{repo_url}.git"

        # Extract repo name from URL
        repo_name = repo_url.rstrip(".git").split("/")[-1]
        dest_dir = self.cache_dir / f"{repo_name}-{version}"

        # Check if already cached - pull latest if it exists
        if dest_dir.exists():
            logger.info("template_cache_hit", repo=repo_name, version=version)
            try:
                # Pull latest changes
                logger.info("pulling_template_updates", repo=repo_name, version=version)

                # Fetch and checkout the specified version
                pull_cmd = ["git", "-C", str(dest_dir), "fetch", "origin"]
                process = await asyncio.create_subprocess_exec(
                    *pull_cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
                )
                await process.communicate()

                # Checkout the version (branch/tag)
                checkout_cmd = ["git", "-C", str(dest_dir), "checkout", version]
                process = await asyncio.create_subprocess_exec(
                    *checkout_cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
                )
                await process.communicate()

                # Pull latest for the version
                pull_cmd = ["git", "-C", str(dest_dir), "pull", "origin", version]
                process = await asyncio.create_subprocess_exec(
                    *pull_cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
                )
                stdout, stderr = await process.communicate()

                if process.returncode == 0:
                    logger.info("template_updated", repo=repo_name, version=version)
                else:
                    logger.warning("template_pull_failed", error=stderr.decode())

            except Exception as e:
                logger.warning("template_update_error", error=str(e))
                # Continue with cached version even if pull fails

            return await self._load_template(dest_dir, version)

        logger.info("cloning_template", repo=repo_url, version=version, dest=str(dest_dir))

        # Clone the repository
        cmd = [
            "git",
            "clone",
            "--branch",
            version,
            repo_url,
            str(dest_dir),
        ]

        try:
            process = await asyncio.create_subprocess_exec(
                *cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
            )
            stdout, stderr = await process.communicate()

            if process.returncode != 0:
                error_msg = stderr.decode()
                logger.error("template_clone_failed", error=error_msg)
                raise RuntimeError(f"Failed to clone template: {error_msg}")

            logger.info("template_cloned", repo=repo_name, version=version)
            return await self._load_template(dest_dir, version)

        except Exception as e:
            logger.error("template_clone_error", error=str(e))
            # Cleanup on failure
            if dest_dir.exists():
                shutil.rmtree(dest_dir)
            raise

    async def _load_template(self, template_dir: Path, version: str) -> Template:
        """Load template from directory.

        Args:
            template_dir: Path to template directory
            version: Template version

        Returns:
            Template object
        """
        name = template_dir.name

        # Find agent definitions (recursively search for .yaml and .md files)
        agents_dir = template_dir / ".claude" / "agents"
        agents = []
        if agents_dir.exists():
            # Collect all agent files, including in subdirectories
            # Skip .git directories
            agent_files = set()
            for ext in ["*.yaml", "*.md"]:
                for agent_file in agents_dir.rglob(ext):
                    # Skip files in .git directories
                    if ".git" in agent_file.parts:
                        continue
                    # Use the filename stem only
                    agent_files.add(agent_file.stem)
            agents = sorted(agent_files)

        # Find config files
        config_path_candidate = template_dir / ".abathur" / "config.yaml"
        config_path: Path | None = config_path_candidate if config_path_candidate.exists() else None

        # MCP config is in .mcp.json at project root
        mcp_config_path_candidate = template_dir / ".mcp.json"
        mcp_config_path: Path | None = (
            mcp_config_path_candidate if mcp_config_path_candidate.exists() else None
        )

        return Template(
            name=name,
            version=version,
            path=template_dir,
            agents=agents,
            config_path=config_path,
            mcp_config_path=mcp_config_path,
        )

    def validate_template(self, template: Template) -> ValidationResult:
        """Validate template structure and required files.

        Args:
            template: Template to validate

        Returns:
            ValidationResult with any errors or warnings
        """
        errors = []
        warnings = []

        # Check for required directories
        claude_dir = template.path / ".claude"
        if not claude_dir.exists():
            errors.append("Missing required directory: .claude/")

        agents_dir = claude_dir / "agents"
        if not agents_dir.exists():
            errors.append("Missing required directory: .claude/agents/")
        elif len(template.agents) == 0:
            warnings.append("No agent definitions found in .claude/agents/")

        # Check for abathur directory
        abathur_dir = template.path / ".abathur"
        if not abathur_dir.exists():
            warnings.append("Optional .abathur/ directory not found")

        # Check config file
        if template.config_path is None:
            warnings.append("No .abathur/config.yaml found")

        # Check MCP config
        if template.mcp_config_path is None:
            warnings.append("No MCP configuration found (.mcp.json at project root)")

        # Validate agent definitions
        for agent_name in template.agents:
            # Search for agent file in multiple locations
            agent_file = None
            for ext in [".yaml", ".md"]:
                # Check abathur subdirectory first
                candidate = agents_dir / "abathur" / f"{agent_name}{ext}"
                if candidate.exists():
                    agent_file = candidate
                    break
                # Check root agents directory
                candidate = agents_dir / f"{agent_name}{ext}"
                if candidate.exists():
                    agent_file = candidate
                    break

            if agent_file and agent_file.exists():
                try:
                    import yaml

                    with open(agent_file) as f:
                        content = f.read()
                        if agent_file.suffix == ".md":
                            # Parse frontmatter from .md files
                            if content.startswith("---"):
                                parts = content.split("---", 2)
                                if len(parts) >= 3:
                                    agent_def = yaml.safe_load(parts[1])
                                else:
                                    errors.append(f"Agent {agent_name}: invalid frontmatter")
                                    continue
                            else:
                                errors.append(
                                    f"Agent {agent_name}: .md file must have YAML frontmatter"
                                )
                                continue
                        else:
                            agent_def = yaml.safe_load(content)

                    # Check required fields
                    if "name" not in agent_def:
                        errors.append(f"Agent {agent_name}: missing 'name' field")
                    if "description" not in agent_def and "specialization" not in agent_def:
                        warnings.append(
                            f"Agent {agent_name}: missing 'description' or 'specialization' field"
                        )
                except Exception as e:
                    errors.append(f"Agent {agent_name}: invalid YAML - {e}")

        return ValidationResult(valid=len(errors) == 0, errors=errors, warnings=warnings)

    async def install_template(self, template: Template) -> None:
        """Install template to project directory.

        Updates existing directories intelligently:
        - Core agent templates are always updated
        - MCP config is always updated
        - Custom agents (not in template) are preserved

        Args:
            template: Template to install

        Raises:
            RuntimeError: If installation fails
        """
        logger.info("installing_template", name=template.name, version=template.version)

        # Ignore function to exclude .git directories
        def ignore_git(src: str, names: list[str]) -> set[str]:
            """Ignore .git directories when copying."""
            return {".git"} if ".git" in names else set()

        try:
            # Copy .claude directory
            claude_src = template.path / ".claude"
            claude_dest = self.project_root / ".claude"

            if claude_src.exists():
                if claude_dest.exists():
                    logger.info("updating_claude_dir", path=str(claude_dest))
                    self._update_claude_directory(claude_src, claude_dest)
                else:
                    shutil.copytree(claude_src, claude_dest, ignore=ignore_git)
                    logger.info("copied_claude_dir", dest=str(claude_dest))

            # Copy .abathur directory
            abathur_src = template.path / ".abathur"
            abathur_dest = self.project_root / ".abathur"

            if abathur_src.exists():
                if abathur_dest.exists():
                    # Merge config files
                    self._merge_configs(abathur_src, abathur_dest)
                else:
                    shutil.copytree(abathur_src, abathur_dest, ignore=ignore_git)
                    logger.info("copied_abathur_dir", dest=str(abathur_dest))

            # Merge .mcp.json from root to root (preserving user's custom servers)
            mcp_src = template.path / ".mcp.json"
            mcp_dest = self.project_root / ".mcp.json"
            if mcp_src.exists():
                self._merge_mcp_config(mcp_src, mcp_dest)
                logger.info("merged_mcp_config", dest=str(mcp_dest))

            # Create metadata file
            metadata = {
                "template_name": template.name,
                "template_version": template.version,
                "installed_at": str(Path.cwd()),
                "agents": template.agents,
            }

            metadata_file = self.project_root / ".abathur" / "metadata.json"
            metadata_file.parent.mkdir(parents=True, exist_ok=True)
            with open(metadata_file, "w") as f:
                json.dump(metadata, f, indent=2)

            logger.info("template_installed", name=template.name)

        except Exception as e:
            logger.error("template_install_error", error=str(e))
            raise RuntimeError(f"Failed to install template: {e}") from e

    def _update_claude_directory(self, src_dir: Path, dest_dir: Path) -> None:
        """Update .claude directory from template.

        Updates:
        - All agent files from template (overwrites existing)
        - MCP config files (overwrites existing)
        - README and documentation files

        Preserves:
        - Custom agents not in template
        - Custom subdirectories

        Args:
            src_dir: Source .claude directory from template
            dest_dir: Destination .claude directory in project
        """
        updated_count = 0
        preserved_count = 0

        # Update agents directory
        src_agents = src_dir / "agents"
        dest_agents = dest_dir / "agents"

        if src_agents.exists():
            dest_agents.mkdir(parents=True, exist_ok=True)

            # Get list of template agent files (all .md and .yaml files recursively)
            # Skip .git directories
            template_agents = set()
            for ext in ["*.md", "*.yaml"]:
                for agent_file in src_agents.rglob(ext):
                    # Skip files in .git directories
                    if ".git" in agent_file.parts:
                        continue
                    rel_path = agent_file.relative_to(src_agents)
                    template_agents.add(str(rel_path))

            # Update all agent files from template
            for agent_rel_path in template_agents:
                src_file = src_agents / agent_rel_path
                dest_file = dest_agents / agent_rel_path

                # Create subdirectories if needed
                dest_file.parent.mkdir(parents=True, exist_ok=True)

                # Copy/update file
                shutil.copy2(src_file, dest_file)
                updated_count += 1

            logger.info("updated_agents", count=updated_count)

            # Count custom agents that were preserved
            for ext in ["*.md", "*.yaml"]:
                for custom_file in dest_agents.rglob(ext):
                    # Skip files in .git directories
                    if ".git" in custom_file.parts:
                        continue
                    rel_path = custom_file.relative_to(dest_agents)
                    if str(rel_path) not in template_agents:
                        preserved_count += 1

            if preserved_count > 0:
                logger.info("preserved_custom_agents", count=preserved_count)

        # Update README and documentation
        for doc_file in ["README.md", "AGENTS.md"]:
            src_doc = src_dir / doc_file
            dest_doc = dest_dir / doc_file
            if src_doc.exists():
                shutil.copy2(src_doc, dest_doc)
                logger.info("updated_documentation", file=doc_file)

        # Update settings.json (Claude Code project settings)
        src_settings = src_dir / "settings.json"
        dest_settings = dest_dir / "settings.json"
        if src_settings.exists():
            # For settings.json, we just copy/merge the enableAllProjectMcpServers setting
            self._merge_claude_settings(src_settings, dest_settings)
            logger.info("merged_claude_settings", file="settings.json")

    def _merge_configs(self, src_dir: Path, dest_dir: Path) -> None:
        """Merge configuration files from source to destination.

        Args:
            src_dir: Source configuration directory
            dest_dir: Destination configuration directory
        """
        # For now, just copy files that don't exist
        # In future, could merge YAML files intelligently
        for src_file in src_dir.glob("*.yaml"):
            dest_file = dest_dir / src_file.name
            if not dest_file.exists():
                shutil.copy2(src_file, dest_file)
                logger.info("copied_config_file", file=src_file.name)

    def _merge_mcp_config(self, src_file: Path, dest_file: Path) -> None:
        """Merge .mcp.json, only updating Abathur-specific MCP servers.

        This preserves any custom user-defined MCP servers while updating
        only the Abathur servers (those with keys starting with "abathur-").

        Args:
            src_file: Source .mcp.json from template
            dest_file: Destination .mcp.json in project
        """
        # Load template MCP config
        with open(src_file) as f:
            template_config = json.load(f)

        # Load existing project MCP config if it exists
        if dest_file.exists():
            with open(dest_file) as f:
                project_config = json.load(f)
        else:
            project_config = {"mcpServers": {}}

        # Ensure mcpServers key exists
        if "mcpServers" not in project_config:
            project_config["mcpServers"] = {}
        if "mcpServers" not in template_config:
            template_config["mcpServers"] = {}

        # Update only Abathur-specific servers (those starting with "abathur-")
        abathur_servers = {
            key: value
            for key, value in template_config["mcpServers"].items()
            if key.startswith("abathur-")
        }

        # Merge: update/add Abathur servers, preserve others
        project_config["mcpServers"].update(abathur_servers)

        # Write merged config
        with open(dest_file, "w") as f:
            json.dump(project_config, f, indent=2)

        logger.info(
            "merged_mcp_config",
            updated_servers=list(abathur_servers.keys()),
            total_servers=len(project_config["mcpServers"]),
        )

    def _merge_claude_settings(self, src_file: Path, dest_file: Path) -> None:
        """Merge .claude/settings.json.

        This merges Claude Code settings like enableAllProjectMcpServers.

        Args:
            src_file: Source settings.json from template
            dest_file: Destination settings.json in project
        """
        # Load template settings
        with open(src_file) as f:
            template_settings = json.load(f)

        # Load existing project settings if it exists
        if dest_file.exists():
            with open(dest_file) as f:
                project_settings = json.load(f)
        else:
            project_settings = {}

        # Merge settings from template (template takes precedence)
        project_settings.update(template_settings)

        # Write merged config
        with open(dest_file, "w") as f:
            json.dump(project_settings, f, indent=2)

        logger.info("merged_claude_settings")

    def list_cached_templates(self) -> list[Template]:
        """List all cached templates.

        Returns:
            List of cached Template objects
        """
        templates: list[Template] = []
        if not self.cache_dir.exists():
            return templates

        for template_dir in self.cache_dir.iterdir():
            if template_dir.is_dir():
                try:
                    # Extract version from directory name (e.g., "repo-v1.0.0")
                    parts = template_dir.name.rsplit("-", 1)
                    version = parts[1] if len(parts) > 1 else "unknown"

                    # Synchronously load template (blocking is fine for listing)
                    import asyncio

                    loop = asyncio.new_event_loop()
                    template = loop.run_until_complete(self._load_template(template_dir, version))
                    loop.close()

                    templates.append(template)
                except Exception as e:
                    logger.warning(
                        "failed_to_load_cached_template", dir=str(template_dir), error=str(e)
                    )

        return templates

    def list_templates(self) -> list[Template]:
        """List all templates (alias for list_cached_templates).

        Returns:
            List of cached Template objects
        """
        return self.list_cached_templates()

    def clear_cache(self, template_name: str | None = None) -> None:
        """Clear template cache.

        Args:
            template_name: If provided, only clear this template. Otherwise clear all.
        """
        if template_name:
            # Clear specific template
            for template_dir in self.cache_dir.glob(f"{template_name}-*"):
                if template_dir.is_dir():
                    shutil.rmtree(template_dir)
                    logger.info("cleared_template_cache", name=template_name)
        else:
            # Clear entire cache
            if self.cache_dir.exists():
                shutil.rmtree(self.cache_dir)
                self.cache_dir.mkdir(parents=True, exist_ok=True)
                logger.info("cleared_all_template_cache")
