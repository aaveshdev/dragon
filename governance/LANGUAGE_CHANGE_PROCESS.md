# Dragon Language Change Process

This document describes the process for proposing, reviewing, and implementing changes to the Dragon programming language specification.

## Overview

Changes to the Dragon language go through a structured process to ensure stability, backward compatibility, and community input.

## Stages

### 1. Discussion
- Ideas are discussed in GitHub issues or community forums
- Participants explore the problem space, potential solutions, and trade-offs
- No formal proposal is required at this stage

### 2. Formal Proposal
- If the idea gains traction, a formal proposal is written using the template in `proposals/PROPOSAL_TEMPLATE.md`
- The proposal is placed in the `proposals/` directory as a draft
- Community feedback is solicited

### 3. Review
- Maintainers review the proposal for:
  - Alignment with Dragon's goals
  - Technical soundness
  - Backward compatibility
  - Implementation feasibility
- The proposal may be revised based on feedback

### 4. Decision
- Maintainers decide to:
  - **Accept**: Approved for implementation
  - **Implement**: Already implemented (if proposed after implementation)
  - **Rejected**: Not approved
  - **Deferred**: Postponed for future consideration
- The proposal is moved to the appropriate directory (`accepted/`, `implemented/`, `rejected/`, `deferred/`)

### 5. Implementation
- Accepted proposals are implemented in a feature branch
- Implementation includes:
  - Code changes (compiler, VM, standard library if needed)
  - Tests
  - Documentation updates
  - Specification updates in `spec/`

### 6. Release
- Implemented changes are included in a release following the release governance
- Release notes document the changes

## Criteria for Acceptance

Proposals are evaluated based on:
- **Problem significance**: How important is the problem being solved?
- **Solution quality**: How well does the proposal solve the problem?
- **Backward compatibility**: Does it break existing code? If so, is the breakage justified and mitigated?
- **Complexity**: What is the implementation and cognitive complexity?
- **Consistency**: Does it fit with existing language features?
- **Community support**: What is the level of community interest?

## Roles

- **Anyone**: Can participate in discussion and submit formal proposals
- **Maintainers**: Review proposals, make decisions, and oversee implementation
- **Contributors**: May implement accepted proposals

## Related Files
- `proposals/PROPOSAL_TEMPLATE.md` - Template for writing proposals
- `spec/compatibility/versioning.md` - Versioning guidelines
- `governance/GOVERNANCE.md` - Overall project governance