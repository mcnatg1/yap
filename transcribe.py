from __future__ import annotations

import argparse
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
        out_path = (out_dir or audio_path.parent) / f"{audio_path.stem}.txt"
        out_path.write_text(text + "\n", encoding="utf-8")
        print(out_path)


if __name__ == "__main__":
    main()
