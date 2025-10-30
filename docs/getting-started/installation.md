# Installation

Welcome to Abathur! This guide will help you install the Abathur CLI orchestration system on your machine. By the end of this guide, you'll have Abathur installed and ready to orchestrate swarms of specialized Claude agents.

## Prerequisites

Before installing Abathur, ensure you have the following installed on your system:

- **Rust 1.83 or higher** - Install via [rustup](https://rustup.rs/)
- **Git** - For cloning the repository ([Download Git](https://git-scm.com/downloads))
- **SQLite** - Usually pre-installed on macOS and Linux
- **Anthropic API Key** - Required for Claude agent execution (optional for core development)

### System Requirements

Abathur supports the following platforms:

- **macOS** - 10.15 (Catalina) or later
- **Linux** - Most modern distributions (Ubuntu 20.04+, Fedora 34+, etc.)
- **Windows** - Windows 10/11 with WSL2 recommended

### Checking Rust Installation

Verify your Rust installation:

```bash
rustc --version
cargo --version
```

**Expected Output**:
```
rustc 1.83.0 (90b35a623 2024-11-26)
cargo 1.83.0 (5ffbef321 2024-10-29)
```

!!! tip "Installing Rust"
    If you don't have Rust installed, use rustup:
    ```bash
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    ```
    Follow the prompts and restart your terminal after installation.

### Anthropic API Key

To use Claude agents, you'll need an Anthropic API key:

1. Sign up at [Anthropic Console](https://console.anthropic.com/)
2. Navigate to **API Keys** section
3. Generate a new API key
4. Save it securely - you'll need it later

!!! warning "API Key Security"
    Never commit your API key to version control. Use environment variables or secure credential managers.

## Installation Methods

Choose the installation method that best suits your needs:

### Method 1: From Source (Recommended)

This is the recommended method for most users, especially during active development.

```bash
# Clone the repository
git clone https://github.com/odgrim/abathur-swarm.git
cd abathur-swarm

# Build the release version
cargo build --release

# Install to your local system
cargo install --path .
```

**Expected Output**:
```
   Compiling abathur-cli v0.1.0 (/path/to/abathur-swarm)
    Finished release [optimized] target(s) in 2m 15s
  Installing /Users/username/.cargo/bin/abathur
   Installed package `abathur-cli v0.1.0` (executable `abathur`)
```

The `abathur` binary will be installed to `~/.cargo/bin/`, which should be in your PATH.

!!! tip "Build Time"
    The first build may take 2-5 minutes as Cargo compiles all dependencies. Subsequent builds will be much faster.

### Method 2: Using Cargo Install

Once Abathur is published to crates.io, you can install it directly:

```bash
cargo install abathur-cli
```

!!! info "Coming Soon"
    This method will be available once Abathur v0.1.0 is published to crates.io.

### Method 3: Pre-built Binaries

Pre-built binaries will be available for major platforms from the GitHub Releases page.

**Download for your platform**:

=== "macOS (Apple Silicon)"
    ```bash
    curl -L https://github.com/odgrim/abathur-swarm/releases/latest/download/abathur-aarch64-apple-darwin.tar.gz -o abathur.tar.gz
    tar xzf abathur.tar.gz
    sudo mv abathur /usr/local/bin/
    ```

=== "macOS (Intel)"
    ```bash
    curl -L https://github.com/odgrim/abathur-swarm/releases/latest/download/abathur-x86_64-apple-darwin.tar.gz -o abathur.tar.gz
    tar xzf abathur.tar.gz
    sudo mv abathur /usr/local/bin/
    ```

=== "Linux (x86_64)"
    ```bash
    curl -L https://github.com/odgrim/abathur-swarm/releases/latest/download/abathur-x86_64-unknown-linux-gnu.tar.gz -o abathur.tar.gz
    tar xzf abathur.tar.gz
    sudo mv abathur /usr/local/bin/
    ```

=== "Windows (WSL2)"
    ```bash
    curl -L https://github.com/odgrim/abathur-swarm/releases/latest/download/abathur-x86_64-pc-windows-msvc.zip -o abathur.zip
    unzip abathur.zip
    # Move to a directory in your PATH
    ```

!!! info "Binary Releases"
    Pre-built binaries will be available starting with version 0.2.0.

### Method 4: Development Installation

For contributors and developers who want to work on Abathur itself:

```bash
# Clone the repository
git clone https://github.com/odgrim/abathur-swarm.git
cd abathur-swarm

# Install Rust development tools
rustup component add rustfmt clippy

# Build in debug mode (faster compilation)
cargo build

# Run without installing
cargo run -- --help
```

This method allows you to make changes and test them immediately.

## Verification

After installation, verify that Abathur is correctly installed:

```bash
abathur --version
```

**Expected Output**:
```
abathur 0.1.0
```

Check available commands:

```bash
abathur --help
```

**Expected Output**:
```
A CLI orchestration system for managing swarms of specialized Claude agents

Usage: abathur [OPTIONS] <COMMAND>

Commands:
  task    Task management operations
  swarm   Swarm orchestration operations
  loop    Loop execution operations
  mcp     MCP server management
  init    Initialize Abathur configuration
  help    Print this message or the help of the given subcommand(s)

Options:
  -c, --config <FILE>  Path to configuration file
  -v, --verbose...     Enable verbose logging (use -vv for debug)
      --json          Output in JSON format
  -h, --help          Print help
  -V, --version       Print version
```

!!! success "Installation Complete!"
    If you see the version number and help output, Abathur is successfully installed!

## Initial Setup

After installing Abathur, initialize your configuration:

```bash
# Initialize database and configuration
abathur init
```

This command will:
1. Create the `.abathur/` directory in your project
2. Initialize the SQLite database with proper schema
3. Generate a default configuration file
4. Set up the template repository

**Expected Output**:
```
✓ Created .abathur/ directory
✓ Initialized database at .abathur/abathur.db
✓ Generated configuration file at .abathur/config.yaml
✓ Cloned template repository from https://github.com/odgrim/abathur-claude-template.git
✓ Abathur initialized successfully!

Next steps:
  1. Set your Anthropic API key: export ANTHROPIC_API_KEY=your_key_here
  2. Review configuration: .abathur/config.yaml
  3. Start using Abathur: abathur task list
```

### Configure API Key

Set your Anthropic API key as an environment variable:

=== "Linux/macOS"
    ```bash
    # Temporary (current session only)
    export ANTHROPIC_API_KEY=your_api_key_here

    # Permanent (add to ~/.bashrc, ~/.zshrc, or ~/.profile)
    echo 'export ANTHROPIC_API_KEY=your_api_key_here' >> ~/.bashrc
    source ~/.bashrc
    ```

=== "Windows (PowerShell)"
    ```powershell
    # Temporary (current session only)
    $env:ANTHROPIC_API_KEY="your_api_key_here"

    # Permanent (system environment variable)
    [System.Environment]::SetEnvironmentVariable('ANTHROPIC_API_KEY', 'your_api_key_here', 'User')
    ```

=== "Windows (WSL2)"
    ```bash
    # Same as Linux/macOS
    export ANTHROPIC_API_KEY=your_api_key_here
    echo 'export ANTHROPIC_API_KEY=your_api_key_here' >> ~/.bashrc
    source ~/.bashrc
    ```

!!! tip "Using a .env File"
    You can also create a `.env` file in your project directory:
    ```bash
    echo "ANTHROPIC_API_KEY=your_api_key_here" > .env
    ```
    Make sure to add `.env` to your `.gitignore`.

## Troubleshooting

### Cargo Binary Not Found

**Symptom**: `abathur: command not found` after installation

**Solution**: Add Cargo's binary directory to your PATH:

```bash
# For Linux/macOS
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc

# For Fish shell
fish_add_path ~/.cargo/bin
```

Restart your terminal and try again.

### Build Fails on Linux

**Symptom**: Compilation errors related to SQLite or OpenSSL

**Solution**: Install required development libraries:

=== "Ubuntu/Debian"
    ```bash
    sudo apt update
    sudo apt install build-essential libsqlite3-dev libssl-dev pkg-config
    ```

=== "Fedora/RHEL"
    ```bash
    sudo dnf install gcc sqlite-devel openssl-devel pkgconfig
    ```

=== "Arch Linux"
    ```bash
    sudo pacman -S base-devel sqlite openssl pkgconf
    ```

### Windows Installation Issues

**Symptom**: Various compilation errors on Windows

**Solution**: Use WSL2 for the best experience:

1. Install WSL2: [Microsoft WSL Documentation](https://docs.microsoft.com/en-us/windows/wsl/install)
2. Install Ubuntu from Microsoft Store
3. Follow the Linux installation instructions above

Alternatively, ensure you have:
- Visual Studio Build Tools installed
- Windows SDK installed

### Rust Version Too Old

**Symptom**: `error: package requires rustc 1.83 or newer`

**Solution**: Update Rust to the latest version:

```bash
rustup update stable
rustc --version
```

### Permission Denied During Installation

**Symptom**: `Permission denied` when running `cargo install`

**Solution**: Ensure you have write permissions to `~/.cargo/bin/`:

```bash
# Check permissions
ls -la ~/.cargo/bin/

# Fix permissions if needed
chmod u+w ~/.cargo/bin/
```

## Updating Abathur

To update to the latest version:

### From Source

```bash
cd abathur-swarm
git pull origin main
cargo install --path . --force
```

### From Crates.io

```bash
cargo install abathur-cli --force
```

The `--force` flag will overwrite the existing installation.

## Uninstalling

To remove Abathur from your system:

```bash
cargo uninstall abathur-cli
```

This will remove the `abathur` binary from `~/.cargo/bin/`.

To also remove configuration and data:

```bash
rm -rf ~/.abathur        # Remove global configuration
rm -rf .abathur          # Remove project-specific configuration (in project directory)
```

!!! danger "Data Loss Warning"
    Removing `.abathur/` directories will delete all task history, agent outputs, and local configuration. Back up any important data first.

## Next Steps

Now that you have Abathur installed, you're ready to start using it!

**Continue your journey**:

- **[Quickstart Guide](quickstart.md)** - Run your first task in 5 minutes
- **[Configuration](configuration.md)** - Customize Abathur for your needs
- **[CLI Commands Reference](../reference/cli-commands.md)** - Explore all available commands

**Explore tutorials**:

- [Creating Your First Task](../tutorials/first-task.md)
- [Swarm Orchestration Basics](../tutorials/swarm-orchestration.md)
- [MCP Integration](../tutorials/mcp-integration.md)

**Need help?**

- [Troubleshooting Guide](../how-to/troubleshooting.md)
- [GitHub Issues](https://github.com/odgrim/abathur-swarm/issues)
- [GitHub Discussions](https://github.com/odgrim/abathur-swarm/discussions)

---

**Estimated Reading Time**: 10 minutes
**Prerequisites**: Basic command-line knowledge
[Previous: Documentation Home](../index.md) | [Next: Quickstart Guide](quickstart.md)
