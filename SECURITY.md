# Security reporting

This branch is preparing the first public release. A supported release version
and maintenance policy will be recorded when its release gates are complete.

Report suspected vulnerabilities through the repository host's private security
reporting facility. If that facility is unavailable, contact the maintainer using
their published Git contact details and arrange a private channel. Do not open a
public issue containing wallet secrets, exploitable deployment details or private
transaction data.

Include the affected revision, command or API operation, expected and observed
behavior, and a minimal public or synthetic reproducer. For indexing problems,
include network, chain point and schema version. For transaction problems, provide
unsigned public fixture data where possible. Never send a recovery phrase or
private signing key.

Registration proves authorship of an envelope; it does not establish chain
confirmation. Local script evaluation is a limited preflight and does not replace
the ledger's validation. Review the current support matrix and release checklist
before selecting a supported workflow.

Dependency advisory results, mitigations and unresolved release blockers belong
in the release evidence. A passing build alone is not a completed security audit.
