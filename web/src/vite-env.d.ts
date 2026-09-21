/// Khai báo kiểu cho Vite (cho phép `import "./index.css"` và `import.meta.env`).
/// TypeScript 7 yêu cầu khai báo module cho import chỉ-có-tác-dụng-phụ (CSS, asset).
/// Ngoài ra các biến `import.meta.env.VITE_*` cũng được định nghĩa ở đây.
/// Vite đã cung cấp sẵn định nghĩa này qua `vite/client`.
/// <reference types="vite/client" />
