# Yap

Local MP3 transcription with `ZoOtMcNoOt/yap-cohere-transcribe-03-2026`, mirrored from `CohereLabs/cohere-transcribe-03-2026`.
Transcripts save beside the source when allowed, otherwise to `%LOCALAPPDATA%\Yap\Transcripts`.

```powershell
cd C:\dev\cohere-transcribe-local
uv venv --python 3.12
.\.venv\Scripts\python -m pip install --force-reinstall "torch==2.11.0+cu128" --index-url https://download.pytorch.org/whl/cu128 --trusted-host download.pytorch.org --trusted-host download-r2.pytorch.org
.\.venv\Scripts\python -m pip install -r requirements.txt
.\.venv\Scripts\python .\transcribe.py "C:\path\to\audio.mp3"
```

Desktop UI:

```powershell
cd C:\dev\cohere-transcribe-local\desktop
npm run tauri dev
```

Yap uses the public mirror first, then falls back to the upstream CohereLabs repo. If the fallback is denied, accept the upstream model terms on Hugging Face and log in:

```powershell
.\.venv\Scripts\hf auth login
```

To override the source:

```powershell
$env:YAP_MODEL_ID="your-name/your-model-mirror"
```
