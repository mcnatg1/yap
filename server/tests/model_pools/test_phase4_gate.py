import json
import subprocess
import unittest

from yap_server.pools.phase4_gate import (
    inspect_container_image,
    normalized_words,
    word_error_rate,
)


class Phase4GateTests(unittest.TestCase):
    def test_image_inspection_attests_arm64_and_checked_revision(self) -> None:
        checked_head = "a" * 40

        def runner(*args: object, **kwargs: object) -> subprocess.CompletedProcess[str]:
            del args, kwargs
            return subprocess.CompletedProcess(
                args=["docker"],
                returncode=0,
                stdout=json.dumps(
                    [
                        {
                            "Id": "sha256:" + "b" * 64,
                            "Architecture": "arm64",
                            "RepoDigests": [],
                            "Config": {
                                "Labels": {
                                    "org.opencontainers.image.revision": checked_head,
                                }
                            },
                        }
                    ]
                ),
                stderr="",
            )

        image = inspect_container_image(
            "yap-asr:phase4-aaaaaaaaaaaaaaaa",
            checked_head,
            runner=runner,
        )

        self.assertEqual(image["architecture"], "arm64")
        self.assertEqual(image["revision"], checked_head)

    def test_image_inspection_rejects_a_different_revision(self) -> None:
        def runner(*args: object, **kwargs: object) -> subprocess.CompletedProcess[str]:
            del args, kwargs
            return subprocess.CompletedProcess(
                args=["docker"],
                returncode=0,
                stdout=json.dumps(
                    [
                        {
                            "Id": "sha256:" + "b" * 64,
                            "Architecture": "arm64",
                            "RepoDigests": [],
                            "Config": {
                                "Labels": {
                                    "org.opencontainers.image.revision": "c" * 40,
                                }
                            },
                        }
                    ]
                ),
                stderr="",
            )

        with self.assertRaises(RuntimeError):
            inspect_container_image(
                "yap-asr:phase4-aaaaaaaaaaaaaaaa",
                "a" * 40,
                runner=runner,
            )

    def test_wer_is_case_and_punctuation_insensitive(self) -> None:
        reference = "Well, I don't wish to see it."
        hypothesis = "well i don't wish to see it"
        self.assertEqual(word_error_rate(reference, hypothesis), 0.0)

    def test_wer_counts_insertions_deletions_and_substitutions(self) -> None:
        self.assertEqual(word_error_rate("one two three", "one four"), 2 / 3)

    def test_normalization_preserves_apostrophes(self) -> None:
        self.assertEqual(normalized_words("Don't stop."), ["don't", "stop"])


if __name__ == "__main__":
    unittest.main()
