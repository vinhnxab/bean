import type { AgentReportDto } from "@/api/bindings";
import { BeanAvatar } from "@/components/brand/BeanAvatar";
import { Badge, StatusDot } from "@/components/ui/badge";
import { statusTone } from "@/features/hub/queries";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";

/**
 * Node agent trong sơ đồ.
 *
 * Trạng thái nằm **trong chính node** (chấm + nhãn), không phải badge nhỏ ở góc:
 * "agent nào đang chờ bạn" là thông tin cần thấy ngay nhất màn hình.
 */
function AgentNode({ report, highlighted }: { report: AgentReportDto; highlighted: boolean }) {
  const { t } = useI18n();
  const tone = statusTone(report.status);
  return (
    <div
      data-testid={`agent-node-${report.role}`}
      data-status={report.status}
      data-relation={report.relation}
      className={cn(
        "flex min-w-0 flex-col gap-1 rounded-md border bg-surface-raised px-3 py-2",
        highlighted ? "border-need" : "border-rule",
      )}
    >
      <div className="flex items-center gap-2">
        <StatusDot tone={tone} />
        <span className="truncate text-sm font-medium">{report.role}</span>
      </div>
      <p className="truncate text-xs text-ink-muted">{report.summary}</p>
      <Badge tone={tone} className="self-start">
        {t(`hub.status.${report.status}`)}
      </Badge>
    </div>
  );
}

/**
 * Sơ đồ quan hệ giữa các agent — **nơi duy nhất** được phép "bỏ mựa".
 *
 * # Vì sao cạnh vuông, không phải node–edge cong
 *
 * Quan hệ ở đây mang **ngữ nghĩa kiến trúc**, mà cạnh cong không truyền được:
 *
 * * nét liền = Manager điều phối
 * * nét đôi, hai mũi tên = review four-eyes (không tự duyệt)
 * * nét **đứt** = cảnh báo mức cao đi thẳng tới người quản trị, **không qua
 *   Manager** (D14.11) — đường này cố tình vẽ tách khỏi phần còn lại.
 *
 * Ba kiểu nét đọc được trước khi đọc chữ, và còn đọc được khi in đen trắng.
 */
export function Topology({ agents }: { agents: AgentReportDto[] }) {
  const { t } = useI18n();
  const managed = agents.filter((agent) => agent.relation === "manages");
  const reviewers = agents.filter((agent) => agent.relation === "reviews");
  // Vai trò trực trật: KHÔNG nối vào Manager, chỉ có đường cảnh báo riêng.
  const watchmen = agents.filter((agent) => agent.relation === "alerts_directly");
  const hasWork = agents.some((agent) => agent.status !== "idle");

  const grid =
    agents.length === 0 ? null : (
      <div className="mt-4 flex flex-col items-stretch gap-0">
        {/* Nguồn sáng duy nhất của trang: đỉnh sáng rơi đúng node Manager. */}
        <div
          className={cn(
            "mx-auto w-full max-w-xs rounded-md border border-rule px-4 py-3",
            hasWork ? "bg-live/5" : "bg-surface",
          )}
        >
          <div className="flex items-center gap-2">
            <BeanAvatar size={28} title={t("hub.manager.name")} />
            <div className="min-w-0">
              <p className="truncate text-sm font-semibold">{t("hub.manager.name")}</p>
              <p className="truncate text-xs text-ink-muted">{t("hub.manager.role")}</p>
            </div>
          </div>
        </div>
        {managed.length > 0 ? <ManagedGroup agents={managed} /> : null}
        {reviewers.length > 0 ? <ReviewGroup agents={reviewers} /> : null}
        {watchmen.length > 0 ? <AlertGroup agents={watchmen} /> : null}
      </div>
    );

  return (
    <section
      aria-labelledby="hub-topology-title"
      data-testid="hub-topology"
      className="rounded-md border border-rule bg-surface-raised p-4 sm:p-5"
    >
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 id="hub-topology-title" className="text-sm font-semibold">
          {t("hub.topology.title")}
        </h2>
        <p className="text-xs text-ink-muted">{t("hub.topology.hint")}</p>
      </div>
      {grid}
      {agents.length === 0 ? <TopologyEmpty /> : null}
    </section>
  );
}

/**
 * Nhóm do Manager điều phối.
 *
 * # Vì sao có "bus" ngang
 *
 * Bản đầu tôi vẽ một đường dọc rồi bỏ rơi, không chạm node nào — nhìn ra như
 * sơ đồ của người khác vẽ, và quan hệ "ai nối vào Manager" thành vô nghĩa. Giờ
 * dùng cấu trúc bus như sơ đồ điện: đường dọc từ Manager xuống, một đường ngang,
 * rồi các nhánh dọc **chạm tới đỉnh từng node**. Đường kẻ bây giờ mang thông tin
 * thật thay vì chỉ trang trí.
 */
function ManagedGroup({ agents }: { agents: AgentReportDto[] }) {
  return (
    <div className="flex flex-col items-center" data-testid="hub-edge-manages">
      <Connector columns={agents.length} />
      <NodeGrid agents={agents} />
    </div>
  );
}

/**
 * Cột và khung cho **một nhóm** node, dùng chung cho lưới node lẫn bus nối.
 *
 * # Vì sao phải tính ở một chỗ
 *
 * Lưới là `grid-cols` **theo bề rộng**, còn bus thì vẽ số nhánh bằng **số agent**.
 * Hai thứ lệch nhau: ở mobile lưới xuống 1 cột nhưng bus vẫn vẽ 4 nhánh ngang, khiến
 * các nhánh chỉ vào không khí. Trả về cả `cols` lẫn `frame` từ cùng một hàm để bus
 * và lưới luôn dùng **cùng số cột và cùng khung bề rộng** — bus nằm trong `frame`
 * y hệt lưới nên nhánh thứ i rơi đúng giữa cột thứ i.
 *
 * Ngưỡng chuyển cột là `min-[900px]`: dưới ngưỡng này mọi nhóm là 1 cột nên bus
 * chỉ cần một nhánh dọc giữa. **Một** ngưỡng cho mọi nhóm là cố ý — nhiều ngưỡng
 * hơn nghĩa là nhiều tổ hợp lưới/bus phải khớp, và chỉ cần một lần quên là hỏng.
 *
 * # Vì sao bảng class viết tay thay vì nội suy chuỗi
 *
 * Tailwind v4 quét **nguyên văn** trong source để sinh CSS, nên
 * `` `grid-cols-${n}` `` không tạo ra class nào cả — biểu tượng bằng mọi grid sập
 * về một cột mà không có lỗi nào báo ra. Bảng dưới liệt kê **đầy đủ** tên class
 * để trình quét thấy; thêm cột mới thì thêm một dòng, không có cách nào quên.
 */
/** `số cột` → tên class đầy đủ (không nội suy) + khung bề rộng tương ứng. */
const COLUMN_CLASSES: Record<number, { cols: string; frame: string }> = {
  1: { cols: "grid-cols-1", frame: "max-w-xs mx-auto" },
  2: {
    cols: "grid-cols-1 min-[900px]:grid-cols-2",
    frame: "max-w-2xl mx-auto",
  },
  3: {
    cols: "grid-cols-1 min-[900px]:grid-cols-3",
    frame: "max-w-4xl mx-auto",
  },
  4: {
    cols: "grid-cols-1 min-[900px]:grid-cols-4",
    frame: "max-w-4xl mx-auto",
  },
};

function nodeLayout(count: number): { cols: string; frame: string } {
  return COLUMN_CLASSES[Math.min(count, 4)];
}

function NodeGrid({ agents }: { agents: AgentReportDto[] }) {
  const { cols, frame } = nodeLayout(agents.length);
  return (
    <div className={`grid w-full gap-2 ${cols} ${frame}`}>
      {agents.map((agent) => (
        <AgentNode key={agent.role} report={agent} highlighted={agent.status === "awaiting_you"} />
      ))}
    </div>
  );
}

/**
 * Đoạn nối giữa node phía trên và nhóm phía dưới.
 *
 * Hai phương án cho hai trạng thái lưới, cùng một nghĩa:
 *
 * * `< 900px` — 1 cột: chỉ một nhánh dọc ở giữa.
 * * `>= 900px` — N cột: bus ngang có N nhánh, mỗi nhánh rơi đúng giữa một cột.
 *
 * Nhánh dọc vẽ bằng CSS, bus vẽ bằng SVG: một đường 1px vẽ bằng `stroke-dasharray`
 * bị nén theo chiều ngang nên nét đứt méo, còn `repeating-linear-gradient` giữ
 * được tỉ lệ dù mảnh cỡ nào.
 */
function Connector({ columns, dashed = false }: { columns: number; dashed?: boolean }) {
  const { frame } = nodeLayout(columns);
  const multi = columns > 1;
  return (
    <>
      <div aria-hidden className={`flex justify-center ${multi ? "min-[900px]:hidden" : ""}`}>
        <span className="flex w-px flex-col items-center">
          <span
            className={`h-6 w-px ${dashed ? "" : "bg-live"}`}
            style={
              dashed
                ? {
                    backgroundImage:
                      "repeating-linear-gradient(to bottom, var(--alert) 0 4px, transparent 4px 8px)",
                  }
                : undefined
            }
          />
          {dashed ? (
            <span className="h-0 w-0 border-x-4 border-t-[5px] border-x-transparent border-t-alert" />
          ) : null}
        </span>
      </div>
      {multi ? (
        <div aria-hidden className={`hidden w-full min-[900px]:block ${frame}`}>
          <Bus columns={columns} dashed={dashed} />
        </div>
      ) : null}
    </>
  );
}

/** Bus ngang + các nhánh xuống từng node (chỉ dùng khi `>= 900px`). */
function Bus({ columns, dashed = false }: { columns: number; dashed?: boolean }) {
  const step = 100 / columns;
  const stroke = dashed ? "var(--alert)" : "var(--live)";
  const dash = dashed ? "3 3" : undefined;
  // Gộp các nhánh thành MỘT `path` thay vì N `<line>`: vừa bỏ được key theo
  // index (Biome bắt lỗi đúng — thứ tự các nhánh phụ thuộc số agent nên key index
  // không ổn định), vừa giảm số nút DOM.
  const ticks = Array.from({ length: columns }, (_, index) => {
    const x = (step * (index + 0.5)).toFixed(3);
    return `M${x} 0V3`;
  }).join("");
  return (
    <svg
      aria-hidden
      viewBox="0 0 100 12"
      preserveAspectRatio="none"
      className="h-3 w-full shrink-0 overflow-visible"
    >
      <path
        d={`M${(step / 2).toFixed(3)} 0H${(100 - step / 2).toFixed(3)}${ticks}`}
        fill="none"
        stroke={stroke}
        strokeWidth="1.5"
        strokeDasharray={dash}
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}

/**
 * Nhóm review — nối **bằng nét đôi hai chiều** với hàng agent phía trên.
 *
 * # Vì sao là móc ngoặc bên trái chứ không phải mũi tên giữa trang
 *
 * Bản đầu tôi vẽ mũi tên ngang ở giữa, lơ lửng giữa hai hàng và **không chạm node
 * nào** — người xem không biết nó nối ai với ai. Giờ dùng cấu trúc móc: một đường
 * ngoặc ôm lấy cả hàng review, nối ngược lên hàng agent phía trên bằng nét đôi có
 * hai mũi tên. Đường luôn chạm, và vẫn nói rõ "quan hệ hai chiều".
 *
 * Review là quan hệ **hai chiều** (mỗi bên review việc của bên kia), nên hai mũi
 * tên. Đường cảnh báo thì đi **một** chiều — đó là điểm phân biệt hình thức.
 */
function ReviewGroup({ agents }: { agents: AgentReportDto[] }) {
  const { t } = useI18n();
  return (
    <div data-testid="hub-edge-reviews" className="mt-6">
      {/*
        Review là quan hệ **giữa hai hàng**, không phải giữa hai node cụ thể: mỗi
        bên review việc của bên kia. Bản đầu tôi vẽ móc ngoặc trải hết chiều rộng
        nên hai mũi tên rơi lơ lửng ở hai góc, không chạm gì cả — trông như sơ đồ
        hỏng. Giờ thu lại thành **mũi tên hai chiều đặt giữa**, nằm giữa hàng điều
        phối và hàng review: đúng vị trí nó mô tả, và vẫn đọc được là quan hệ
        hai chiều nhờ có mũi tên ở cả hai đầu.

        Màu `--ink-muted` (không phải `--live`/`--alert`) là cố ý: đây là quan hệ
        cấu trúc, không phải kênh dữ liệu đang chạy.
      */}
      <div className="flex items-center justify-center gap-2">
        {/* Mũi tên hai chiều dựng theo chiều dọc: mũi tên chỉ đúng nghĩa khi nó
            nằm trên trục của đường nối. Xoay tam giác để "chỉ ngang" sẽ ra một
            ký hiệu vô nghĩa. */}
        <span aria-hidden className="flex flex-col items-center">
          <span className="h-0 w-0 border-x-4 border-b-[5px] border-x-transparent border-b-ink-muted" />
          <span className="h-3.5 w-px bg-ink-muted" />
          <span className="h-0 w-0 border-x-4 border-t-[5px] border-x-transparent border-t-ink-muted" />
        </span>
        <p className="text-xs font-medium text-ink-muted">{t("hub.reviews.note")}</p>
      </div>
      <div className="mt-2">
        <NodeGrid agents={agents} />
      </div>
    </div>
  );
}

/**
 * Nhóm trực trật — đường cảnh báo **tách khỏi Manager**.
 *
 * Viền trên đứt + nét đứt + màu `--alert` là ba tín hiệu cộng lại, vì đây là
 * quyết định kiến trúc thật (D14.11): cảnh báo mức cao tới thẳng người quản
 * trị, không lọc qua Manager. Nếu chỉ dựa vào màu thì người mù màu không thấy
 * khác biệt — nên cả ba cùng nói.
 */
function AlertGroup({ agents }: { agents: AgentReportDto[] }) {
  const { t } = useI18n();
  return (
    <div
      data-testid="hub-edge-alerts"
      className="mt-6 flex flex-col items-center border-t border-dashed border-alert pt-4"
    >
      <NodeGrid agents={agents} />
      {/* Đường cảnh báo: cùng bộ nối với nhóm điều phối nhưng nét đứt + màu
          `--alert`, vì nó đi một chiều thẳng xuống "Bạn" chứ không quay lại
          Manager. */}
      <Connector columns={agents.length} dashed />
      {/* Node đích: chính bạn. Không có node này thì "thẳng tới bạn" chỉ là lời hứa
          bằng chữ — đường đứt phải chạm tới một đích thật. */}
      <div
        data-testid="hub-alert-target"
        className="flex flex-col items-center gap-1 rounded-md border border-alert bg-alert/5 px-4 py-2"
      >
        <p className="text-sm font-semibold text-alert">{t("hub.alerts.target")}</p>
        <p className="text-center text-xs text-ink-muted">{t("hub.alerts.note")}</p>
      </div>
    </div>
  );
}

/** Trạng thái rỗng là lời mời hành động, kèm mascot — không phải một dòng xám. */
function TopologyEmpty() {
  const { t } = useI18n();
  return (
    <div className="mt-4 flex flex-col items-center gap-2 py-6 text-center">
      <BeanAvatar size={56} />
      <p className="text-sm font-medium">{t("hub.empty.title")}</p>
      <p className="max-w-sm text-xs text-ink-muted">{t("hub.empty.description")}</p>
    </div>
  );
}
