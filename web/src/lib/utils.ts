import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Gộp class Tailwind, xử lý xung đột (hàm `cn` chuẩn của shadcn/ui — agents.md mục 3.2).
 * component trong `components/ui/` (do shadcn CLI sinh, M10) dùng hàm này.
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
