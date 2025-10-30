# Contributing to Abathur Swarm

Welcome to the Abathur Swarm contributing guide! This section helps you contribute to the project, whether you're improving documentation, fixing bugs, adding features, or enhancing the agentic orchestration system.

We value all contributions and strive to make the contribution process clear, welcoming, and efficient. This guide provides everything you need to get started and make meaningful contributions.

## Available Guides

### Getting Started
- **[Contribution Overview](overview.md)**: How to contribute and what we're looking for *(coming soon)*
- **[Development Setup](development-setup.md)**: Set up your local development environment *(coming soon)*
- **[Code of Conduct](code-of-conduct.md)**: Community guidelines and expectations *(coming soon)*

### Code Contributions
- **[Development Workflow](workflow.md)**: Git workflow, branches, and pull requests *(coming soon)*
- **[Coding Standards](coding-standards.md)**: Rust style guide and best practices *(coming soon)*
- **[Testing Requirements](testing.md)**: Unit, integration, and property test expectations *(coming soon)*
- **[Agent Development Guide](agent-development.md)**: How to create new specialized agents *(coming soon)*

### Documentation Contributions
- **[Documentation Guide](documentation-guide.md)**: How to write and improve documentation *(coming soon)*
- **[Diátaxis Framework](diataxis-framework.md)**: Understanding our documentation structure *(coming soon)*
- **[Writing Tutorials](writing-tutorials.md)**: Guidelines for creating learning-oriented content *(coming soon)*
- **[Writing How-To Guides](writing-howtos.md)**: Creating problem-solving documentation *(coming soon)*
- **[Writing Reference Docs](writing-reference.md)**: Comprehensive technical documentation *(coming soon)*
- **[Writing Explanations](writing-explanations.md)**: Conceptual and architectural content *(coming soon)*

### Review and Release
- **[Pull Request Process](pull-requests.md)**: What happens after you submit *(coming soon)*
- **[Code Review Guidelines](code-review.md)**: What reviewers look for *(coming soon)*
- **[Release Process](release-process.md)**: How releases are managed *(coming soon)*

## Ways to Contribute

### Documentation
- Fix typos, improve clarity, or update outdated content
- Write new tutorials, how-to guides, or explanations
- Add diagrams and visual aids
- Improve code examples

### Code
- Fix bugs reported in issues
- Implement new features or agents
- Improve performance or error handling
- Add tests to increase coverage
- Refactor for better maintainability

### Community
- Answer questions in discussions
- Report bugs with detailed reproduction steps
- Suggest features or improvements
- Review pull requests
- Help triage issues

### Testing and Quality
- Test new features and report issues
- Improve test coverage
- Add property tests for edge cases
- Performance benchmarking

## Quick Start for Contributors

1. **Fork the repository**: Create your own copy of Abathur Swarm
2. **Clone locally**: `git clone https://github.com/your-username/abathur-swarm`
3. **Create a branch**: `git checkout -b feature/your-feature-name`
4. **Make changes**: Follow coding standards and add tests
5. **Run tests**: `cargo test` and ensure all pass
6. **Commit**: Write clear, descriptive commit messages
7. **Push**: `git push origin feature/your-feature-name`
8. **Open PR**: Submit a pull request with detailed description

See [Development Workflow](workflow.md) for complete details.

## Contribution Standards

### Code Quality
- **Tests required**: All new code must include tests
- **Clippy clean**: No clippy warnings allowed
- **Formatted**: Run `cargo fmt` before committing
- **Documented**: Public APIs need doc comments
- **Type-safe**: Leverage Rust's type system

### Documentation Quality
- **Clear and concise**: Short sentences, simple language
- **Working examples**: All code examples must work
- **Proper category**: Follow Diátaxis framework
- **Accessible**: Follow accessibility guidelines
- **Cross-linked**: Link to related documentation

### Commit Standards
- **Descriptive**: Explain the "why" not just the "what"
- **Atomic**: One logical change per commit
- **Sign-off**: Include `Signed-off-by` line
- **Reference issues**: Link to relevant issue numbers

## Getting Help

### Questions and Discussions
- **Documentation questions**: Open a discussion
- **Bug reports**: Create an issue with reproduction steps
- **Feature proposals**: Start with a discussion to gather feedback
- **Pull request help**: Comment on your PR if you're stuck

### Communication Channels
- **GitHub Issues**: Bug reports and feature requests
- **GitHub Discussions**: Questions and general discussion
- **Pull Requests**: Code review and technical discussion

## Diátaxis Framework Context

**Contributing guides are mixed-type documentation**: They combine elements from multiple Diátaxis categories to serve different contributor needs:

- **Tutorial elements**: Getting started guides for new contributors
- **How-to elements**: Specific workflows like "How to write tests"
- **Reference elements**: Coding standards and API guidelines
- **Explanation elements**: Project philosophy and design principles

This flexibility helps contributors at different stages find the information they need.

## Recognition

We value all contributions and recognize contributors:
- All contributors are acknowledged in release notes
- Significant contributions highlighted in the changelog
- Active contributors may be invited as maintainers

## Related Documentation

**Before contributing**:
- Read [System Architecture](../explanation/architecture.md) to understand the codebase
- Review [Reference Documentation](../reference/index.md) for technical details
- Check existing [Issues](https://github.com/your-repo/abathur-swarm/issues) for planned work

**For specific contributions**:
- **Bug fixes** → Review [Testing Requirements](testing.md)
- **New features** → Start with [Feature Proposal](feature-proposals.md)
- **Documentation** → Follow [Documentation Guide](documentation-guide.md)
- **Agents** → Read [Agent Development Guide](agent-development.md)

## Code of Conduct

We are committed to providing a welcoming and inclusive environment. All participants are expected to uphold our [Code of Conduct](code-of-conduct.md). Please report any unacceptable behavior to the maintainers.

## License

By contributing to Abathur Swarm, you agree that your contributions will be licensed under the same license as the project (see [LICENSE](../../LICENSE)).

---

*Ready to contribute? Start with [Development Setup](development-setup.md) or browse [open issues](https://github.com/your-repo/abathur-swarm/issues).*
