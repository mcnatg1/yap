import os
from pathlib import Path
from tempfile import TemporaryDirectory

from transcribe import unique_path, write_transcript


def main() -> None:
    with TemporaryDirectory() as tmp:
        root = Path(tmp)
        first = root / "sample.txt"
        first.write_text("existing\n", encoding="utf-8")
        assert unique_path(first).name == "sample-2.txt"

        output = write_transcript(root / "audio.wav", "hello", root / "out")
        assert output == root / "out" / "audio.txt"
        assert output.read_text(encoding="utf-8") == "hello\n"

        os.environ["YAP_TRANSCRIPTS_DIR"] = str(root / "fallback")
        blocked = root / "blocked"
        blocked.mkdir()
        (blocked / "rode.txt").mkdir()
        fallback = write_transcript(blocked / "rode.wav", "local", None)
        assert fallback == root / "fallback" / "rode.txt"
        assert fallback.read_text(encoding="utf-8") == "local\n"


if __name__ == "__main__":
    main()
