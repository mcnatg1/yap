from __future__ import annotations

import argparse
import os
from pathlib import Path


MODEL_ID = "CohereLabs/cohere-transcribe-03-2026"


def load_model():
    from transformers import AutoProcessor, CohereAsrForConditionalGeneration

    processor = AutoProcessor.from_pretrained(MODEL_ID)
    model = CohereAsrForConditionalGeneration.from_pretrained(
        MODEL_ID,
        device_map="auto",
        torch_dtype="auto",
    )
    return processor, model


def transcribe(path: Path, language: str, punctuation: bool, processor, model) -> str:
    from transformers.audio_utils import load_audio

    audio = load_audio(str(path), sampling_rate=16000)
    inputs = processor(
        audio=audio,
        sampling_rate=16000,
        return_tensors="pt",
        language=language,
        punctuation=punctuation,
    )
    audio_chunk_index = inputs.get("audio_chunk_index")
    inputs.to(model.device, dtype=model.dtype)

    outputs = model.generate(**inputs, max_new_tokens=256)
    text = processor.decode(
        outputs,
        skip_special_tokens=True,
        audio_chunk_index=audio_chunk_index,
        language=language,
    )
    return text[0] if isinstance(text, list) else text


def local_transcripts_dir() -> Path:
    override = os.environ.get("YAP_TRANSCRIPTS_DIR")
    if override:
        return Path(override)

    local_app_data = os.environ.get("LOCALAPPDATA")
    if local_app_data:
        return Path(local_app_data) / "Yap" / "Transcripts"

    return Path.home() / ".yap" / "transcripts"


def unique_path(path: Path) -> Path:
    if not path.exists():
        return path

    for index in range(2, 1000):
        candidate = path.with_name(f"{path.stem}-{index}{path.suffix}")
        if not candidate.exists():
            return candidate

    raise RuntimeError(f"Could not find an unused output name for {path}")


def write_transcript(audio_path: Path, text: str, out_dir: Path | None) -> Path:
    preferred = (out_dir or audio_path.parent) / f"{audio_path.stem}.txt"
    if out_dir:
        preferred.parent.mkdir(parents=True, exist_ok=True)

    try:
        preferred.write_text(text + "\n", encoding="utf-8")
        return preferred
    except OSError:
        if out_dir:
            raise

    fallback_dir = local_transcripts_dir()
    fallback_dir.mkdir(parents=True, exist_ok=True)
    fallback = unique_path(fallback_dir / f"{audio_path.stem}.txt")
    fallback.write_text(text + "\n", encoding="utf-8")
    return fallback


def main() -> None:
    parser = argparse.ArgumentParser(description="Transcribe audio locally with CohereLabs Transcribe.")
    parser.add_argument("audio", nargs="+", type=Path)
    parser.add_argument("--language", default="en")
    parser.add_argument("--no-punctuation", action="store_true")
    parser.add_argument("--out-dir", type=Path)
    args = parser.parse_args()

    out_dir = args.out_dir
    if out_dir:
        out_dir.mkdir(parents=True, exist_ok=True)

    try:
        processor, model = load_model()
    except Exception as exc:
        message = str(exc).lower()
        if "requires approval" in message or "gated" in message or "access denied" in message:
            raise SystemExit(
                "Hugging Face access denied. Accept the model terms, then run: "
                r".\.venv\Scripts\hf auth login"
            ) from exc
        raise

    for audio_path in args.audio:
        audio_path = audio_path.resolve()
        try:
            text = transcribe(audio_path, args.language, not args.no_punctuation, processor, model).strip()
        except Exception as exc:
            message = str(exc).lower()
            if "requires approval" in message or "gated" in message or "access denied" in message:
                raise SystemExit(
                    "Hugging Face access denied. Accept the model terms, then run: "
                    r".\.venv\Scripts\hf auth login"
                ) from exc
            raise
        out_path = write_transcript(audio_path, text, out_dir)
        print(out_path)


if __name__ == "__main__":
    main()
