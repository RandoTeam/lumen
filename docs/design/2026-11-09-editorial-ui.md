# Дизайн: Paper & Ink — Editorial (2026-11-09)

Рабочий прототип: `web/` (Preact + Vite).
Скриншоты: `docs/design/screens/`.
Предыдущая UX-спека: `docs/design/2026-11-08-ux-spec.md` (IA и правила).

## 1. Направление

**Paper & Ink — Editorial.** Это не «ещё один Material», не неон и не
градиенты. Ощущение качественного бумажного издания: тёплая бумага, глубокие
чернила, один зелёный акцент, тонкие линии, много воздуха.

Почему так:

- читается как инструмент, а не игрушка;
- не устаревает вместе с модным градиентом;
- хорошо работает и на 6" телефоне, и на 11" планшете;
- дёшево по ресурсам: нет blur-слоёв на весь экран, тяжёлых теней и
  анимаций;
- serif в заголовках даёт характер, sans в интерфейсе — скорость чтения,
  mono в метриках — инженерную точность.

## 2. Токены

### Цвет

| Токен | Значение | Роль |
|---|---|---|
| `paper` | `#F5F2EA` | фон приложения |
| `paper-2` | `#ECE8DD` | вторичные поверхности, треки |
| `card` | `#FFFCF7` | карточки, поля, композер |
| `ink` | `#171813` | основной текст |
| `ink-2` | `#4A4B44` | вторичный текст |
| `ink-3` | `#7A7A70` | метаданные, подписи |
| `line` | `#E2DCCF` | hairline-границы |
| `line-strong` | `#D3CCBD` | неактивные переключатели |
| `accent` | `#245246` | главный акцент |
| `accent-2` | `#2F6A58` | hover/status |
| `accent-soft` | `#E6EFE9` | активные состояния, user bubble |
| `accent-ink` | `#173A31` | текст на accent-soft |
| `warn` | `#8A5A18` | «не влезает в память» |
| `danger` | `#9C3B2E` | ошибки |

Правила: без градиентов, без glow, максимум одна тень на экран (карточка
композера), статус — точка + текст, а не цветная плашка на весь экран.

### Типографика

| Роль | Шрифт | Размер / интерлиньяж |
|---|---|---|
| Display (h1) | Newsreader 400 | `clamp(42px, 7vw, 54px) / 0.98`, tracking −0.025em |
| Заголовок карточки (h3) | Newsreader 500 | 20px / 1.1 |
| Заголовок чата (h2) | Newsreader 400 | 21px / 1.1 |
| Body | Inter 400 | 15px / 1.45 |
| Сообщение ассистента | Inter 400 | 16.5px / 1.66, max 72ch |
| Метаданные | IBM Plex Mono 500 | 9–10px, tracking 0.01–0.14em |
| Eyebrow / label | Inter 600 | 9.5–10px, uppercase, tracking 0.14–0.16em |

Шрифты подключаются локально в продакшене (subset woff2), не с Google CDN.
В прототипе — Google Fonts для скорости итераций.

### Сетка и форма

- Базовая сетка 4px; отступы экрана 20px (телефон) / 44px (планшет).
- Радиусы: 7px (quant), 10–12px (кнопки), 14–16px (поля, строки),
  18–22px (карточки), 999px (pill).
- Границы 1px `line`; активная карточка — 1px `accent` + inset-ring.
- Тени: `0 1px 2px rgba(25,24,18,.04), 0 6px 18px rgba(25,24,18,.035)`.

### Движение

- Появление экрана/строки: `translateY(8px) → 0`, opacity 0→1, 300–400ms,
  `cubic-bezier(.2,.7,.2,1)`.
- Нажатие: scale 0.96 за 150ms.
- Стриминг: каретка 2px, blink 1s steps.
- `prefers-reduced-motion` и переключатель «Reduce motion» отключают всё.

## 3. Экраны

### Chats (главный)

- Eyebrow с датой, крупный serif «Chats», круглая кнопка «+».
- Поиск — карточка 46px с иконкой.
- Секции Pinned / Recent, строки: монограмма модели, заголовок, сниппет,
  модель с цветной точкой, время mono.
- Активная строка — карточка с левой 3px полосой accent.

### Chat

- Шапка: back (только телефон), имя модели serif, статус-точка, подзаголовок
  `backend · quant · layers`, overflow.
- Сообщения: user — мягкая карточка справа; assistant — редакционный блок
  без пузыря, роль с монограммой, тело 16.5px, метрики mono.
- Стриминг: бейдж `streaming` + каретка.
- Композер: карточка 20px с attach, textarea (auto-grow), mic, круглая
  кнопка отправки; подсказка про Enter/локальность.

### Models

- Storage-карточка: занято/свободно, полоса, легенда по моделям, чип
  устройства.
- Фильтры All / Text / MoE / LoRA.
- Карточка модели: монограмма, имя, arch/layers/context, quant-pill,
  capability chips, заметка, fit-meter `estimate / free`, статус
  «Fits in memory» / «Storage-bound», размер и кнопка Set active.
- Активная модель — accent-рамка и кнопка Active.

### Settings

- Группы Generation / Performance / Appearance / Privacy / About.
- Строки 48px, значения mono/sans, chevron, toggle.
- Внизу — версия и движок.

## 4. Адаптивность

| | Телефон (< 980px) | Планшет (≥ 980px) |
|---|---|---|
| Навигация | нижняя панель 3 раздела | rail 84px с брендом и устройством |
| Chats | список ↔ чат (стек) | список 356px + чат |
| Models | 1 колонка | 2 колонки, max-width 980 |
| Settings | 1 колонка | max-width 980 |
| Chat | back в шапке | back скрыт |

Брейкпоинт 980px, а не 600: на 600–900px two-pane ещё тесно для чтения.

## 5. Реализация

- `web/` — Preact 10 + Vite 7 + TypeScript, без UI-библиотек.
- CSS-переменные вместо токен-файла; один `styles.css`.
- Иконки — inline SVG, stroke 1.7, без icon-font.
- Прод-бандл: **31 KB JS (11.9 KB gzip) + 16.6 KB CSS (4.0 KB gzip)**.
- Прототип: `npm run dev`, `?frame=phone` — рамка телефона,
  `tablet-preview.html` — планшет 1180×820 в масштабе 0.5.
- Никакой бизнес-логики в JS: только представление и анимация стриминга
  (демо).

## 6. Интеграция с Rust

Два варианта, оба сохраняют Rust-ядро:

### A. WebView + JNI-мост (рекомендуется для этого дизайна)

- Rust-ядро и llama.cpp остаются нативными; UI — Preact в системном WebView.
- Ассеты (`dist/`) встраиваются в APK; загрузка через `WebViewAssetLoader`
  или `file:///android_asset`.
- Мост: `@JavascriptInterface` (тонкий Java-шим) или `WebMessageListener`.
  Команды: `models.list`, `models.preflight`, `engine.load`, `engine.send`,
  `engine.cancel`, `engine.stats`, `settings.get/set`.
- Стриминг токенов: Rust → JNI → `evaluateJavascript("window.sllm.token(...)")`
  (или `postWebMessage`). Никакого HTTP-сервера и открытых портов.
- Цена: WebView process +50–90 MB PSS, cold start +150–250 ms. APK почти не
  растёт (JS/CSS < 60 KB + шрифты).
- Плюсы: дизайн-итерации в браузере, доступность WebView (TalkBack, IME,
  выделение, predictive back), богатый текст.

### B. Slint + тот же дизайн

- Сохраняет минимальный PSS (~50 MB) и отсутствие WebView.
- Дизайн переносится вручную; IME/rich text/a11y слабее.
- Имеет смысл, если PSS/старт важнее скорости дизайна.

**Решение:** делать A, но с бюджетом: PSS UI ≤ 120 MB без модели, cold start
≤ 600 ms, APK ≤ 12 MB. Если бюджет нарушен — переносим токены и компоненты в
Slint (B), не меняя ядро.

## 7. Реализовано на устройстве (2026-11-09)

WebView-шелл собран и проверен на MI 8:

- `crates/sllm-webapp` — `MainActivity.java` (тонкий Java: Activity, WebView,
  insets, JNI) + Rust `cdylib` с `nativeVersion`/`nativeModels`.
- `web/dist` упакован в APK; ассеты отдаются через
  `shouldInterceptRequest` с `https://app.local/`, иначе ES-модули на
  `file://` блокируются CORS.
- JNI: `Sllm.models()` → Rust `sllm-core` сканирует
  `Android/data/dev.snapdragonllm/files/models`, считает preflight, RAM-fit
  и отдаёт JSON. В Models видны реальные Clover/Bonsai/Qwen, чипы MoE,
  Ternary, LoRA, статус Storage-bound.
- Insets: `FrameLayout` с padding от `systemBars() + ime()` (IME только когда
  `isVisible`), статус-бар и жест-бар не перекрывают контент.
- При фокусе в композере нижняя навигация скрывается
  (`:has(.composer-input:focus)`), композер остаётся над клавиатурой.
- IME работает: ввод, курсор, отправка, автоскролл; проверено на Gboard RU.

Измерения:

| Метрика | Значение |
|---|---|
| APK (arm64, release) | **796 576 B (0.80 MB)** |
| Cold start | 389–550 ms |
| PSS (demo UI, без модели) | 84–99 MB |
| JS / CSS (gzip) | 13.1 KB / 29.1 KB (CSS + fonts) |

Темы: Light / Dark / Night / AMOLED + System. Переключатель в Settings,
`data-theme`, нативная перекраска WebView/root и иконок баров.
Скриншоты: `device-dark-chats.png`, `device-dark-models.png`,
`device-dark-chat.png`, `device-night-settings.png`,
`device-amoled-settings.png`, `device-light-settings.png`.
Motion: токены 120/200/320/280/140 ms, reduced motion, без blur на SD845.

Скриншоты: `docs/design/screens/device-chats.png`,
`device-chat.png`, `device-chat-ime.png`, `device-models.png`,
`device-settings.png`.

Ограничения текущего среза:

- Чат — демо-стриминг на клиенте; движок ещё не подключён.
- Шрифты уже локальные: Newsreader/Inter/Plex Mono через `@fontsource`
  (латиница; кириллица Inter подключена, Newsreader — latin-only).
- Split-GGUF, mmproj и выбор активной модели пока только в модели данных.

## 8. Доступность и ввод

- Все кнопки — `<button>` с `aria-label`; навигация — `<nav>`.
- Фокус-кольцо 2px accent.
- `aria-live="polite"` на списке сообщений.
- Контраст: ink на paper ≈ 15:1, ink-3 на paper ≈ 4.6:1 (только метаданные).
- IME: textarea с `field-sizing: content`, Enter — отправка, Shift+Enter —
  новая строка; на телефоне подсказка про локальность.
- WebView даёт нативный IME/TalkBack — это главный аргумент A.

## 9. Что дальше

1. Подключить `sllm-llama-sys` и реальный стриминг Qwen 0.5B через JNI.
2. Локальные шрифты (Newsreader/Inter/Plex Mono subset woff2).
3. Split-GGUF, mmproj, выбор активной модели, импорт SAF.
4. Планшетные скриншоты — на живом Pad 2.
