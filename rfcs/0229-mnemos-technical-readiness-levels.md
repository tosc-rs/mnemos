# RFC - MnemOS Technical Readiness Level

MnemOS will for a long time have a wide variety of capabilities in a wide variety of different levels of "done".

## Overview

In order to better signal how far along an implementation or idea is, this RFC proposes establishing the following levels as somewhat of a standard:

* A: Production Level - paid support available
* B: Ready for general usage
* C: In wide usage
* D: In Implementation
* E: In Design/RFC/Experiment
* F: Early proposal
* G-W: Reserved
* X: Deprecated
* Y: Retired
* Z: Declined

These levels apply to a "component", which could be a crate, or a feature/module of a crate.

Although these levels exist to set and manage expectations, no contractual guarantee of support or stability exists
based on these ratings alone - if you need guarantees, please contact a MnemOS team member to establish a contractual
agreement. All other support is provided on a best effort and volunteer basis.

## Level Details

### A: Production Level - paid support available

This is the highest level of readiness, and is mature enough that "off the shelf" support for these components
is available from a member of the MnemOS team, including Long Term Support.

Developers should feel very comfortable using MTRL "A" components for any projects.

Components with an "A" MTRL:

* Strictly follow semver and have a version >= 1.0.0
* Have extensive documentation
* Have extensive testing
* Are widely used across the MnemOS project and platforms

### B: Ready for General Usage

This level is considered very mature, but may not be stable enough for active or long term support. These
components are likely mature enough that support or improvements could be made on a contract basis.

Developers should feel generally comfortable using MTRL "B" components for any projects. It is likely possible
to sponsor efforts to move a component from MTRL "B" to MTRL "A".

Components with a "B" MTRL:

* Strictly follow semver and have a version >= 1.0.0
* Have basic documentation
* Have some testing
* Are used across the MnemOS project and platforms

### C: In Wide Usage

This level is considered general mature, though it may have "rough edges" or may not be mature when used outside
of the way it is currently employed.

This level is intended for components that are already used and seem to be working fine, but have not yet been
thoroughly documented or tested for robustness. They are likely in use in one or more places in the MnemOS
repository, and likely have been mostly stable for a reasonable amount of time.

Developers should expect that components with a "C" MTRL are likely to work, but may require additional care
and verification, particularly when integrating into a new application or platform. It is likely possible to
sponsor efforts to move a component from MTRL "C" to "B" or "A".

Components with a "C" MTRL:

* May not strictly follow semver, and may have a version < 1.0.0
* Have some documentation
* Have some testing
* May have some original design documentation
* Has numerous examples of usage in public code

### D: In implementation

This level is for things that should generally work, but may still have rough edges, and may not be suitable
for "production" usage. They may not be fully feature complete, and may not be "generalizable" to other
platforms or applications.

Developers should take care before using these components in new designs, unless they intend to assist in the
development or verification of these features.

It may be possible to sponsor the development of MTRL "D" components to a more mature level, if there are not
still open design questions.

Components with a "D" MTRL:

* May not strictly follow semver, and may have a version < 1.0.0
* May not have documentation
* May not have testing
* May have some original design documentation
* May be used in one or more places in the MnemOS project

### E: In Design/RFC/Experiment

This level is for components that are still being actively designed and implemented.

Developers should not generally use these components unless they are assisting with the design or implementation
of the component.

It may or may not be possible to sponsor work to complete MTRL "E" components.

Components with an "E" MTRL have no quality requirements.

### F: Early Proposal

This level is for speculative and imagined proposals. MTRL "F" components likely do not exist, though they may
be used as an "aspirational" goal or expressed future intent.

It may or may not be possible to sponsor work to complete MTRL "F" components.

Components with an "F" MTRL have no quality requirements.

### X: Deprecated

This level is for components that have been marked as deprecated, but have not yet been removed. Components
marked as Deprecated are not recommended for new designs.

It may be possible to sponsor work to continue long term support of deprecated components, depending on the reason
for deprecation, and MTRL prior to being moved to "X".

Components with an "X" MTRL:

* Should have a written explanation of the reason for deprecation
* May have a suggested upgrade/replacement path

### Y: Retired

This level is the "terminal state" for components marked as Deprecated. Retired components have been removed from the
MnemOS project, and documentation only exists for historical reasons. Retired components are not recommended for any
usage.

It is not likely to sponsor work to continue long term support for retired components.

### Z: Declined

This level is the "terminal state" for components that never reached MTRL levels A, B, or C, and were abandonded.
Components marked as "declined" were likely rejected due to unaddressable design issues, or some other disqualifying
reason.

It is not likely to sponsor work towards implementing declined components, without significant redesign and/or
reimplementation.

## Open questions:

* How do we handle "is this component good on this platform"?
    * Just take the "highest level" from any platform?
    * Make a matrix of components and platforms?
* Is it worth doing this now, when everything is D/E, with maybe C for the next long time?
* How do we handle "this component doesn't even make sense on this platform"?
* Should we have something else for tracking platform readiness (e.g. tier 1-3, like rustc?) compared to components/features? Is platform support just a "component"?
