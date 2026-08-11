## Overview

This document describes the governance of the Spin project.

The Spin project consists of several codebases with different release cycles. These codebases include:

- Core Spin:
    - [Spin](https://github.com/spinframework/spin)
- SDKs:
    - [Spin Python SDK](https://github.com/spinframework/spin-python-sdk)
    - [Spin JavaScript SDK](https://github.com/spinframework/spin-js-sdk)
    - [Spin Rust SDK](https://github.com/spinframework/spin-rust-sdk)
    - [Spin .NET SDK](https://github.com/spinframework/spin-dotnet-sdk)
    - [Spin Go SDK](https://github.com/spinframework/spin-go-sdk)
    - [JS Wasi Ext](https://github.com/spinframework/js-wasi-ext)
- Plugins:
    - [Spin Plugins Index Repository](https://github.com/spinframework/spin-plugins)
    - [Fermyon Platform Plugin](https://github.com/fermyon/platform-plugin)
    - [Spin Test Plugin](https://github.com/spinframework/spin-test)
    - [Spin Deps Plugin](https://github.com/spinframework/spin-deps-plugin)
    - [Spin OTel Plugin](https://github.com/spinframework/otel-plugin)
- Triggers:
    - [Spin Trigger Plugins](https://github.com/spinframework/spin-trigger-plugins)
- Other
    - [Spin Fileserver](https://github.com/spinframework/spin-fileserver)
    - [Spin Redirect](https://github.com/spinframework/spin-redirect)

Each repository is subject to the same overall governance model, but has different teams of people (“maintainers”) with permissions and access to the repository. This is meant to increase diversity of maintainers in the Spin project and also increases the velocity of code changes. Major changes and features to the project including additions to the repository list above are to be proposed through the [Spin Improvement Proposal](docs/content/sips/index.md) process.

## Spin Project Roles

The Spin project is built and maintained by a community of contributors, maintainers, and core maintainers. These roles represent increasing levels of responsibility, ownership and trust within the project.

### Contributors

**Contributors** are community members who actively improve the project through code, documentation, issue reports, discussions, testing, community support or other meaningful contributions. 

Contributors are expected to follow project contribution guidelines, abide by the Code of Conduct, collaborate constructively with the community, and help improve the quality of the project.

Contributors do not need to hold a formal project role to participate in the direction of Spin.

### Maintainers

Maintainers are trusted contributors who have demonstrated sustained involvement in the project and take responsibility beyond their individual contributions.

Maintainers may:
- Triage and manage issues
- Review pull requests and provide technical feedback
- Help maintain and improve code quality
- Guide and mentor contributors
- Participate in technical and project discussions
- Take ownership of particular areas or components of the project
- Help identify and prioritize work within the project

Maintainers are expected to demonstrate sound technical and community judgement and a strong understanding of the project's standards and direction.

The Maintainer role does not necessarily grant merge, release or administrative access. Maintainer is a meaningful project role in its own right and is not considered a probationary Core Maintainer position.

### Core Maintainers

**Core Maintainers** are Maintainers who have demonstrated sustained ownership of the project, sound technical and community judgement, and the ability to independently review and make decisions about changes entering the project.

Core Maintainers are trusted with merge access and share responsibility for the project's technical direction, quality releases, and long-term health. Core Maintainers also serve as stewards of the project community. Their responsibilities include:
- Reviewing and merging changes to the project
- Helping establish and maintain the project's technical direction
- Maintaining the quality, security, and stability of the project
- Participating in releases and other project operations
- Upholding the Code of Conduct and helping resolve community and contributor concerns
- Fostering a welcoming, inclusive, and productive project community
- Helping evolve project governance as the project and community grow
- Identifying when project processes, roles, decision-making structures or contributor pathways need to change
- Participating in decisions regarding the long-term health and sustainability of the project

Core Maintainers are expected to exercise their authority in the interests of the Spin project and its community.

### Repository Maintainer Teams

Each repository within the Spin project may have its own Maintainers and Core Maintainers appropriate to the work being produced by that repository. Project roles are not limited to software developers.

Maintainers, Core Maintainers and Emeritus Maintainers for each project should be outlined in the MAINTAINERS.md file in the corresponding GitHub repository.

### Becoming a Maintainer

New maintainers MUST be nominated by an existing Maintainer or Core Maintainer.

Maintainers and Core Maintainers for the relevant repository should consider whether the nominee has demonstrated sustained and constructive participation in the project and is preared to take responsibility beyond their individual contributions. Once agreement has been reached, the Maintainer may be added via a pull request to the relevant MAINTAINERS.md file.

### Becoming a Core Maintainer

Core Maintainer status represents a higher level of project trust and responsibility is not granted solely based on the length or volume of an individual's contributions. 

Candidates for Core Maintainer should have demonstrated:
- Sustained ownership and involvement in the project
- Sound technical judgement
- Sound community judgement and constructive interactions with contributors
- A strong understanding of the project's architecture, standards, and direction
- The ability to independently review changes and determine whether they are appropriate to enter the project
- A willingness to take responsibility for the long-term health of the project and its community

New Core Maintainers MUST be nominated by an existing Core Maintainer. Core Maintainers for the relevant repository will discuss nomination and reach agreement in a private setting. Once a decision has been made, the new Core Maintainer may be added via a pull request to the relevant MAINTAINERS.md file and granted the appropriate repository permissions.

### Inactivity and Emeritus Status

Maintainers and Core Maintainers MUST remain active on the project. If a Maintainer or a Core Maintainer is unresponsive for more than three months, they may be moved to Emeritus Maintainer status unless the remaining Core Maintainers for the project and the Spin Governance Committee agree to extend that period.

A Maintainer and Core Maintainer may also voluntarily step down and become an Emeritus Maintainer.

A Maintainer or Core Maintainer may be removed for a Code of Conduct violation by the Spin Governance Committee using the contact information in MAINTAINERS.md.

When a project has no active Core Maintainers, the Core Maintainers of the spinframework/spin repository become responsible for the project and may identify new maintainers or archive the project.

### Spin Governance Committee

The Core Maintainers for [github.com/spinframework/spin](https://github.com/spinframework/spin) also serve as the interim Spin Governance Committee and have the following additional responsibilities:

- Maintaining the mission, vision, values, and scope of the project
- Refining this governance document
- Making project level decisions
- Making decisions about project licensing, contribution requirements, and other legal matters
- Resolving escalated project decisions when responsible project maintainers are blocked
- Managing the Spin brand
- Managing access to Spin assets such as source repositories, hosting, project calendars
- Handling code of conduct violations for any repository under the Spin project
- Deciding what sub-groups and repositories are part of the Spin project
- Overseeing the resolution and disclosure of security issues
- Managing financial decisions related to the project

In addition to the responsibilities listed above, this group is also responsible for bootstrapping a multi-stakeholder steering committee of 5-7 people to govern the project. The group is responsible for creating a steering committee Charter and shaping and executing the processes around selecting committee members. Once a steering committee is in place, the Spin Governance Committee will be dismantled and replaced with the Spin Steering Committee. The Spin Steering Committee should then establish additional governance structures as it sees fit (e.g., a Code of Conduct Committee for project moderation).

The Spin Governance Committee have to match the following criteria:

- Spin Governance Committee members MUST remain active on the project. If they are unresponsive for > 3 months, they will lose membership, unless the remaining members of the committee agree to extend the period to be greater than 3 months

The Spin Governance Committee will select a chair to set agendas, call meetings, and oversee the decision making process.

## Decision Making

The default decision making process is objection-free consensus. In other words, a decision is made when all decision makers have had time to consider the decision and do not raise any objections. Silence on any consensus decision is equivalent to non-objection. Explicit agreement may be stated at will.

Decision making scenarios MUST be promoted appropriately by the maintainer or committee member overseeing the issue. All substantial changes in any part of the Spin project including for governance related changes require a SIP.

All substantial changes to governance require a supermajority quorum on the governance committee.

In the extreme case that objection-free consensus cannot be reached after a reasonable amount of time and effort between project maintainers on a project level decision, a project maintainer can call for a [supermajority](https://en.wikipedia.org/wiki/Supermajority#Two-thirds_vote) vote from the project maintainers for a repo on a decision. If quorum cannot be met for a decision, all members of the Spin Governance Committee are added to the relevant vote.

If a decision impacts multiple repositories or requires a coordinated effort across multiple repositories and project maintainers are unable to reach a decision on their own for the relevant projects, a maintainer can call for a decision from the Spin Governance Committee.

In the extreme case that objection-free consensus cannot be reached after a reasonable amount of time and effort on the governance committee, a committee member can call for a supermajority vote form the committee. If another member seconds the vote, the vote MUST take place.

## Glossary

- Objection-free consensus: A decision is made when all decision makers have had time to consider the decision and do not raise any objections. Silence on any consensus decision is equivalent to non-objection. Explicit agreement may be stated at will.
- Supermajority: Two-thirds majority where at least two-thirds of the group is in favor of the decision being made. More context [here](https://en.wikipedia.org/wiki/Supermajority#Two-thirds_vote).
- Emeritus Maintainer: These are project maintainers that are no longer active. We model this after the Helm Emeritus Maintainer role. More context [here](http://technosophos.com/2018/01/11/introducing-helm-emeritus-core-maintainers.html).
