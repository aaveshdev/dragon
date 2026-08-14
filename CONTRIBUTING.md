# Contributing to Dragon

Thank you for considering contributing to the Dragon programming language! We welcome contributions from the community.

## How to Contribute

There are many ways to contribute to Dragon:

1. **Report bugs** - Open an issue describing the problem
2. **Suggest features** - Open an issue with your idea
3. **Improve documentation** - Help make our docs clearer and more complete
4. **Fix bugs** - Look for issues labeled "good first issue" or "help wanted"
5. **Implement features** - Work on features from our roadmap
6. **Improve performance** - Help optimize the compiler, VM, or standard library
7. **Add tests** - Help increase our test coverage
8. **Review code** - Help review pull requests

## Getting Started

1. Fork the repository on GitHub
2. Clone your fork locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/dragon.git
   cd dragon
   ```
3. Set up the upstream remote:
   ```bash
   git remote add upstream https://github.com/aaveshdev/dragon
   git fetch upstream
   ```
4. Create a branch for your work:
   ```bash
   git checkout -b feature/your-feature-name
   ```
5. Make your changes
6. Test your changes (see [Testing](#testing))
7. Commit your changes:
   ```bash
   git commit -m "Describe your changes clearly"
   ```
8. Push to your fork:
   ```bash
   git push origin feature/your-feature-name
   ```
9. Open a Pull Request against the upstream main branch

## Development Setup

See [contributor-docs/development-environment.md](./contributor-docs/development-environment.md) for detailed setup instructions.

## Code Style

We follow standard Rust conventions:

- Use `rustfmt` to format your code: `cargo fmt`
- Use `clippy` to catch potential issues: `cargo clippy`
- Write clear, descriptive commit messages
- Comment non-obvious code
- Keep functions focused and reasonably sized
- Follow Rust naming conventions (snake_case for variables/functions, PascalCase for types)

## Pull Request Process

1. Ensure your code passes all tests: `cargo test`
2. Ensure your code is formatted: `cargo fmt`
3. Ensure your code passes clippy: `cargo clippy -- -D warnings`
4. Update documentation if necessary
5. Describe your changes clearly in the pull request
6. Reference any related issues
7. Wait for review from maintainers
8. Address any feedback from reviewers
9. Once approved, maintainers will merge your PR

## Reporting Bugs

When reporting a bug, please include:
- Dragon version (from `dragon --version`)
- Operating system and architecture
- Steps to reproduce the issue
- Expected behavior vs actual behavior
- Any error messages or output
- Minimal code example that reproduces the issue (if applicable)

## Suggesting Features

When suggesting a feature, please consider:
- How the feature aligns with Dragon's goals
- Whether the feature could be implemented as a library instead
- Backward compatibility concerns
- Implementation complexity
- Whether you're willing to help implement it

## Language Changes

Changes to the Dragon language itself follow a specific process:
1. Discussion in an issue
2. Formal proposal using the template in `proposals/PROPOSAL_TEMPLATE.md`
3. Review by maintainers
4. Decision (accept, implement, reject, defer)
5. Implementation in a feature branch
6. Testing and documentation
7. Merge and release

See [governance/LANGUAGE_CHANGE_PROCESS.md](./governance/LANGUAGE_CHANGE_PROCESS.md) for details.

## Getting Help

If you need help:
- Check the existing documentation
- Look at similar code in the repository
- Ask in the project's communication channels
- Look for "good first issue" labels on GitHub

Thank you for contributing to Dragon!