# Homebrew Formula for Abathur
# To install: brew install yourorg/tap/abathur

class Abathur < Formula
  include Language::Python::Virtualenv

  desc "Hivemind Swarm Management System for Claude agents"
  homepage "https://github.com/yourorg/abathur"
  url "https://github.com/yourorg/abathur/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "SHA256_CHECKSUM_HERE"  # Update after release
  license "MIT"

  depends_on "python@3.11"
  depends_on "git"

  resource "anthropic" do
    url "https://files.pythonhosted.org/packages/anthropic-0.18.0.tar.gz"
    sha256 "ANTHROPIC_SHA256"
  end

  resource "typer" do
    url "https://files.pythonhosted.org/packages/typer-0.12.0.tar.gz"
    sha256 "TYPER_SHA256"
  end

  resource "rich" do
    url "https://files.pythonhosted.org/packages/rich-13.7.0.tar.gz"
    sha256 "RICH_SHA256"
  end

  resource "pydantic" do
    url "https://files.pythonhosted.org/packages/pydantic-2.5.0.tar.gz"
    sha256 "PYDANTIC_SHA256"
  end

  resource "python-dotenv" do
    url "https://files.pythonhosted.org/packages/python-dotenv-1.0.0.tar.gz"
    sha256 "DOTENV_SHA256"
  end

  resource "keyring" do
    url "https://files.pythonhosted.org/packages/keyring-24.3.0.tar.gz"
    sha256 "KEYRING_SHA256"
  end

  resource "structlog" do
    url "https://files.pythonhosted.org/packages/structlog-24.1.0.tar.gz"
    sha256 "STRUCTLOG_SHA256"
  end

  resource "aiosqlite" do
    url "https://files.pythonhosted.org/packages/aiosqlite-0.19.0.tar.gz"
    sha256 "AIOSQLITE_SHA256"
  end

  resource "psutil" do
    url "https://files.pythonhosted.org/packages/psutil-5.9.0.tar.gz"
    sha256 "PSUTIL_SHA256"
  end

  resource "pyyaml" do
    url "https://files.pythonhosted.org/packages/pyyaml-6.0.1.tar.gz"
    sha256 "PYYAML_SHA256"
  end

  def install
    virtualenv_install_with_resources
  end

  def caveats
    <<~EOS
      Abathur has been installed!

      To get started:
        1. Set your Anthropic API key:
           abathur config set-key YOUR_API_KEY

        2. Initialize a project:
           abathur init

        3. View available commands:
           abathur --help

      Documentation: https://github.com/yourorg/abathur/tree/main/docs

      Note: The CLI entry point uses the module invocation workaround.
      If you encounter issues, use:
        python3 -m abathur.cli.main <command>
    EOS
  end

  test do
    assert_match "Abathur version", shell_output("#{bin}/abathur version")
  end
end
