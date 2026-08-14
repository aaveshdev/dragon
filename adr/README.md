# Architectural Decision Records (ADR)

This directory contains architectural decisions made for the Dragon programming language project.

## What is an ADR?

An Architectural Decision Record (ADR) is a document that captures an important architectural decision made along with its context and consequences.

## ADR Format

Each ADR follows a consistent format:

### Title
A short, descriptive title that includes the ADR number (e.g., "ADR 1: Use SQLite for Local Storage").

### Status
Proposed | Accepted | Superseded

### Context
The problem or situation that led to this decision.

### Decision
The chosen solution or approach.

### Consequences
The implications of this decision, including:
- Positive consequences (benefits)
- Negative consequences (drawbacks or liabilities)

### Related Documents
Links to other ADRs or documents that are related.

## When to Create an ADR

Create an ADR when making decisions about:
- Major architectural components
- Significant design patterns
- Technology or framework selections
- Major changes to the build system or tooling
- Decisions that affect multiple teams or have long-term implications

## When NOT to Create an ADR

Do not create ADRs for:
- Minor code changes
- Bug fixes
- Routine refactoring
- Decisions that only affect a small, isolated component

## Related Files
- `adr/ADR_TEMPLATE.md` - Template for creating new ADRs