---
date: 2026-11-22
---

# Bound humidity chart detail by time interval

## Status

Proposed.

## Context

A year with alternate minutes missing produced a 29 MiB page during an ARM64 rehearsal.
The chart spans 532 drawing units and the history table lists individual readings.
[W3C guidance for complex images](https://www.w3.org/WAI/tutorials/images/complex/)

## Decision

Divide each chart period into at most 300 intervals.
Represent each continuous run by its endpoints and humidity range.
Keep the first and last runs in each interval and the interior run when only one lies between them.
For two or more interior runs, mark their lowest and highest humidity at the observed times.
Join endpoints only across consecutive recorded minutes.

## Alternatives

Streaming every run would reduce temporary allocations in the receiver.
The browser would still download and draw every run.

## Consequences

Each interval contributes at most four chart entries.
Some interior changes are visible only in the history table.
Revisit the interval count if the chart gains zoom controls.

## Confirmation

Compare the release receiver on dense and intermittent histories.
Check gap boundaries, extrema, pagination, and the chart size limit.
