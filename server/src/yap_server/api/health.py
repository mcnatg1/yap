from yap_server.schemas import HealthView, ServerCapabilities


_HEALTH_VIEW = HealthView(
    service="yap-server",
    status="ok",
    api_version="1",
    auth="not_configured",
    capabilities=ServerCapabilities(
        batch_jobs=False,
        live_streaming=False,
        job_status=False,
    ),
)


def health(*, batch_jobs: bool = False) -> dict[str, object]:
    if not batch_jobs:
        return _HEALTH_VIEW.to_wire()
    return HealthView(
        service="yap-server",
        status="ok",
        api_version="1",
        auth="not_configured",
        capabilities=ServerCapabilities(
            batch_jobs=True,
            live_streaming=False,
            job_status=True,
        ),
    ).to_wire()
