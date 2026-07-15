import logging
import signal

from yap_server.api.app import serve
from yap_server.config import ServerSettings
from yap_server.jobs.runtime import (
    BatchRuntime,
    build_batch_runtime,
    ensure_development_batch_bind,
)


def _raise_keyboard_interrupt(signum: int, frame: object) -> None:
    del signum, frame
    raise KeyboardInterrupt


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    signal.signal(signal.SIGTERM, _raise_keyboard_interrupt)
    if hasattr(signal, "SIGBREAK"):
        signal.signal(signal.SIGBREAK, _raise_keyboard_interrupt)
    runtime: BatchRuntime | None = None
    try:
        settings = ServerSettings.from_env()
        runtime = build_batch_runtime()
        if runtime is not None:
            ensure_development_batch_bind(settings.host)
    except ValueError as error:
        if runtime is not None:
            runtime.close()
        raise SystemExit(str(error)) from None
    except (OSError, RuntimeError):
        if runtime is not None:
            runtime.close()
        raise SystemExit("Yap private server startup failed.") from None

    try:
        serve(
            settings,
            job_service=runtime.service if runtime is not None else None,
        )
    except KeyboardInterrupt:
        return
    except OSError:
        raise SystemExit("Yap private server runtime became unavailable.") from None
    finally:
        if runtime is not None:
            runtime.close()


if __name__ == "__main__":
    main()
