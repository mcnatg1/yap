import unittest

from yap_server.api import health


class HealthTests(unittest.TestCase):
    def test_health_contract_names_yap_server(self) -> None:
        self.assertEqual(
            health(),
            {"status": "ok", "service": "yap-server", "version": "0.1.0"},
        )


if __name__ == "__main__":
    unittest.main()

