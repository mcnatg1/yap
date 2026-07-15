import os
import unittest
from unittest.mock import patch

from yap_server.config import ServerSettings


class ServerSettingsTests(unittest.TestCase):
    def test_environment_defaults_to_the_loopback_service_address(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(
                ServerSettings.from_env(),
                ServerSettings(host="127.0.0.1", port=18765),
            )

    def test_environment_reads_an_explicit_loopback_host_and_port(self) -> None:
        with patch.dict(
            os.environ,
            {"YAP_SERVER_HOST": "::1", "YAP_SERVER_PORT": "28765"},
            clear=True,
        ):
            self.assertEqual(
                ServerSettings.from_env(),
                ServerSettings(host="::1", port=28765),
            )

    def test_private_bind_requires_the_exact_opt_in(self) -> None:
        for allow_value in (None, "0", "true"):
            environment = {"YAP_SERVER_HOST": "192.168.50.1"}
            if allow_value is not None:
                environment["YAP_SERVER_ALLOW_PRIVATE_BIND"] = allow_value
            with self.subTest(allow_value=allow_value):
                with patch.dict(os.environ, environment, clear=True):
                    with self.assertRaisesRegex(
                        ValueError, "YAP_SERVER_ALLOW_PRIVATE_BIND=1"
                    ):
                        ServerSettings.from_env()
    def test_private_bind_is_allowed_after_explicit_opt_in(self) -> None:
        with patch.dict(
            os.environ,
            {
                "YAP_SERVER_HOST": "192.168.50.1",
                "YAP_SERVER_PORT": "18766",
                "YAP_SERVER_ALLOW_PRIVATE_BIND": "1",
            },
            clear=True,
        ):
            self.assertEqual(
                ServerSettings.from_env(),
                ServerSettings(host="192.168.50.1", port=18766),
            )

    def test_wildcard_bind_is_rejected_without_opt_in(self) -> None:
        with patch.dict(
            os.environ,
            {"YAP_SERVER_HOST": "0.0.0.0"},
            clear=True,
        ):
            with self.assertRaisesRegex(
                ValueError, "YAP_SERVER_ALLOW_PRIVATE_BIND=1"
            ):
                ServerSettings.from_env()

    def test_invalid_environment_port_is_rejected(self) -> None:
        for port in ("not-a-port", "-1", "65536"):
            with self.subTest(port=port):
                with patch.dict(
                    os.environ,
                    {"YAP_SERVER_PORT": port},
                    clear=True,
                ):
                    with self.assertRaisesRegex(ValueError, "YAP_SERVER_PORT"):
                        ServerSettings.from_env()
