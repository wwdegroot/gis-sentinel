# Findings
This document reports user findings.

## Bugs

- Deleting or Disabling service point doesnt remove it from active alerst. The disabled or deleted service point keeps being shown as an active incident. Disabling and deleting should remove it from the active incident list.
- Right now the history tab responds with a json with a list of probes. The content of the probe response containt unneccesary data like raw_response_snippet. Provide a different response object that keeps the json point as small as possible with just the minimal data needed for the graph.
- export to csv/json endpoints do not work yet.
