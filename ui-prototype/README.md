# UI Prototype

Nơi dựng giao diện tĩnh (HTML/CSS/JS) trước khi port sang template SSR + HTMX của hub.

## Cấu trúc

Mỗi **theme** là một thư mục riêng dưới `ui-prototype/`:

```
ui-prototype/
  simple/            ← theme đầu tiên
    index.html
    css/styles.css
    js/app.js
    pages/
      monitors.html
      status.html
  <theme-khác>/      ← thêm theme mới = thêm thư mục
```

## Quy ước

- Mỗi theme tự chứa (self-contained): css/js/pages riêng, mở `index.html` là chạy được.
- Dùng **Tailwind CDN** cho nhanh khi prototype. Bản production của hub dùng Tailwind
  standalone CLI (không Node) — class name giữ nguyên nên port sang SSR rất nhẹ.
- Vanilla JS, không build step.

## Chạy thử

Mở thẳng file, hoặc serve tĩnh:

```bash
cd ui-prototype && python3 -m http.server 4000
# rồi mở http://localhost:4000/simple/
```
