# Yap

Local MP3 transcription with `CohereLabs/cohere-transcribe-03-2026`.

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

If the model download is denied, accept the model terms on Hugging Face and log in:

```powershell
.\.venv\Scripts\hf auth login
```
