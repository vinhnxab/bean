import { useStatus } from "@/features/status/queries";
import { useI18n } from "@/i18n";

/**
 * Chấm "sống" hiển thị trạng thái kết nối tới model, đặt ở header sidebar trái.
 *
 * Nguồn sự thật là `/api/status` (hook `useStatus`, refetch 30 giây): endpoint
 * này chỉ trả lời khi backend đang chạy và cấu hình model đọc được. Ba trạng
 * thái: đã kết nối (xanh `--live`, kèm tên model), đang kết nối (hổ phách
 * `--need`, khi query chưa có kết quả lần đầu), mất kết nối (đỏ `--alert`).
 *
 * Hiệu ứng "động" là chấm `animate-ping` nở ra phía sau chấm tĩnh — ngôn ngữ
 * chỉ-báo-trực-tiếp quen thuộc của iOS. Màu đi hết qua token, không dùng màu
 * Tailwind thô (AGENTS.md mục 12.1).
 */
export function ModelStatusPill() {
  const { t } = useI18n();
  const status = useStatus();

  if (status.isError) {
    return <StatusDot dot="bg-alert" label={t("status.modelDisconnected")} text="text-alert" />;
  }
  if (status.isPending) {
    return <StatusDot dot="bg-need" label={t("status.modelConnecting")} text="text-muted-foreground" />;
  }
  return (
    <StatusDot
      dot="bg-live"
      label={status.data?.model ?? t("status.modelConnected")}
      text="text-muted-foreground"
    />
  );
}

function StatusDot({ dot, label, text }: { dot: string; label: string; text: string }) {
  return (
    // Không dùng `role="status"`: đây là chỉ báo thụ động, trình đọc màn hình
    // không nên đọc to mỗi lần refetch 30 giây đổi trạng thái — và `getByRole`
    // trong test cũng trùng với skeleton loading. `title` là đủ để thấy trọn
    // tên model khi bị cắt bởi `truncate`.
    <span className="flex min-w-0 items-center gap-2" title={label}>
      <span aria-hidden="true" className="relative flex size-2.5 shrink-0">
        <span className={`absolute inline-flex h-full w-full animate-ping rounded-full opacity-60 ${dot}`} />
        <span className={`relative inline-flex size-2.5 rounded-full ${dot}`} />
      </span>
      <span className={`min-w-0 truncate text-xs font-semibold ${text}`}>{label}</span>
    </span>
  );
}
