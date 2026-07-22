export const MAX_FEEDBACK_SCREENSHOTS = 3;
export const MAX_FEEDBACK_SCREENSHOT_BYTES = 5 * 1024 * 1024;
export const MAX_FEEDBACK_SCREENSHOT_TOTAL_BYTES = 10 * 1024 * 1024;

const SUPPORTED_IMAGE_TYPES = new Set([
  "image/png",
  "image/jpeg",
  "image/webp",
]);

export interface PreparedFeedbackScreenshot {
  filename: string;
  mimeType: string;
  dataBase64: string;
  sizeBytes: number;
}

function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error(`读取截图失败：${file.name}`));
    reader.onload = () => {
      if (typeof reader.result !== "string") {
        reject(new Error(`读取截图失败：${file.name}`));
        return;
      }
      const comma = reader.result.indexOf(",");
      if (comma < 0) {
        reject(new Error(`截图编码失败：${file.name}`));
        return;
      }
      resolve(reader.result.slice(comma + 1));
    };
    reader.readAsDataURL(file);
  });
}

export async function prepareFeedbackScreenshotFiles(
  files: Iterable<File>,
  existingCount: number,
  existingBytes = 0,
): Promise<PreparedFeedbackScreenshot[]> {
  const selected = Array.from(files);
  if (existingCount + selected.length > MAX_FEEDBACK_SCREENSHOTS) {
    throw new Error(`最多上传 ${MAX_FEEDBACK_SCREENSHOTS} 张截图`);
  }

  let totalBytes = existingBytes;
  for (const file of selected) {
    if (!SUPPORTED_IMAGE_TYPES.has(file.type.toLowerCase())) {
      throw new Error("仅支持 PNG、JPEG、WebP 截图");
    }
    if (file.size > MAX_FEEDBACK_SCREENSHOT_BYTES) {
      throw new Error(`单张截图不能超过 5 MB：${file.name}`);
    }
    totalBytes += file.size;
  }
  if (totalBytes > MAX_FEEDBACK_SCREENSHOT_TOTAL_BYTES) {
    throw new Error("截图总大小不能超过 10 MB");
  }

  return Promise.all(
    selected.map(async (file) => ({
      filename: file.name.trim() || "截图",
      mimeType: file.type.toLowerCase(),
      dataBase64: await readFileAsBase64(file),
      sizeBytes: file.size,
    })),
  );
}

export function feedbackScreenshotDataUrl(
  screenshot: Pick<PreparedFeedbackScreenshot, "mimeType" | "dataBase64">,
): string {
  return `data:${screenshot.mimeType};base64,${screenshot.dataBase64}`;
}
