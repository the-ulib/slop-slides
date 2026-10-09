import { PanelRightClose } from "lucide-react";
import { useApp } from "../store";
import { ChatPanel } from "./ChatPanel";
import { NarrationPanel } from "./NarrationPanel";

export function RightSidebar() {
  const tab = useApp((s) => s.sidebarTab);
  return <div className="flex h-full min-h-0 flex-col bg-background">
    <div className="flex h-10 shrink-0 items-center gap-1 border-b border-border px-2">
      <div role="tablist" aria-label="Right sidebar" className="flex flex-1 gap-1">
        {(["chat", "narration"] as const).map((id) => <button key={id} id={`tab-${id}`} role="tab" aria-selected={tab === id} aria-controls={`panel-${id}`} onClick={() => useApp.getState().setSidebarTab(id)} className={`rounded-md px-3 py-1.5 text-xs ${tab === id ? "bg-accent text-foreground" : "text-muted-foreground hover:text-foreground"}`}>{id === "chat" ? "Chat" : "Narration"}</button>)}
      </div>
      <button aria-label="Hide sidebar" title="Hide sidebar" className="rounded p-1 text-muted-foreground hover:bg-accent" onClick={() => useApp.getState().setChatOpen(false)}><PanelRightClose className="size-3.5" /></button>
    </div>
    {/* Keep both mounted: composer drafts, attachments and edits survive tab switches. */}
    <div id="panel-chat" role="tabpanel" aria-labelledby="tab-chat" hidden={tab !== "chat"} className="min-h-0 flex-1"><ChatPanel embedded /></div>
    <div id="panel-narration" role="tabpanel" aria-labelledby="tab-narration" hidden={tab !== "narration"} className="min-h-0 flex-1"><NarrationPanel /></div>
  </div>;
}
