# yap-server staging

This directory is the MVP staging area for the future `yap-server` repo.

Keep it boring until there is a real service:

- API contracts live here first, likely `openapi/`.
- Router/service code lives here once Phase 8 starts.
- Model worker code lives here only after the router contract exists.
- Host setup stays in `../infra/yap-server-node/`.
- Shared desktop/server contracts stay here until type drift proves a separate `yap-contracts` repo is worth it.

Phase 8 grows inside this shape:

```text
server/
  README.md
  openapi/
    README.md
    yap-server.v1.yaml      # later
    live-wss.md             # later
  src/
    yap_server/
      api/
      workload_router/
      pools/
      schemas/
      config/
  tests/
    README.md
    contract/
    api/
    workload_router/
```

Use `workload_router/` instead of vague `router/`. Use `schemas/` for API and message shapes. Do not add a repo `models/` directory; runtime model files belong on the server node, not in Git.

Skipped for now: Nx/Turborepo, package workspace wiring, framework/server dependencies, checked-in model weights, and fake GB300 profiles.
