# vendor/

Thư mục này chứa **mã nguồn crate đã vá tay**, không phải dependency của riêng Bean.

## Vì sao?

`cargo build` cảnh báo:

```
warning: the following packages contain code that will be rejected by a future
version of Rust: proc-macro-error2 v2.0.1
```

Đường đi của nó: `teloxide 0.17.0` → `aquamarine 0.6.0` → `proc-macro-error2 2.0.1`.

Đây là **dependency gián tiếp** và không gỡ được bằng feature:

* `aquamarine` là dependency **không optional** của `teloxide`, và không feature nào
  của `teloxide` gate được nó.
* Cả ba crate đều đã ở bản mới nhất, nên không có đường nâng cấp nào.

## Lỗi gốc

`E0365` — `extern crate proc_macro is private and cannot be re-exported`.
Trong `src/lib.rs` bản gốc:

```rust
extern crate proc_macro;   // dòng 277: riêng tư

pub mod __export {
    pub use proc_macro;    // dòng ~494: re-export mục riêng tư → E0365
}
```

## Cách vá

Sửa **một dòng** ở `src/lib.rs`: khai báo `pub extern crate proc_macro;`.
Nhờ vậy việc re-export trở nên hợp lệ, giống hệt cách upstream đã sửa.

Không sửa gì khác. Bản vendor giữ nguyên phần còn lại của crate 2.0.1, cùng
`LICENSE-MIT` / `LICENSE-APACHE`.

## Khi nào gỡ được

Khi upstream phát hành bản đã sửa:

1. Xoá khối `[patch.crates-io]` trong `Cargo.toml` (workspace root).
2. Xoá thư mục này.
3. `cargo update -p proc-macro-error2` rồi `make check`.

Theo dõi: `docs/known-issues.md` mục **K21**.
