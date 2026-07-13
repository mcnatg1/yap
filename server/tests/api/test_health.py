import unittest

from yap_server.api import health


class HealthTests(unittest.TestCase):
    def test_health_matches_the_frozen_phase_3_contract(self) -> None:
        self.assertEqual(
            health(),
            {
                "service": "yap-server",
                "status": "ok",
                "apiVersion": "1",
                "auth": "not_configured",
                "capabilities": {
                    "batchJobs": False,
                    "liveStreaming": False,
                    "jobStatus": False,
                },
            },
        )


if __name__ == "__main__":
    unittest.main()
