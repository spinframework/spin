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

## Spin Project Maintainers

Project maintainers are responsible for activities surrounding the development and release cycle of a specific repository they own. Technical decisions for code resides with the project maintainers, unless there is a decision related to cross maintainer groups that cannot be resolved by those groups. Those cases can be escalated to the Spin Governance Committee (discussed below).

Project maintainers do not need to be software developers. No explicit role is placed upon them, and they can be anyone appropriate for the work being produced.

Project maintainers and emeritus maintainers for each project should be outlined in the MAINTAINERS.md file in the corresponding GitHub repository.

Changes to project maintainers use the following:

- A project maintainer may step down by emailing the mailing list. When a project maintainer steps down, they become an emeritus maintainer.
- Project maintainers MUST remain active on the project. If they are unresponsive for > 3 months, they will lose project maintainer-ship, unless the remaining project maintainers of the given project and the Spin Governance Committee agree to extend the period to be greater than 3 months.
- New maintainers MUST be nominated by existing maintainers. Maintainers are to discuss and agree in a private setting adding a new maintainer. Once a decision has been made, a maintainer may be added to the project via a pull request to the relevant MAINTAINERS.md file.
- A maintainer may be removed for a [code of conduct](CODE_OF_CONDUCT.md) violation by the Spin Governance Committee. Code of conduct violations may be submitted to any member(s) on the Spin Governance Committee by email. See email information on MAINTAINERS.md.
- When a project has no active maintainers, the maintainers of the [spinframework/spin Github repo](https://github.com/spinframework/spin) become responsible for it, and may archive the project, or find new maintainers.

## Spin Governance Committee

The project maintainers for [github.com/spinframework/spin](https://github.com/spinframework/spin) also serve as the Spin Governance Committee and have the following additional responsibilities:

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
