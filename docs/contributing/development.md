# Development Setup

This guide walks you through setting up your local development environment for contributing to Abathur.

## Prerequisites

Before you begin, ensure you have these tools installed:

- **Rust 1.83 or higher**: Install via [rustup](https://rustup.rs/)
- **Git**: For version control
- **SQLite 3**: Usually pre-installed on macOS and Linux
- **A code editor**: VS Code, RustRover, or your preferred IDE

### Verify Prerequisites

```bash
# Check Rust version
rustc --version
# Should output: rustc 1.83.0 or higher

# Check Cargo version
cargo --version

# Check Git version
git --version

# Check SQLite version
sqlite3 --version
```

## Clone the Repository

### Fork and Clone

1. Fork the repository on GitHub
2. Clone your fork locally:

```bash
git clone https://github.com/YOUR_USERNAME/abathur.git
cd abathur
```

3. Add upstream remote:

```bash
git remote add upstream https://github.com/yourorg/abathur.git
```

4. Verify remotes:

```bash
git remote -v
```

**Expected Output**:
```
origin    https://github.com/YOUR_USERNAME/abathur.git (fetch)
origin    https://github.com/YOUR_USERNAME/abathur.git (push)
upstream  https://github.com/yourorg/abathur.git (fetch)
upstream  https://github.com/yourorg/abathur.git (push)
```

## Install Rust Toolchain Components

Install required Rust toolchain components:

```bash
# Install rustfmt (code formatter)
rustup component add rustfmt

# Install clippy (linter)
rustup component add clippy

# Verify installation
cargo fmt --version
cargo clippy --version
```

## Build from Source

### Debug Build (Fast Compilation)

For development, use debug builds:

```bash
# Build in debug mode (default)
cargo build
```

**Build Time**: 1-2 minutes on first build (subsequent builds are incremental)

The binary will be at `target/debug/abathur`.

### Release Build (Optimized)

For testing performance:

```bash
# Build with optimizations
cargo build --release
```

**Build Time**: 2-5 minutes (optimizations enabled)

The binary will be at `target/release/abathur`.

### Verify Build

```bash
# Run the built binary
./target/debug/abathur --version

# Or use cargo run
cargo run -- --version
```

## Run in Development Mode

### Running Commands

Use `cargo run` for development:

```bash
# Show help
cargo run -- --help

# Initialize project
cargo run -- init

# List tasks
cargo run -- task list

# Run with specific arguments
cargo run -- task status
```

### Development with Logging

Enable logging for debugging:

```bash
# Info level logging
RUST_LOG=info cargo run -- task list

# Debug level logging
RUST_LOG=debug cargo run -- task list

# Trace specific module
RUST_LOG=abathur_cli::domain::task=trace cargo run -- task show <id>

# Multiple modules
RUST_LOG=abathur_cli::domain=debug,abathur_cli::infrastructure=trace cargo run -- swarm start
```

**Log Levels**:
- `error`: Only errors
- `warn`: Warnings and errors
- `info`: General information (default)
- `debug`: Detailed debugging information
- `trace`: Very detailed trace information

### Watch Mode (Auto-Rebuild)

Install `cargo-watch` for automatic rebuilds:

```bash
# Install cargo-watch
cargo install cargo-watch

# Watch and run tests on file changes
cargo watch -x test

# Watch and run specific command
cargo watch -x "run -- task list"

# Watch, test, and run
cargo watch -x test -x "run -- --help"
```

## IDE Setup Recommendations

### Visual Studio Code

1. Install recommended extensions:
   - **rust-analyzer**: Rust language server
   - **CodeLLDB**: Debugging support
   - **crates**: Crate version management
   - **Even Better TOML**: TOML syntax highlighting

2. Add workspace settings (`.vscode/settings.json`):

```json
{
  "rust-analyzer.checkOnSave.command": "clippy",
  "rust-analyzer.cargo.features": "all",
  "editor.formatOnSave": true,
  "editor.rulers": [100],
  "[rust]": {
    "editor.defaultFormatter": "rust-lang.rust-analyzer"
  }
}
```

3. Add launch configuration (`.vscode/launch.json`):

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "lldb",
      "request": "launch",
      "name": "Debug Abathur",
      "cargo": {
        "args": ["build", "--bin=abathur", "--package=abathur-cli"]
      },
      "args": ["task", "list"],
      "cwd": "${workspaceFolder}"
    }
  ]
}
```

### RustRover / IntelliJ IDEA

1. Install Rust plugin
2. Open project: **File → Open → Select abathur directory**
3. Configure Rust toolchain: **Settings → Languages & Frameworks → Rust**
4. Enable Clippy: **Settings → Languages & Frameworks → Rust → Linter → Clippy**

### Neovim / Vim

1. Install rust-analyzer LSP
2. Add to your configuration:

```lua
-- Using nvim-lspconfig
require('lspconfig').rust_analyzer.setup{
  settings = {
    ["rust-analyzer"] = {
      checkOnSave = {
        command = "clippy"
      }
    }
  }
}
```

## Debugging Tips

### Using rust-gdb or rust-lldb

```bash
# Build with debug symbols
cargo build

# Debug with lldb (macOS)
rust-lldb ./target/debug/abathur

# Debug with gdb (Linux)
rust-gdb ./target/debug/abathur
```

### Print Debugging

Use `dbg!` macro for quick debugging:

```rust
let task_id = uuid::Uuid::new_v4();
dbg!(&task_id);  // Prints with file and line number

let result = calculate_priority(task);
dbg!(result);  // Prints variable name and value
```

### Using tracing

The project uses `tracing` for structured logging:

```rust
use tracing::{debug, info, warn, error};

#[tracing::instrument]
async fn process_task(task: &Task) -> Result<()> {
    info!("Processing task: {}", task.id);
    debug!("Task details: {:?}", task);
    // Function automatically traces entry/exit
    Ok(())
}
```

Enable tracing output:

```bash
RUST_LOG=debug cargo run -- task list
```

### Inspecting Database

View the SQLite database directly:

```bash
# Open database
sqlite3 .abathur/tasks.db

# List tables
.tables

# Show schema
.schema tasks

# Query tasks
SELECT * FROM tasks WHERE status = 'pending';

# Exit
.exit
```

### Common Issues

#### Issue: `linker cc not found`

**Solution** (macOS):
```bash
xcode-select --install
```

**Solution** (Ubuntu/Debian):
```bash
sudo apt-get install build-essential
```

#### Issue: SQLite errors

**Solution**: Remove and reinitialize database:
```bash
rm -rf .abathur/
cargo run -- init
```

#### Issue: Slow compilation

**Solution**: Use faster linker:

Add to `~/.cargo/config.toml`:
```toml
[target.x86_64-apple-darwin]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]

[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```

Install lld:
```bash
# macOS
brew install llvm

# Ubuntu/Debian
sudo apt-get install lld
```

#### Issue: `cargo run` uses old binary

**Solution**: Force clean rebuild:
```bash
cargo clean
cargo build
```

## Project Structure

Understanding the codebase layout:

```
abathur/
├── src/
│   ├── main.rs              # CLI entry point
│   ├── lib.rs               # Library root
│   ├── cli/                 # CLI command implementations
│   ├── application/         # Application services
│   ├── domain/              # Domain models and logic
│   │   ├── models/          # Core domain entities
│   │   └── ports/           # Repository trait definitions
│   └── infrastructure/      # External integrations
│       ├── database/        # SQLite repository implementations
│       ├── config/          # Configuration management
│       └── logging/         # Logging setup
├── tests/                   # Integration tests
├── benches/                 # Performance benchmarks
├── .abathur/               # Project configuration (created by init)
└── Cargo.toml              # Rust project manifest
```

## Next Steps

Now that your development environment is set up:

1. Read the [Testing Guidelines](testing.md) to understand how to run and write tests
2. Review the [Style Guide](style-guide.md) for coding standards
3. Check out [open issues](https://github.com/yourorg/abathur/issues) labeled `good-first-issue`
4. Read the [Architecture Documentation](../explanation/architecture.md) to understand system design

## Getting Help

- **Documentation**: Browse the `docs/` directory
- **GitHub Issues**: [Report bugs or ask questions](https://github.com/yourorg/abathur/issues)
- **GitHub Discussions**: [Community support](https://github.com/yourorg/abathur/discussions)

## Related Documentation

- [Testing Guidelines](testing.md) - How to run and write tests
- [Style Guide](style-guide.md) - Code and documentation standards
- [Architecture](../explanation/architecture.md) - System design overview
