import type { CaseGraph } from "./types";

export function bytesToBase64(bytes: Uint8Array): string {
  const chunkSize = 0x8000;
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return btoa(binary);
}

export function textToBase64(text: string): string {
  return bytesToBase64(new TextEncoder().encode(text));
}

export function dataUrlBase64(dataUrl: string, expectedMime: string): string {
  const prefix = `data:${expectedMime};base64,`;
  if (!dataUrl.startsWith(prefix)) {
    throw new Error(`导出图片 MIME 必须是 ${expectedMime}`);
  }
  const payload = dataUrl.slice(prefix.length);
  if (!payload) throw new Error("导出图片内容为空");
  return payload;
}

export async function buildVisualPdfBase64(
  coverPngDataUrl: string,
  viewPngDataUrls: string[],
): Promise<string> {
  const { PDFDocument } = await import("pdf-lib");
  const document = await PDFDocument.create();
  for (const dataUrl of [coverPngDataUrl, ...viewPngDataUrls]) {
    const image = await document.embedPng(dataUrlBase64(dataUrl, "image/png"));
    const dimensions = image.scale(1);
    const page = document.addPage([dimensions.width, dimensions.height]);
    page.drawImage(image, {
      x: 0,
      y: 0,
      width: dimensions.width,
      height: dimensions.height,
    });
  }
  return bytesToBase64(await document.save());
}

function drawWrappedText(
  context: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  maxWidth: number,
  lineHeight: number,
): number {
  let line = "";
  let cursorY = y;
  for (const character of text) {
    const candidate = line + character;
    if (line && context.measureText(candidate).width > maxWidth) {
      context.fillText(line, x, cursorY);
      line = character;
      cursorY += lineHeight;
    } else {
      line = candidate;
    }
  }
  if (line) context.fillText(line, x, cursorY);
  return cursorY + lineHeight;
}

export function createVisualCoverDataUrl(graph: CaseGraph, revision: number): string {
  const canvas = document.createElement("canvas");
  canvas.width = 1240;
  canvas.height = 1754;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("当前环境无法生成 PDF 封面");

  context.fillStyle = "#f8f6f1";
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.fillStyle = "#244f7a";
  context.fillRect(96, 130, 10, 130);
  context.fillStyle = "#20252b";
  context.font = '600 54px system-ui, -apple-system, "PingFang SC", sans-serif';
  let y = drawWrappedText(context, graph.title, 142, 178, 970, 76);
  context.fillStyle = "#5d6670";
  context.font = '28px system-ui, -apple-system, "PingFang SC", sans-serif';
  context.fillText("案情可视化分析", 142, y + 28);

  y += 180;
  context.fillStyle = "#20252b";
  context.font = '600 26px system-ui, -apple-system, "PingFang SC", sans-serif';
  context.fillText("数据范围", 142, y);
  context.fillStyle = "#5d6670";
  context.font = '24px system-ui, -apple-system, "PingFang SC", sans-serif';
  y = drawWrappedText(
    context,
    `当前案件工作区，修订 ${revision}，共 ${graph.views.length} 个视图、${graph.nodes.length} 个事实节点、${graph.edges.length} 条关系。`,
    142,
    y + 50,
    970,
    42,
  );
  context.fillStyle = "#20252b";
  context.font = '600 26px system-ui, -apple-system, "PingFang SC", sans-serif';
  context.fillText("生成时间", 142, y + 52);
  context.fillStyle = "#5d6670";
  context.font = '24px system-ui, -apple-system, "PingFang SC", sans-serif';
  context.fillText(new Date().toLocaleString("zh-CN", { hour12: false }), 142, y + 102);

  context.fillStyle = "#e3e6e8";
  context.fillRect(142, 1430, 970, 2);
  context.fillStyle = "#5d6670";
  context.font = '22px system-ui, -apple-system, "PingFang SC", sans-serif';
  drawWrappedText(
    context,
    "本材料由 AI 辅助整理。争议事实、推断内容和材料依据应由律师复核后使用。",
    142,
    1490,
    970,
    38,
  );
  return canvas.toDataURL("image/png");
}
