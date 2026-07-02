# Yap

Tauri desktop app for local/offline transcription fallback work.

```powershell
cd C:\dev\cohere-transcribe-local\desktop
npm install
npm test
npm run build
npm run tauri dev
```

The old Python/Cohere runner was removed from the app runtime. Server/DGX batch transcription belongs in the server connector work, not this local fallback branch.
