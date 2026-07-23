import type { ReactNode } from "react";

export interface ViewerDocument {
  id: string;
  filename: string;
  displayName: string;
  sourcePath: string;
  extractedTextPath: string | null;
  extractionStatus: string;
}

export interface DocumentViewerAccess {
  allowOriginal(): Promise<void>;
  readText(kind: "derived" | "original"): Promise<string>;
  readBytes(): Promise<number[]>;
  openOriginal(): Promise<void>;
  revealOriginal(): Promise<void>;
}

export interface OriginalRenderContext {
  document: ViewerDocument;
  assetUrl: string | null;
  assetError: string | null;
  access: DocumentViewerAccess;
}

export type OriginalRenderer = (context: OriginalRenderContext) => ReactNode;
