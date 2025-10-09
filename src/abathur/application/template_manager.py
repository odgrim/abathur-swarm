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

        # Check if already cached
        if dest_dir.exists():
            logger.info("template_cache_hit", repo=repo_name, version=version)
            return await self._load_template(dest_dir, version)

        logger.info("cloning_template", repo=repo_url, version=version, dest=str(dest_dir))

        # Clone the repository
        cmd = [
            "git",
            "clone",
            "--depth",
            "1",
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

            # Remove .git directory
            git_dir = dest_dir / ".git"
            if git_dir.exists():
                shutil.rmtree(git_dir)

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

        # Find agent definitions
        agents_dir = template_dir / ".claude" / "agents"
        agents = []
        if agents_dir.exists():
            agents = [f.stem for f in agents_dir.glob("*.yaml")]

        # Find config files
        config_path_candidate = template_dir / ".abathur" / "config.yaml"
        config_path: Path | None = config_path_candidate if config_path_candidate.exists() else None

        mcp_config_path_candidate = template_dir / ".claude" / "mcp.json"
        if not mcp_config_path_candidate.exists():
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
            warnings.append("No MCP configuration found (.claude/mcp.json or .mcp.json)")

        # Validate agent definitions
        for agent_name in template.agents:
            agent_file = agents_dir / f"{agent_name}.yaml"
            if agent_file.exists():
                try:
                    import yaml

                    with open(agent_file) as f:
                        agent_def = yaml.safe_load(f)

                    # Check required fields
                    if "name" not in agent_def:
                        errors.append(f"Agent {agent_name}: missing 'name' field")
                    if "specialization" not in agent_def:
                        warnings.append(f"Agent {agent_name}: missing 'specialization' field")
                except Exception as e:
                    errors.append(f"Agent {agent_name}: invalid YAML - {e}")

        return ValidationResult(valid=len(errors) == 0, errors=errors, warnings=warnings)

    async def install_template(self, template: Template) -> None:
        """Install template to project directory.

        Args:
            template: Template to install

        Raises:
            RuntimeError: If installation fails
        """
        logger.info("installing_template", name=template.name, version=template.version)

        try:
            # Copy .claude directory
            claude_src = template.path / ".claude"
            claude_dest = self.project_root / ".claude"

            if claude_src.exists():
                if claude_dest.exists():
                    logger.warning("claude_dir_exists", path=str(claude_dest))
                else:
                    shutil.copytree(claude_src, claude_dest)
                    logger.info("copied_claude_dir", dest=str(claude_dest))

            # Copy .abathur directory
            abathur_src = template.path / ".abathur"
            abathur_dest = self.project_root / ".abathur"

            if abathur_src.exists():
                if abathur_dest.exists():
                    # Merge config files
                    self._merge_configs(abathur_src, abathur_dest)
                else:
                    shutil.copytree(abathur_src, abathur_dest)
                    logger.info("copied_abathur_dir", dest=str(abathur_dest))

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
