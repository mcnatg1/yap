# Server contracts

Server-tier contracts start here.

- `openapi.json` will describe HTTP health plus the contract-only batch upload
  and job-status boundary.
- `live-events.schema.json` will describe the contract-only live event and
  reconnect vocabulary.

Phase 3 implements only `GET /v1/health`. Upload, job handlers, live WSS,
authentication, and inference remain later phases. Keep generated clients out
until type drift becomes real.
