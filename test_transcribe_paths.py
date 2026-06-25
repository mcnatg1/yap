import os
from pathlib import Path
from tempfile import TemporaryDirectory

from transcribe import DEFAULT_MODEL_ID, UPSTREAM_MODEL_ID, model_ids, unique_path, write_transcript


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

        os.environ.pop("YAP_MODEL_ID", None)
        os.environ.pop("YAP_MODEL_FALLBACK_ID", None)
        assert model_ids() == [DEFAULT_MODEL_ID, UPSTREAM_MODEL_ID]

        os.environ["YAP_MODEL_ID"] = "local/mirror"
        os.environ["YAP_MODEL_FALLBACK_ID"] = "local/mirror"
        assert model_ids() == ["local/mirror"]


if __name__ == "__main__":
    main()
