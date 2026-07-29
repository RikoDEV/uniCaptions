import { listen } from "@tauri-apps/api/event";

export interface CaptionEvent {
  text: string;
  translatedText: string | null;
  isFinal: boolean;
  timestamp: number;
}

const CAPTION_UPDATE_EVENT = "caption-update";

export function onCaptionUpdate(cb: (event: CaptionEvent) => void) {
  return listen<CaptionEvent>(CAPTION_UPDATE_EVENT, (event) => cb(event.payload));
}
