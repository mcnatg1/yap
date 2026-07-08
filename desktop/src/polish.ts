import { polishNumGpuLayers } from "@/settings";

const defaultPolishModel = "gemma4:e2b-it-q4_K_M";

export type PolishTone = "light" | "clean" | "notes";

export const polishToneLabels: Record<PolishTone, string> = {
  light: "Light",
  clean: "Clean",
  notes: "Notes",
};

export const polishToneHints: Record<PolishTone, string> = {
  light: "Trim fillers and stutters; keep your voice.",
  clean: "Smooth into clear, readable prose.",
  notes: "Condense into concise meeting-style notes.",
};

const polishInstructions: Record<PolishTone, string> = {
  light:
    "Lightly clean the transcript. Remove filler words, obvious repeated stutters, and tiny false starts. Keep the speaker's voice and meaning. Do not summarize.",
  clean:
    "Clean the transcript into clear spoken prose. Preserve meaning, fix repeated false starts, and make it easier to read without making it sound artificial.",
  notes:
    "Turn the transcript into concise meeting-style notes. Preserve the important details and keep the wording grounded in the transcript.",
};

type OllamaChatResponse = {
  message?: {
    content?: string;
    thinking?: string;
  };
  eval_count?: number;
  eval_duration?: number;
  total_duration?: number;
};

export type PolishResult = {
  text: string;
  model: string;
  tokensPerSecond?: number;
  totalSeconds?: number;
};

export async function polishTranscript({
  model = defaultPolishModel,
  text,
  tone,
}: {
  model?: string;
  text: string;
  tone: PolishTone;
}): Promise<PolishResult> {
  const source = text.trim();
  if (!source) throw new Error("The selected transcript is empty.");

  const numGpu = await polishNumGpuLayers().catch(() => 0);

  const response = await fetch("http://127.0.0.1:11434/api/chat", {
    body: JSON.stringify({
      model,
      messages: [
        {
          role: "system",
          content:
            "You are a private local transcript cleanup engine. Return only the cleaned text. Never return an empty response.",
        },
        {
          role: "user",
          content: `${polishInstructions[tone]}\n\nTranscript:\n${source}`,
        },
      ],
      stream: false,
      think: false,
      keep_alive: "10m",
      options: {
        num_gpu: numGpu,
        temperature: tone === "light" ? 0.2 : 0.3,
        num_predict: tone === "notes" ? 320 : 220,
      },
    }),
    headers: { "Content-Type": "application/json" },
    method: "POST",
  }).catch((error) => {
    throw new Error(
      `Ollama is not available. Start Ollama, then run: ollama pull ${model}. ${String(error)}`,
    );
  });

  if (!response.ok) {
    const detail = await response.text().catch(() => "");
    throw new Error(friendlyOllamaError(response.status, detail, model));
  }

  const result = (await response.json()) as OllamaChatResponse;
  const output = result.message?.content?.trim();
  if (!output) {
    throw new Error("Gemma did not return polished text. Yap will retry with thinking disabled on the next run.");
  }

  return {
    model,
    text: output,
    tokensPerSecond: tokensPerSecond(result),
    totalSeconds: secondsFromNanos(result.total_duration),
  };
}

function tokensPerSecond(result: OllamaChatResponse) {
  if (!result.eval_count || !result.eval_duration) return undefined;

  const seconds = secondsFromNanos(result.eval_duration);
  if (!seconds) return undefined;

  return Math.round((result.eval_count / seconds) * 10) / 10;
}

function secondsFromNanos(value?: number) {
  if (!value) return undefined;
  return Math.round((value / 1_000_000_000) * 10) / 10;
}

function friendlyOllamaError(status: number, detail: string, model: string) {
  const lower = detail.toLowerCase();
  if (status === 404 || lower.includes("not found")) {
    return `Polish model missing. Run: ollama pull ${model}`;
  }

  return detail.trim() || "Ollama is not available. Start Ollama, then try Polish again.";
}
