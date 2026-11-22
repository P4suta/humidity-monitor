---
date: 2026-11-22
---

# Draw shape-preserving humidity trends

## Status

Proposed.

## Context

Minute markers and an expanded table obscure the trend on a phone.
The [bounded chart](bound-humidity-chart-detail.md) already preserves gaps and observed extremes.

## Decision

Divide the displayed time into at most 48 intervals.
Draw a monotone cubic curve through the endpoints and extremes of each continuous run, ordered by reception time.
Mark isolated observations and show twelve exact readings when the reader opens the history.
Render the chart and history navigation on the server.

## Alternatives

A moving average hides brief excursions.
Straight segments preserve the observations but leave sharp corners at changes in slope.

## Consequences

The curve represents the shape between selected readings.
The [D3 monotone curve](https://d3js.org/d3-shape/curve#curveMonotoneX) preserves the range between neighbors.
The [data table](https://www.w3.org/WAI/tutorials/images/complex/) gives exact minute readings.

## Confirmation

Check that curves pass through observed extremes and stop at missing minutes.
Measure the annual page on ARM64.
Revisit the interval count if the chart gains zoom controls.
