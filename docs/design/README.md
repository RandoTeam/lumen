# Design

Актуальная спецификация: [`2026-11-08-ux-spec.md`](2026-11-08-ux-spec.md).

Коротко:

- **Главный экран — Chats**, не пустой чат и не модели.
- **Телефон:** 3 корневых раздела — Chats, Models, Settings; чат — push-экран.
  Нижняя навигация, без hamburger-only.
- **Планшет:** navigation rail + список чатов + беседа (two-pane); Models и
  Settings — list-detail; max width текста 720 dp.
- **Paper & Ink** — светлая тема по умолчанию; **Graphite & Ember** — тёмная.
  Один акцент, тёплые нейтрали, 1 px borders, без градиентов, неона и glow.
- **Модели:** Qwen 0.5B — быстрый старт; Bonsai 2-4B — balanced; Clover — только
  по явному тапу, с честным «file 11.7 GB / active ~2.3 GB / 0.6 tok/s».
- **Никакого Kotlin:** UI будет на Rust (Slint — основной кандидат).
