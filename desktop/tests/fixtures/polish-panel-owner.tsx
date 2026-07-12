import { useRef, useState } from "react";
import { createRoot } from "react-dom/client";

import { PolishPanel } from "@/components/panels/polish-panel";
import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";

const item: RecordingJobView = {
  id: 1,
  intent: "recording",
  name: "meeting.wav",
  output: "C:/meeting.txt",
  path: "C:/meeting.wav",
  pipeline: createInitialPipelineState(),
  route: "serverBatch",
  status: "complete",
};

function PolishOwnerFixture() {
  const [polishedText, setPolishedText] = useState<string>();
  const [saveCalls, setSaveCalls] = useState(0);
  const pendingSave = useRef(new Promise<string>(() => undefined));

  return (
    <>
      <button onClick={() => setPolishedText("Externally changed draft")} type="button">
        Mutate draft
      </button>
      <output aria-label="Save calls">{saveCalls}</output>
      <PolishPanel
        item={item}
        onLoadText={async () => "Original transcript"}
        onPolished={(_outputPath, text) => setPolishedText(text)}
        onSave={async () => {
          setSaveCalls((count) => count + 1);
          return pendingSave.current;
        }}
        originalText="Original transcript"
        polishedText={polishedText}
      />
    </>
  );
}

createRoot(document.getElementById("root")!).render(<PolishOwnerFixture />);
