# Package boundaries

The library owns protocol decoding and interpretation of explicit observations.
Small executables supply connection configuration and report results. Private
deployment configuration belongs to the operator, outside this repository.

Start with the Frigate adapter. Keep additional adapters independent until shared
behavior emerges; do not create a generic device abstraction in advance. Dashboard
presentation and Home Assistant configuration remain in their existing systems.

The current Frigate adapter makes two sequential requests. Its timestamp is the
start of collection, not an atomic snapshot of both endpoints. Recording metadata
establishes a reported segment interval, not verified playback or continuous
coverage. A missing camera in stats and an empty recording window are observations,
not automatic diagnoses. The checker never repairs the system it observes.

Future analysis work should reuse typed observations and add explicit resource
budgets, durable job state, and tests for uncertain outcomes. AI requests must
reserve budget before dispatch; a timeout cannot be assumed to mean no charge.
Semantic search readiness and actual retrieval quality need distinct evidence.
These capabilities are not implemented in this first adapter.

Targeted fixes in upstream applications should remain separate PRs in those
projects. This crate is for maintained integrations and analysis we own, rather
than a replacement for Frigate, Home Assistant, or their native configuration.
