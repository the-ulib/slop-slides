import { X } from "lucide-react";
import { useEffect } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";

import { RightSidebar } from "./components/RightSidebar";
import { CodeView } from "./components/CodeView";
import { Home } from "./components/Home";
import { Presenter } from "./components/Presenter";
import { SlideImageExport } from "./components/SlideImageExport";
import { SlideRail } from "./components/SlideRail";
import { Stage } from "./components/Stage";
import { TopBar } from "./components/TopBar";
import { useApp } from "./store";

export function App() {
  const deck = useApp((s) => s.deck);
  const presenting = useApp((s) => s.presenting);

  return (
    <>
      {deck ? <Editor /> : <Home />}
      {presenting && <Presenter />}
      <SlideImageExport />
      <ErrorToast />
    </>
  );
}

function Editor() {
  const view = useApp((s) => s.view);
  const chatOpen = useApp((s) => s.chatOpen);
  // Re-lint whenever deck.html changes on disk (agent, HTML view, slide operations).
  const deckVersion = useApp((s) =>
    s.deck ? [s.deck.id, s.deck.shellHash, ...s.deck.slides.map((x) => `${x.id}:${x.hash}`)].join("|") : "",
  );
  useEffect(() => {
    void useApp.getState().refreshLint();
  }, [deckVersion]);
  return (
    <div className="flex h-full flex-col">
      <TopBar />
      <Group orientation="horizontal" className="min-h-0 flex-1">
        <Panel id="rail" defaultSize={220} minSize={150} maxSize={360}>
          <SlideRail />
        </Panel>
        <ResizeHandle />
        <Panel id="stage" minSize={360}>
          {/* The HTML view stays mounted so unsaved edits survive switching to the slides. */}
          <CodeView active={view === "code"} />
          {view === "slides" && <Stage />}
        </Panel>
        {chatOpen && (
          <>
            <ResizeHandle />
            <Panel id="chat" defaultSize={380} minSize={300} maxSize={640}>
              <RightSidebar />
            </Panel>
          </>
        )}
      </Group>
    </div>
  );
}

function ResizeHandle() {
  return (
    <Separator className="relative w-px bg-border outline-none transition-colors after:absolute after:inset-y-0 after:-left-1 after:-right-1 data-[separator=active]:bg-primary data-[separator=hover]:bg-primary/50" />
  );
}

function ErrorToast() {
  const error = useApp((s) => s.error);
  if (!error) return null;
  return (
    <div className="fixed bottom-4 left-1/2 z-[60] flex max-w-lg -translate-x-1/2 items-start gap-2 rounded-lg border border-destructive/30 bg-card px-3 py-2 text-sm shadow-lg">
      <span className="selectable flex-1 text-destructive">{error}</span>
      <button
        type="button"
        onClick={() => useApp.getState().setError(null)}
        className="text-muted-foreground hover:text-foreground"
      >
        <X className="size-4" />
      </button>
    </div>
  );
}
