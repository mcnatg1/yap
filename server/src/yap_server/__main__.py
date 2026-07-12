import logging
import signal

from yap_server.api.app import serve
from yap_server.config import ServerSettings


def _raise_keyboard_interrupt(signum: int, frame: object) -> None:
    del signum, frame
    raise KeyboardInterrupt


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    if hasattr(signal, "SIGBREAK"):
        signal.signal(signal.SIGBREAK, _raise_keyboard_interrupt)
    try:
        settings = ServerSettings.from_env()
    except ValueError as error:
        raise SystemExit(str(error)) from error

    try:
        serve(settings)
    except KeyboardInterrupt:
        return


if __name__ == "__main__":
    main()
