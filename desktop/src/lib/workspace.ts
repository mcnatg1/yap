const workspaceViews = ["home", "transcribe", "polish"] as const;

export type WorkspaceView = (typeof workspaceViews)[number];

export type RailAction = WorkspaceView | "details" | "help";

export const workspaceCopy: Record<WorkspaceView, { title: string; description: string }> = {
  home: {
    title: "Welcome back",
    description: "",
  },
  transcribe: {
    title: "Transcribe",
    description: "Add recordings to your organization's transcription queue.",
  },
  polish: {
    title: "Polish",
    description: "Clean text.",
  },
};

export function isWorkspaceView(value: unknown): value is WorkspaceView {
  return typeof value === "string" && (workspaceViews as readonly string[]).includes(value);
}
