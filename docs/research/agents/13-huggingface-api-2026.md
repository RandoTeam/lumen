# Отчёт 13. Публичный API Hugging Face Hub для SnapdragonLLM (ноябрь 2026)

Дата проверки: **2026-11-08, 00:20–01:10 UTC**. Все запросы — `curl` без токена,
с одного IP (не-authenticated). Живые ответы HF сверены с официальной документацией
и OpenAPI-спецификацией. Ничего не додумано: где проверки не было — стоит
**«не проверено»**.

Обозначения: ✅ — проверено живым запросом; 📄 — официальная документация;
⚠️ — не проверено / зависит от аккаунта.

---

## TL;DR

- Поиск: `GET https://huggingface.co/api/models`. Работают `search`, `author`,
  `filter` (теги/задачи/языки), `pipeline_tag`, `num_parameters`, `sort`,
  `direction`, `limit`, `cursor`, `full`, `expand`, `config`.
- **`author` чувствителен к регистру** и принимает только каноническое имя:
  `author=Qwen` → есть, `author=qwen` → `[]`; `author=bartowski` → есть,
  `author=Bartowski` → `[]`. Лечится через `/api/organizations/{name}/overview`
  (регистронезависим) или `/api/users/{name}/overview` (307 на канонический логин).
- Пагинация — только cursor через `Link: <...>; rel="next"`. `X-Total-Count`
  в ответах `/api/models` не наблюдался. `limit` фактически максимум **1000**.
- Детали: `GET /api/models/{repo_id}`. **Размеры файлов появляются только с
  `?blobs=true`**; без него `siblings` содержит только `rfilename`.
- `GET /api/models/{id}/tree/main?recursive=true` — самый удобный способ получить
  все файлы, размеры, `lfs.oid` (sha256) и `xetHash`; пагинация cursor,
  по умолчанию 1000 (100 при `expand=true`).
- `POST /api/models/{id}/paths-info/{rev}` — батч до **2000** путей; в ответе
  `size`, `oid`, `lfs.oid`, `xetHash`, `lastCommit`.
- Поле `gguf` в детали модели: `total` (параметры), `architecture`,
  `context_length`, `chat_template`, `totalFileSize`. Есть и в списке через
  `expand=gguf`, но **`totalFileSize` — размер одного «главного» файла, не
  обязательно выбранного кванта**.
- Скачивание: `https://huggingface.co/{repo}/resolve/{rev}/{file}?download=true`
  → `302` на подписанный CDN (`us.aws.cdn.hf.co`), ссылка живёт **≈50 минут**.
  Range/resume работает: `206`, `Content-Range`, `Accept-Ranges: bytes`.
- Заголовки файла: `X-Linked-Size`, `X-Linked-ETag` = **sha256 LFS** (для
  LFS/Xet-файлов), `X-Xet-Hash`; `Last-Modified` нет. ETag на CDN — это Xet-hash, **не** sha256.
- Gated: `401` + `X-Error-Code: GatedRepo` без токена. Gemma 3 (`gated=manual`,
  license `gemma`), Llama 3.x (`manual`), Gemma 4 31B — уже Apache-2.0 и **не**
  gated. Community-GGUF Gemma 3 может быть ungated, но license остаётся `gemma`.
- CORS: **да, из WebView на `https://app.local` работать можно**. HF эхо-ит
  `Origin`, отдаёт `Access-Control-Expose-Headers` с `Link`, `X-Linked-Size`,
  `X-Linked-ETag`, `RateLimit`; preflight разрешает `authorization` и `POST`
  `content-type`; финальный CDN отдаёт `ACAO: *`.
- Rate limit (аноним): Hub API **500 / 5 мин**, Resolvers **3000 / 5 мин**.
  Заголовки: `RateLimit: "api";r=498;t=278`,
  `RateLimit-Policy: "fixed window";"api";q=500;w=300`.
- **Каждый GET и HEAD на `/resolve/` считается скачиванием** (официально).
  Размеры брать из `/tree` или `?blobs=true`, а не HEAD-запросами.
- GGUF-метаданные без полной загрузки: Range первых 4–16 МБ + локальный парсер
  заголовка. В первых ~6 МБ лежат `general.architecture`, `block_count`,
  `context_length`, `file_type`; проверено на реальных файлах.
- RAM: считать `weights + KV + compute + mmproj + runtime`; для MoE вес — весь
  файл, для ternary — размер файла, а не `params × 2 бита`.
- Рекомендуемая архитектура: метаданные можно из JS, но **скачивание — только
  Rust (reqwest/ureq) + foreground service**, с Range, sha256 и отменой.
  `DownloadManager` — только как простой fallback для публичных одиночных файлов.

---

## 1. Поиск моделей

### 1.1 Endpoint и параметры

```http
GET https://huggingface.co/api/models
```

| Параметр | Тип | Что делает | Проверено |
|---|---|---|---|
| `search` | string | Подстрока в `id` модели, **регистронезависимо** | ✅ |
| `author` | string | Точное каноническое имя user/org; **регистрозависимо**; только одно значение | ✅ |
| `filter` | string, можно повторять | Тег/задача/язык/библиотека; несколько `filter` = **AND** | ✅ |
| `pipeline_tag` | string | Фильтр по `pipeline_tag` (`text-generation`, `image-text-to-text`, …) | ✅ |
| `num_parameters` | string | Диапазон параметров: `min:6B,max:9B`, `min:1B`, `max:9B` | ✅ |
| `gated` | bool | `true`/`false` — только gated/не-gated | ✅ |
| `sort` | enum | `downloads`, `likes`, `lastModified`, `createdAt`, `trendingScore` | ✅ |
| `direction` | int | `-1` (desc) или `1` (asc); **asc только для `createdAt`** | ✅ |
| `limit` | int | Фактический максимум **1000**; `5000` → 1000 | ✅ |
| `cursor` | string | Opaque cursor из `Link`-заголовка | ✅ |
| `full` | bool | `true` — расширенный ответ; у **list endpoint** даже `full=false` включает расширенные поля (наблюдаемый кворк) | ✅ |
| `expand` | enum | Одно значение; можно повторять `&expand=...`; список ниже | ✅ |
| `config` | bool | Поверхностный `config` + tokenizer/chat template; не гиперпараметры | ✅ |

Официальные значения `expand` (из ошибки API и OpenAPI):

```
author, baseModels, cardData, config, createdAt, disabled, downloads,
downloadsAllTime, evalResults, gated, inference, inferenceProviderMapping,
lastModified, library_name, likes, mask_token, model-index, pipeline_tag,
private, safetensors, sha, siblings, spaces, tags, transformersInfo,
trendingScore, widgetData, gguf, resourceGroup, xetEnabled
```

Важно: `expand` **не принимает список через запятую**:
`expand=safetensors,gguf` → `400 Invalid option`. Нужно повторять параметр:

```http
GET /api/models?...&expand=safetensors&expand=gguf&expand=downloadsAllTime
```

`library=gguf` **игнорируется** (вернул `sentence-transformers/...`). Для GGUF
использовать `filter=gguf`. Это же делает сайт `hf.co/models?library=gguf`,
но API-параметр `library` не работает.

### 1.2 Что реально возвращается

Минимальный ответ (без `full`/`expand`):

```json
[
  {
    "_id": "6ab1ab2a0904bf417755525e",
    "id": "bartowski/MiMo-V2.6-Distill-Qwen-9B-GGUF",
    "likes": 186,
    "trendingScore": 11,
    "private": false,
    "downloads": 989262,
    "tags": ["gguf", "mimo_v2", "conversational", "license:apache-2.0"],
    "pipeline_tag": "image-text-to-text",
    "createdAt": "2026-09-21T22:09:46.000Z",
    "modelId": "bartowski/MiMo-V2.6-Distill-Qwen-9B-GGUF"
  }
]
```

В минимальном ответе **нет** `author`, `sha`, `lastModified`, `siblings`,
`cardData`. С `full=true` (или даже `full=false`) появляются:

```json
{
  "id": "Qwen/Qwen3-4B",
  "author": "Qwen",
  "gated": false,
  "lastModified": "2025-07-26T03:46:39.000Z",
  "sha": "1cfa9a7208912126459214e8b04321603b3df60c",
  "downloads": 6379202,
  "tags": ["transformers", "safetensors", "qwen3", "license:apache-2.0"],
  "pipeline_tag": "text-generation",
  "library_name": "transformers",
  "createdAt": "...",
  "siblings": [{"rfilename": "config.json"}]
}
```

Но `full=true` в **списке** в проверенных ответах не дал `cardData`,
`config`, `safetensors` и `gguf` — только в детали или через `expand`.

Примечание: кворк с `full=false` наблюдался только у **list** endpoint. У
`/api/models/{id}` ответ и без `full` уже содержит `cardData`, `config`,
`gguf`, `safetensors` (проверено); `full` на detail почти ничего не меняет.

### 1.3 Сортировки: важные ограничения

| `sort` | По умолчанию | `direction=1` (asc) |
|---|---|---|
| не указан | `trendingScore` desc (проверено на `filter=gguf`: совпадает с явным) | — |
| `downloads` | desc | ❌ `400 only descending sort is supported` |
| `likes` | desc | ❌ |
| `lastModified` | desc | ❌ |
| `trendingScore` | desc | ❌ |
| `createdAt` | desc | ✅ работает (старые сначала) |

`sort=relevance` **не существует** (`400 Invalid sort parameter`); релевантность
включается автоматически, когда есть `search` и не задан `sort`.
`direction=desc`/`asc` — не принимаются, только `-1`/`1`.
`downloads` — счётчик за период (в UI показывается как downloads last month);
`downloadsAllTime` — только через `expand=downloadsAllTime` ✅
(пример: `Qwen/Qwen3.5-4B` — `downloadsAllTime: 56957681`).
`trendingScore` — внутренняя метрика, формула не документирована ⚠️.

### 1.4 Пагинация

Только cursor. Ответ содержит:

```http
Link: <https://huggingface.co/api/models?filter=gguf&limit=3&sort=downloads&direction=-1&cursor=eyIkb3IiOlt7ImRvd25sb2FkcyI6Njg4NTY4NiwiX2lkIjp7IiRndCI6IjZhOWU4NmQxZGFiN2Y4ODAzMGUyYWYyMCJ9fSx7ImRvd25sb2FkcyI6eyIkbHQiOjY4ODU2ODZ9fSx7ImRvd25sb2FkcyI6bnVsbH1dLCJzZWFyY2hTZXF1ZW5jZVRva2VuIjoiQ0xiUTNBSWFDU0VBQUFDQVRVUmFRUm9PV2d4cW5vYlIycmY0Z0REaXJ5QWlEbG9NYXA2ZzBkcTMrSUF3NHE4ZyJ9>; rel="next"
```

- Курсор opaque, уже URL-encoded (`%3D` в конце). Его надо использовать **как
  есть**, не декодируя и не пере-кодируя вручную.
- Следующей страницы нет — заголовка `Link` нет.
- `X-Total-Count` в `/api/models` **не наблюдался** (хотя он в
  `Access-Control-Expose-Headers`). Не полагаться.
- Для поиска cursor содержит `searchSequenceToken` — менять запрос между
  страницами нельзя, иначе курсор невалиден.

Лимит: `limit=1000` максимум; `limit=5000` вернул 1000 ✅. Для мобильного
интерфейса разумно 20–40 на страницу, но не более 1000.

### 1.5 Rate limit

Официальная таблица (docs, сентябрь 2025; значения могут меняться) 📄:

| План | API / 5 мин | Resolvers / 5 мин | Pages / 5 мин |
|---|---:|---:|---:|
| Аноним (по IP) | 500 | 3000 | 100 |
| Free (токен) | 1000 | 5000 | 200 |
| PRO | 2500 | 12000 | 400 |
| Team | 3000 | 20000 | 400 |
| Enterprise | 6000 | 50000 | 600 |

Наблюдено анонимно ✅:

```http
RateLimit: "api";r=498;t=278
RateLimit-Policy: "fixed window";"api";q=500;w=300
```

Для `/resolve/`:

```http
RateLimit: "resolvers";r=2999;t=38
RateLimit-Policy: "fixed window";"resolvers";q=3000;w=300
```

`r` — остаток, `t` — секунд до сброса окна. При 429 нужно ждать `t` (документация
`huggingface_hub` 1.2+ делает это автоматически). `Retry-After` не проверялся ⚠️.
Анонимные запросы получают предупреждение:

```http
X-HF-Warning: unauthenticated; Warning: You are sending unauthenticated requests
to the HF Hub. Please set a HF_TOKEN to enable higher rate limits and faster downloads.
```

Практический бюджет: 500 API-запросов / 5 мин = 1.67 rps. Поиск с debounce
300–400 мс и кэшем укладывается с большим запасом; `paths-info`/`tree` — это
тоже API-бакет, не resolvers.

### 1.6 Примеры запросов

```http
# Поиск GGUF по названию, популярные
GET https://huggingface.co/api/models?search=Qwen3-4B&filter=gguf&sort=downloads&direction=-1&limit=20&full=true

# Конкретный автор + GGUF + диапазон параметров
GET https://huggingface.co/api/models?author=unsloth&filter=gguf&num_parameters=min:1B,max:9B&sort=lastModified&direction=-1&limit=20&full=true

# Только метаданные для карточек: параметры, лицензия, downloadsAllTime
GET https://huggingface.co/api/models?search=gemma&filter=gguf&limit=20&expand=gguf&expand=safetensors&expand=cardData&expand=downloadsAllTime
```

Пример ответа с `expand=gguf` (chat_template обрезан):

```json
[
  {
    "_id": "6a...",
    "id": "unsloth/Qwen3.8-27B-GGUF",
    "trendingScore": 114,
    "gguf": {
      "total": 27320697856,
      "architecture": "qwen35",
      "context_length": 262144,
      "chat_template": "{%- ... -%}",
      "totalFileSize": 16056478688
    }
  }
]
```

`expand=gguf` тянет `chat_template` (десятки-сотни КБ). В приложении его надо
сразу выбрасывать и хранить только `total`, `architecture`, `context_length`,
`totalFileSize`.

---

## 2. Детали модели

### 2.1 Endpoint

```http
GET https://huggingface.co/api/models/{repo_id}
```

`repo_id` — `author/name`, слеш **не** кодируется (`Qwen/Qwen3-4B`).

| Поле | Тип | Комментарий |
|---|---|---|
| `id`, `modelId` | string | `author/name` |
| `author` | string | канонический автор |
| `sha` | string | commit sha; использовать для pinned download |
| `createdAt`, `lastModified` | ISO-8601 | |
| `downloads` | int | счётчик (GGUF: все `.gguf`; GET и HEAD считаются) |
| `likes` | int | в детали; `trendingScore` есть в списке, в detail не наблюдался |
| `private`, `gated` | bool / `"auto"` / `"manual"` | `gated` может отсутствовать в минимальном списке |
| `tags` | string[] | `license:*`, `base_model:*`, `base_model:quantized:*`, `gguf`, язык, задача |
| `pipeline_tag` | string | |
| `library_name` | string | у GGUF-репозиториев часто отсутствует |
| `cardData` | object | `license`, `license_link`, `base_model`, `datasets`, `language`, `quantized_by`, …; **может отсутствовать** |
| `config` | object | только `architectures`, `model_type`, `tokenizer_config`; **не** `hidden_size`/`num_hidden_layers` |
| `siblings` | array | файлы; размеры только при `?blobs=true` |
| `gguf` | object | только для GGUF-репо: `total`, `architecture`, `context_length`, `chat_template`, `totalFileSize` |
| `safetensors` | object | только для safetensors: `parameters` по dtype и `total` |
| `usedStorage` | int | служебное; может включать историю/ревизии, **не** размер загрузки |
| `spaces`, `widgetData`, `model-index`, `transformersInfo`, `inference` | — | не нужны для каталога |

`cardData` реального GGUF-репо:

```json
{
  "quantized_by": "bartowski",
  "pipeline_tag": "image-text-to-text",
  "base_model_relation": "quantized",
  "tags": ["mimo_v2", "agentic", "distillation", "code", "tool-use"],
  "base_model": "XiaomiMiMo/MiMo-V2.6-Distill-Qwen-9B"
}
```

У Qwen3-4B:

```json
{
  "library_name": "transformers",
  "license": "apache-2.0",
  "license_link": "https://huggingface.co/Qwen/Qwen3-4B/blob/main/LICENSE",
  "pipeline_tag": "text-generation",
  "base_model": ["Qwen/Qwen3-4B-Base"]
}
```

Лицензия не всегда в `cardData.license` (у MiMo её там нет); fallback — тег
`license:*` в `tags`.

`baseModels` через `expand=baseModels` (деталь):

```json
{"relation": "quantized", "models": [{"id": "XiaomiMiMo/MiMo-V2.6-Distill-Qwen-9B"}]}
```

`safetensors` для Qwen3-4B (`full=true`/`expand=safetensors`):

```json
{"parameters": {"BF16": 4022468096}, "total": 4022468096}
```

### 2.2 Размеры файлов: `?blobs=true`

**Критично:** без `blobs=true` в `siblings` только имена:

```json
{"rfilename": "MiMo-V2.6-Distill-Qwen-9B-IQ2_M.gguf"}
```

С `?blobs=true`:

```json
{
  "rfilename": "MiMo-V2.6-Distill-Qwen-9B-IQ2_M.gguf",
  "blobId": "7e861b845b5ee53069049c9b645366f899396f45",
  "size": 3536901664,
  "lfs": {
    "sha256": "d5f703d1bf31c2d790a20e14df1e5a118b74cbf28995f60ebf734c32e2046d82",
    "size": 3536901664,
    "pointerSize": 135
  }
}
```

Поля:
- `blobId` — git blob sha1 (не для проверки содержимого);
- `size` — фактический размер;
- `lfs.sha256` — **sha256 содержимого** (то, что нужно для integrity);
- `lfs.pointerSize` — размер LFS pointer (135 байт), не файла.

### 2.3 `tree`: все файлы и размеры одним запросом

```http
GET https://huggingface.co/api/models/{id}/tree/main?recursive=true&expand=true&limit=1000
```

Ответ — массив:

```json
[
  {
    "type": "file",
    "oid": "7e861b845b5ee53069049c9b645366f899396f45",
    "size": 3536901664,
    "lfs": {
      "oid": "d5f703d1bf31c2d790a20e14df1e5a118b74cbf28995f60ebf734c32e2046d82",
      "size": 3536901664,
      "pointerSize": 135
    },
    "xetHash": "74cbc46b2556070f509c50a6deb508f1205fe8612248e00e722c08e702fd561f",
    "path": "MiMo-V2.6-Distill-Qwen-9B-IQ2_M.gguf",
    "lastCommit": {"id": "6715349c...", "title": "Upload ...", "date": "2026-09-21T22:36:42.000Z"}
  }
]
```

Важно: в `tree` поле называется **`lfs.oid`**, а в
`/api/models/{id}?blobs=true` — **`lfs.sha256`**. Значение одно и то же.

Параметры `tree` (OpenAPI):

| Параметр | Значение |
|---|---|
| `recursive` | `true` — все файлы рекурсивно; `false` — только корень |
| `expand` | `true` — добавляет `lastCommit`, security-статусы |
| `limit` | по умолчанию **1000**; при `expand=true` — **100** |
| `cursor` | из `Link: rel="next"` |

Для больших репозиториев (тысячи файлов) обязательно следовать cursor, иначе
получите только первую 1000/100.

### 2.4 `paths-info` (POST, батч)

```http
POST https://huggingface.co/api/models/{id}/paths-info/main
Content-Type: application/json

{"paths": ["MiMo-V2.6-Distill-Qwen-9B-IQ2_M.gguf", "README.md"], "expand": true}
```

Ответ:

```json
[
  {
    "type": "file",
    "oid": "7e861b845b5ee53069049c9b645366f899396f45",
    "size": 3536901664,
    "lfs": {"oid": "d5f703...", "size": 3536901664, "pointerSize": 135},
    "xetHash": "74cbc46b...",
    "path": "MiMo-V2.6-Distill-Qwen-9B-IQ2_M.gguf",
    "lastCommit": {"id": "...", "title": "...", "date": "..."}
  }
]
```

Ограничение: **до 2000 путей** за запрос (JSON schema: `maxItems: 2000`).
`paths` может быть строкой или массивом. CORS preflight для POST разрешён ✅.

### 2.5 Реальные GGUF-репозитории (ноябрь 2026)

| Репозиторий | Файл | Размер (байт) | Комментарий |
|---|---|---:|---|
| `bartowski/MiMo-V2.6-Distill-Qwen-9B-GGUF` | `...-IQ2_M.gguf` | 3 536 901 664 | 8.95B параметров, arch `qwen35` |
| `unsloth/Qwen3.8-27B-GGUF` | `Qwen3.8-27B-Q4_0.gguf` | 16 056 478 688 | 27.3B параметров, есть BF16-шарды 54.66 GB |
| `Qwen/Qwen3-4B-GGUF` | `Qwen3-4B-Q4_K_M.gguf` | 2 497 280 256 | 4.02B параметров |
| `Qwen/Qwen2.5-0.5B-Instruct-GGUF` | `...-q4_k_m.gguf` | 491 400 032 | 0.49B |
| `ewchampion/Ternary-Bonsai-1.7B-TQ2_0-GGUF` | `...-TQ2_0.gguf` | 624 576 416 | ternary, `file_type=37` |
| `ales27pm/lumen-qwen3-bootstrap-adapters-gguf` | `lumen-cortex-lora.gguf` | 104 622 848 | LoRA-адаптер, не модель |

TheBloke жив: `TheBloke/Llama-2-7B-Chat-GGUF` отдаётся, файл
`llama-2-7b-chat.Q4_K_M.gguf` = 4 081 004 224 байта ✅. (Репозиторий старый,
но HF перенёс его в Xet-хранилище.)

`gguf.totalFileSize` — **не** размер выбранного кванта:
для `Qwen/Qwen3-4B-GGUF` он равен 2 497 280 256 (это Q4_K_M), для
`unsloth/Qwen3.8-27B-GGUF` — 16 056 478 688 (Q4_0), для MiMo — 17 920 693 440
(bf16). Hub выбирает один «главный» файл. Для честной оценки брать `tree`.

`usedStorage` тоже не использовать: у MiMo 139 933 806 528 ≈ сумма файлов, у
Qwen3.8-27B — 1 053 655 307 648 при сумме текущих файлов ~472 GB. Судя по
расхождению, поле включает не только текущие файлы (историю/ревизии). Это не
размер загрузки.

---

## 3. Скачивание

### 3.1 URL и редирект

```http
GET https://huggingface.co/{repo_id}/resolve/{revision}/{path}?download=true
```

`revision` — `main`, ветка, тег или **полный** commit sha (короткий sha не
принимается 📄). Для воспроизводимости пинить `sha` из детали:

```http
GET https://huggingface.co/bartowski/MiMo-V2.6-Distill-Qwen-9B-GGUF/resolve/4371da10c84fb26da3592d4cf312d24aa82b7b65/MiMo-V2.6-Distill-Qwen-9B-IQ2_M.gguf
```

Ответ (анонимно, `HEAD`, но GET такой же):

```http
HTTP/1.1 302 Found
Location: https://us.aws.cdn.hf.co/xet-bridge-us/.../74cbc46b...?X-Xet-Cas-Uid=public&...&Expires=1794101478&Signature=...
X-Repo-Commit: 4371da10c84fb26da3592d4cf312d24aa82b7b65
Accept-Ranges: bytes
X-Linked-Size: 3536901664
X-Linked-ETag: "d5f703d1bf31c2d790a20e14df1e5a118b74cbf28995f60ebf734c32e2046d82"
X-Xet-Hash: 74cbc46b2556070f509c50a6deb508f1205fe8612248e00e722c08e702fd561f
Cache-Control: no-store
Link: <.../xet-read-token/...>; rel="xet-auth", <.../reconstructions/...>; rel="xet-reconstruction-info"
```

Финальный CDN:

```http
HTTP/1.1 200 OK
content-type: application/octet-stream
accept-ranges: bytes
content-disposition: inline; filename*=UTF-8''MiMo-...gguf; filename="MiMo-...gguf"
etag: "74cbc46b2556070f509c50a6deb508f1205fe8612248e00e722c08e702fd561f"
content-length: 3536901664
access-control-allow-origin: *
```

`?download=true` меняет только `response-content-disposition` на `attachment`
(проверено: в `Location` появляется `attachment%3B+filename...`). На выбор
локального имени не влияет — имя брать из `rfilename`.

### 3.2 Range / resume

```http
GET .../resolve/main/file.gguf
Range: bytes=0-1023
```

→ `302`, затем:

```http
HTTP/1.1 206 Partial Content
accept-ranges: bytes
content-range: bytes 0-1023/3536901664
content-length: 1024
etag: "74cbc46b..."
access-control-allow-origin: *
```

Проверено ✅. `Accept-Ranges: bytes` есть и на 302, и на 206.
Докачка:

1. Знать уже скачанные `N` байт.
2. Отправить `Range: bytes=N-`.
3. Проверить `206` и `Content-Range: bytes N-.../total`.
4. Если пришёл `200` — сервер не принял Range; начать заново.
5. `If-Range` можно использовать с ETag, но **ETag на HF-редиректе и CDN
   разный**: HF отдаёт `X-Linked-ETag` (sha256), CDN — `etag` (Xet hash).
   Надёжнее сравнивать `X-Linked-ETag` между повторными resolve, а не `ETag`
   CDN.

`Last-Modified` **нет** ни на HF, ни на CDN (проверено). Не строить логику
докачки на нём.

### 3.3 Срок жизни ссылки

Подписанный CDN URL живёт **≈50 минут** (наблюдено: `Expires - now` =
2906…3151 секунд). Если скачивание >50 минут, старый URL начнёт отдавать
ошибку (вероятно 403; не проверено). Тогда нужно заново сделать resolve
(лучше на тот же `sha`) и продолжить с `Range` — файл тот же,
`X-Linked-ETag` тот же.

### 3.4 LFS / Xet / целостность

- `lfs.oid` (tree) = `lfs.sha256` (detail blobs) = `X-Linked-ETag` =
  sha256 **содержимого файла**.
- `X-Xet-Hash` и `ETag` CDN — Xet-хеш, **не** sha256; для проверки не подходит.
- `lfs.pointerSize` (135) — размер pointer-файла, не содержимого.
- Для старых чисто-LFS репозиториев Range также стандартен, но отдельно
  проверить не удалось ⚠️ (все проверенные репозитории уже Xet-backed).

Проверка после скачивания: `sha256(file) == lfs.oid` и `size == lfs.size`.
Лучше считать sha256 **потоком при записи**, чтобы не читать гигабайты второй раз.

### 3.5 Gated-модели

Без токена:

```http
HTTP/1.1 401 Unauthorized
X-Error-Code: GatedRepo
X-Error-Message: Access to model google/gemma-3-4b-it is restricted. You must
have access to it and be authenticated to access it. Please log in.
```

С токеном:

```http
Authorization: Bearer hf_xxxxxxxxxxxxxxxxx
```

Токен нужен и для API, и для `/resolve/`. Запрос доступа к gated-модели
оформляется **только в браузере** на странице модели 📄; в приложении нельзя
«принять лицензию» за пользователя. Сценарий: показать «Gated», кнопку
«Открыть на HF», после возврата — проверить доступ запросом с токеном
(⚠️ не проверено, нет аккаунта).

Наблюдённые gated-значения ✅:

| Модель | `gated` | `cardData.license` |
|---|---|---|
| `google/gemma-3-4b-it` | `manual` | `gemma` |
| `meta-llama/Llama-3.2-1B` | `manual` | `llama3.2` |
| `meta-llama/Llama-3.1-8B-Instruct` | `manual` | `llama3.1` |
| `google/gemma-4-31B-it` | `false` | `apache-2.0` |
| `mistralai/Mistral-7B-Instruct-v0.3` | `false` | `apache-2.0` |
| `Qwen/Qwen3-4B` | `false` | `apache-2.0` |
| `unsloth/gemma-3-4b-it-GGUF` | `false` | `gemma` (base: `google/gemma-3-4b-it`) |

То есть gated — свойство **репозитория**, а не модели: community-GGUF Gemma 3
не gated, но лицензия `gemma` и базовые условия Google всё равно действуют.
Gemma 4 (2026) уже Apache-2.0 и без gate — это важно для каталога.

---

## 4. GGUF-специфика

### 4.1 Как найти `.gguf`

1. `GET /api/models/{id}/tree/main?recursive=true` → фильтр `path.endsWith(".gguf")`.
2. Или `GET /api/models/{id}?blobs=true` → `siblings[].rfilename`.
3. `filter=gguf` в поиске — это тег **репозитория**, не список файлов.

### 4.2 Квант по имени

Имя — основной и самый надёжный источник для UI. Типичные токены:

```
Q2_K, Q3_K_S/M/L, Q4_0, Q4_1, Q4_K_S/M/L, Q5_0, Q5_1, Q5_K_S/M,
Q6_K, Q8_0, F16, BF16,
IQ1_S/M, IQ2_XXS/XS/S/M, IQ3_XXS/XS/S/M, IQ4_XS/NL,
TQ1_0, TQ2_0, Q2_0, MXFP4,
UD-... (Unsloth Dynamic), QAT, XL/L (Unsloth-варианты)
```

Не кванты и не модели: `mmproj-*`, `imatrix*`, `*calibration*`, `mtp-*`,
`*-lora.gguf`, `*adapter*`.

Приблизительный bpw (из официальной таблицы GGUF docs) 📄:

| Тип | bpw |
|---|---:|
| TQ1_0 | 1.69 |
| IQ1_S | 1.56 |
| IQ1_M | 1.75 |
| IQ2_XXS | 2.06 |
| TQ2_0 | 2.06 |
| IQ2_XS | 2.31 |
| IQ2_S | 2.50 |
| Q2_K | 2.63 |
| IQ3_XXS | 3.06 |
| Q3_K | 3.44 |
| IQ4_XS | 4.25 |
| Q4_K | 4.50 |
| Q5_K | 5.50 |
| Q6_K | 6.56 |
| Q8_0 | 8.50 |
| MXFP4 | ~4.25 |

Для `UD-*`/`XL`/`L` точный bpw по имени не выводится: Unsloth смешивает типы
по тензорам. Считать по размеру файла.

### 4.3 Квант по метаданным GGUF

`general.file_type` — числовой enum, зависит от версии/форка. Проверено ✅:

| Файл | `general.file_type` |
|---|---:|
| `Qwen/Qwen2.5-0.5B-Instruct-GGUF/qwen2.5-0.5b-instruct-q4_k_m.gguf` | **15** (Q4_K_M) |
| `ewchampion/Ternary-Bonsai-1.7B-TQ2_0-GGUF/...TQ2_0.gguf` | **37** (TQ2_0) |

В форке проекта (`02-ternary.md`) дополнительно: `TQ1_0=34`, `TQ2_0=35`,
`Q2_0=42` — но это уже нумерация конкретного дерева, не универсальная.
Поэтому в приложении: **имя — первично, `file_type` — уточнение**; неизвестный
`file_type` не должен блокировать загрузку, если имя распознано.

### 4.4 Метаданные без скачивания всего файла

Есть два уровня.

**(A) Агрегат от Hub** — поле `gguf` в детали или `expand=gguf` в списке.
Отдельного endpoint `/api/models/{id}/gguf` **нет**: живой запрос
`GET https://huggingface.co/api/models/unsloth/Qwen3.8-27B-GGUF/gguf` вернул
`404 {"error":"Sorry, we can't find the page you are looking for."}` ✅.

```json
{
  "total": 8953803264,
  "architecture": "qwen35",
  "context_length": 262144,
  "chat_template": "...",
  "totalFileSize": 17920693440
}
```

Это даёт число параметров и архитектуру, но **не** даёт `file_type`,
`block_count`, `head_count_kv`, `key_length` конкретного файла.

**(B) Range + локальный парсер** — официально рекомендованный подход
(`@huggingface/gguf` работает именно так) 📄:

```http
GET .../resolve/main/file.gguf
Range: bytes=0-16777215
```

Заголовок GGUF:

```
magic "GGUF" (4 bytes)
version uint32
tensor_count uint64
metadata_kv_count uint64
[key string][value_type uint32][value] ...
```

Проверено на реальном TQ2_0 ✅ (8 МБ хватило; metadata закончилась на 5 927 014):

```
magic=GGUF version=3 tensor_count=310 kv_count=35
general.architecture = qwen3
general.size_label = 1.7B
general.license = apache-2.0
qwen3.block_count = 28
qwen3.context_length = 32768
qwen3.embedding_length = 2048
qwen3.attention.head_count = 16
qwen3.attention.head_count_kv = 8
qwen3.attention.key_length = 128
qwen3.attention.value_length = 128
general.file_type = 37
```

Для MiMo (248 320 токенов в tokenizer) 8 МБ **не хватило** — metadata
продолжается дальше (парсер остановился на tokenizer-массивах). Практика:
запрашивать 8–16 МБ, парсить до нужных ключей, не требовать весь header.
`tokenizer.ggml.tokens` — огромный массив; его можно пропускать, зная тип
`ARRAY` (uint32 elem_type + uint64 count).

Что даёт header для RAM/UI: `architecture`, `block_count`, `embedding_length`,
`context_length`, `attention.head_count`, `attention.head_count_kv`,
`attention.key_length`, `attention.value_length`, `feed_forward_length`,
`expert_count`/`expert_used_count` (MoE), `file_type`, `general.size_label`,
`general.license`, `general.base_model.*`.

### 4.5 Split shards

Пример ✅:

```
BF16/Qwen3.8-27B-BF16-00001-of-00002.gguf   49 986 159 616
BF16/Qwen3.8-27B-BF16-00002-of-00002.gguf    4 671 576 000
```

Правило: regex `-(\d{5})-of-(\d{5})\.gguf$`; группировать по префиксу до
`-00001-of-`; проверять, что есть все `N` частей; сумма размеров = размер
модели. llama.cpp загружает первую часть и сам находит остальные по имени —
**не склеивать** файлы и не переименовывать; сохранять исходную структуру
каталогов (`BF16/...`).

### 4.6 mmproj / vision

Примеры ✅ (MiMo):

```
mmproj-MiMo-V2.6-Distill-Qwen-9B-f16.gguf    918 166 048
mmproj-MiMo-V2.6-Distill-Qwen-9B-bf16.gguf   921 704 992
```

Другие варианты имён: `mmproj-*.gguf`, `*-mmproj.gguf`,
`mmproj-model-f16.gguf`, `vision-*.gguf`, `projector-*.gguf`.
Это отдельный файл, который надо качать и подключать отдельно; в RAM он
занимает свой размер + vision compute. Без него модель работает как текстовая.

### 4.7 LoRA adapters

Пример ✅ (`ales27pm/lumen-qwen3-bootstrap-adapters-gguf`):

```
lumen-cortex-lora.gguf   104 622 848
```

Метаданные адаптера (Range 1 МБ):

```
general.architecture = qwen3
general.type = adapter
adapter.type = lora
general.base_model.0.name = Qwen3 1.7B Unsloth Bnb 4bit
general.base_model.0.organization = Unsloth
general.base_model.0.repo_url = https://huggingface.co/unsloth/Qwen3-1.7B-unsloth-bnb-4bit
adapter.lora.alpha = 48
```

Правила:
- `general.type == "model"` — полноценная модель; `"adapter"` — LoRA.
- В общем списке моделей адаптеры лучше не показывать как самостоятельные;
  отдельная вкладка «LoRA» с привязкой к `base_model`.
- Проверять совместимость базы по `general.base_model.0.repo_url` /
  `cardData.base_model`; sha базы в адаптере нет, поэтому возможны
  несовпадения версий.
- RAM: размер адаптера + compute; веса базы не дублируются.

---

## 5. Чувствительность `author` к регистру

### 5.1 Живые результаты

| Запрос | Результат |
|---|---|
| `?author=Qwen&limit=3` | 3 модели ✅ |
| `?author=qwen&limit=3` | `[]` ✅ |
| `?author=QWEN&limit=1` | `[]` ✅ |
| `?author=bartowski&limit=2` | 2 модели ✅ |
| `?author=Bartowski&limit=2` | `[]` ✅ |
| `?author=barTowski&limit=1` | `[]` ✅ |
| `?author=TheBloke&limit=1` | `TheBloke/KafkaLM-70B-German-V0.1-GGUF` ✅ |
| `?author=thebloke&limit=1` | `[]` ✅ |
| `?author=THebloke&limit=1` | `[]` ✅ |
| `?author=Unsloth&limit=1` | `[]` ✅ (канон — `unsloth`) |
| `?author=Qwen,bartowski&limit=3` | `[]` ✅ |
| `?author=Qwen&author=bartowski&limit=3` | `[]` ✅ |

Вывод: это не «case-insensitive login», а **точное совпадение канонического
имени**. Usernames на сайте могут быть регистронезависимыми при логине, но
фильтр `author` — нет.

### 5.2 Нормализация

У HF есть регистронезависимые overview-эндпоинты:

```http
GET https://huggingface.co/api/organizations/qwen/overview   → 200, "name": "Qwen"
GET https://huggingface.co/api/organizations/Qwen/overview   → 200, "name": "Qwen"
GET https://huggingface.co/api/users/bartowski/overview      → 200, "user": "bartowski", "fullname": "Bartowski"
GET https://huggingface.co/api/users/Bartowski/overview      → 307 → /api/users/bartowski/overview
```

Алгоритм:

```
canonicalAuthor(raw):
  for kind in ["organizations", "users"]:
    r = GET /api/{kind}/{urlencode(raw)}/overview   (follow redirects)
    if r.status == 200:
      return r.name ?? r.user
  return null
```

Кэшировать `lower(raw) -> canonical` на 7 дней. Если `null` — не делать
`author=` вслепую; показать «автор не найден» или использовать fallback.

Fallback (последняя попытка): `GET /api/models?search=<raw>&limit=100&full=true`
и фильтр `m.id.split("/")[0].toLowerCase() == raw.toLowerCase()`. Минус:
неполная выдача для плодовитых авторов и лишний трафик. Лучше — canonical
через overview.

Для UI: показывать `fullname` как display-name, а в запрос отправлять `name`
/ `user` (канон). Тогда `author=Qwen` и `author=bartowski` всегда работают.

---

## 6. CORS и архитектура запросов

### 6.1 Что отдаёт HF

Запрос `GET /api/models?limit=1` с `Origin: https://app.local` ✅:

```http
HTTP/1.1 200 OK
Access-Control-Allow-Origin: https://app.local
Vary: Origin
Access-Control-Max-Age: 86400
Access-Control-Expose-Headers: X-Repo-Commit,X-HF-Warning,X-Request-Id,X-Error-Code,
  X-Error-Message,X-Total-Count,ETag,Link,Accept-Ranges,Content-Range,
  X-Linked-Size,X-Linked-ETag,X-Xet-Hash,RateLimit,RateLimit-Policy
```

Preflight для GET с токеном ✅:

```http
OPTIONS /api/models?limit=1
Origin: https://app.local
Access-Control-Request-Method: GET
Access-Control-Request-Headers: authorization,x-hf-token

→ 200
Access-Control-Allow-Origin: https://app.local
Access-Control-Allow-Headers: authorization,x-hf-token
Access-Control-Allow-Methods: GET
```

Preflight для `paths-info` POST ✅:

```http
Access-Control-Allow-Methods: POST
Access-Control-Allow-Headers: content-type
```

`Origin: null` → `Access-Control-Allow-Origin: null`; `Origin: file://` →
`Access-Control-Allow-Origin: file://` (тоже работает, но так делать не надо).

Финальный CDN ✅:

```http
access-control-allow-origin: *
access-control-allow-headers: *
access-control-allow-methods: *
access-control-expose-headers: *
```

Даже `OPTIONS` на CDN с `Access-Control-Request-Headers: range` вернул
`allow-headers: *`. То есть **Range-запросы из JS тоже не блокируются CORS**.

### 6.2 Вывод

- Метаданные (`search`, `model`, `tree`, `paths-info`) из WebView JS на
  `https://app.local` **работают**. Это подтверждено реальными заголовками.
- Скачивание гигабайтов через `fetch` в WebView — плохая идея: память WebView,
  нельзя надёжно писать в файл, нет фонового режима, нет докачки после смерти
  процесса. CORS тут не проблема, проблема — среда.
- Рекомендуемая схема:

```
WebView (React, https://app.local)
   │ JSON через window.Sllm.* (JNI)
Rust (sllm-app / новый модуль hf)
   ├─ metadata: reqwest/ureq + rustls, кэш в SQLite
   └─ download: foreground service + Range + sha256 + .part
```

JNI-контракт в стиле существующего моста (`web/src/native.ts`):

```
window.Sllm.hfSearch(json) -> json
window.Sllm.hfModel(json)  -> json
window.Sllm.hfTree(json)   -> json
window.Sllm.hfDownloadStart(json) -> {id}
window.Sllm.hfDownloadCancel(id)
// прогресс — через существующую очередь sllmEvent / poll()
```

`ureq` (blocking, без tokio) проще для Android-сервиса; `reqwest` удобнее для
параллельных шардов и стриминга. В обоих случаях: rustls, таймауты, User-Agent,
`Accept-Encoding: gzip` для API, отключение gzip для бинарных файлов.

### 6.3 DownloadManager: когда можно

| Критерий | WebView fetch | DownloadManager | Rust + FGS |
|---|---|---|---|
| Метаданные/CORS | ✅ | ❌ | ✅ |
| Гигабайты без OOM | ❌ | ✅ | ✅ |
| Range/resume вручную | ⚠️ | ⚠️ ограниченно | ✅ |
| sha256 потоком | ❌ | ❌ | ✅ |
| Отмена/пауза | ⚠️ | ✅ | ✅ |
| Несколько шардов | ❌ | ⚠️ | ✅ |
| Фон/перезапуск | ❌ | ✅ | ✅ |
| Gated Authorization | ✅ | ✅ | ✅ |
| Прогресс/скорость | ✅ | ⚠️ через cursor | ✅ |

`DownloadManager` подходит как fallback для одного публичного файла без
проверки целостности, но не для каталога GGUF с шардами, mmproj и LoRA.
Для Android 14+ foreground service с `dataSync`; WorkManager — для ретраев,
не для самого многогигабайтного потока.

### 6.4 Офлайн-ориентированность

- При старте показывать локальный каталог (seed JSON + SQLite), сеть — вторична.
- Кэшировать: поиск (TTL 10–15 мин), детали (TTL 24 ч), tree (TTL 6–24 ч),
  `sha`/размеры (до смены `lastModified`).
- `If-None-Match` на API **не даёт 304** (проверено: вернулся 200 с тем же
  ETag). Значит, TTL + сравнение `sha`/`lastModified`, а не HTTP-кэш.
- На `/resolve/` стоит `Cache-Control: no-store` — HTTP-кэш не поможет.
- Офлайн-поиск — по локально сохранённым моделям; сетевой поиск — отдельная
  вкладка/кнопка.
- Токен — только в Rust/Keystore, не в JS и не в localStorage.

---

## 7. Оценка RAM

### 7.1 Какие поля нужны

С экрана/из API:

| Поле | Откуда |
|---|---|
| `params_total` | `safetensors.total` или `gguf.total` |
| `file_size` / `shards[]` | `tree` / `?blobs=true` |
| `quant`, `bpw` | имя файла, `general.file_type` |
| `arch` | `gguf.architecture` / header |
| `n_layers` | `*.block_count` |
| `n_attn_layers` | только `full_attention` слои (hybrid) |
| `n_kv_heads` | `*.attention.head_count_kv` |
| `k_len`, `v_len` | `*.attention.key_length`, `*.attention.value_length` |
| `ctx` | пользовательский контекст |
| `kv_bpv` | f16=2, q8_0=1.0625, q4_0=0.5625 |
| `mmproj_size` | размер mmproj |
| `moe`, `expert_count`, `expert_used` | header |
| `ternary` | TQ1_0/TQ2_0/Q2_0 |

### 7.2 Формула

```
weights = Σ size(selected shards)                  # байты
kv      = ctx * n_attn_layers * n_kv_heads
          * (k_len + v_len) * kv_bpv                # байты
compute = max(384 MiB, 0.15 * weights)             # эвристика, не замер
runtime = 150–300 MiB                              # процесс, библиотеки, арены
ram     = weights + kv + compute + runtime + mmproj_size
```

Пояснения:

- `weights` — именно размер файлов, а не `params * bpw`: у GGUF есть
  embeddings/output, которые часто в Q4_K/Q6_K даже в «2-битной» модели.
- `n_attn_layers`: у обычных dense — все слои; у hybrid (Qwen3.5/3.8,
  `qwen35`) — только `full_attention`; у гибридов с linear attention вместо KV
  есть рекуррентное состояние (SSM), его надо добавлять отдельно, точная
  формула зависит от реализации ⚠️.
- `kv_bpv`: f16 2.0; q8_0 34/32 = 1.0625; q4_0 18/32 = 0.5625.
- `compute` — грубая оценка; для MoE и больших `batch`/`ubatch` она выше.
  Для честного UI показывать «оценка» и «запас 15%».
- `mmap` **не** освобождает RAM в Android-смысле: page cache входит в PSS, LMK
  может убить процесс. Планировать по полному размеру файла.
- Для MoE: в файле лежат **все** эксперты; активная часть меньше по compute, но
  по RAM/диску — весь файл. Если приложение держит hot-expert cache, добавить
  его размер.
- Для ternary: `TQ2_0 ≈ 2.06 bpw`, `TQ1_0 ≈ 1.69 bpw`, но всё равно считать по
  размеру файла; `Q2_0` — отдельный тип (~2 bpw), не путать с TQ.
- `mmproj`: + размер файла (0.9 GB в примере MiMo) + compute на image tokens.

### 7.3 Примеры (расчёт по формуле, не замеры)

| Модель | Вес | KV @4k f16 | compute | Итог ≈ |
|---|---:|---:|---:|---:|
| Qwen2.5-0.5B Q4_K_M | 0.49 GB | 48 MiB | 384 MiB | **~1.1 GB** |
| Qwen3-4B Q4_K_M | 2.50 GB | 576 MiB | 384 MiB | **~3.5 GB** |
| Ternary-Bonsai 1.7B TQ2_0 | 0.62 GB | 448 MiB | 384 MiB | **~1.6 GB** |
| Ternary-Bonsai 1.7B TQ2_0 + KV q8 | 0.62 GB | 238 MiB | 384 MiB | **~1.4 GB** |
| MiMo 9B IQ2_M (hybrid) | 3.54 GB | 128 MiB + SSM | 506 MiB | **~4.2 GB + SSM** |
| Qwen3.8-27B Q4_0 | 16.06 GB | 256 MiB + linear state | 2.3 GB | **~18 GB** |

KV-расчёты:

- Qwen3-4B: 36 слоёв, 8 KV-голов, `key=value=128`:
  `4096 * 36 * 8 * (128+128) * 2 = 603 979 776 B = 576 MiB`.
- Qwen2.5-0.5B: 24 слоя, 2 KV-головы, 64:
  `4096 * 24 * 2 * 128 * 2 = 50 331 648 B = 48 MiB`.
- Qwen3.8-27B: 64 слоя, но full attention только 16 (interval 4), 4 KV-головы,
  256: `4096 * 16 * 4 * 512 * 2 = 268 435 456 B = 256 MiB` + linear-attention
  state (в конфиге `linear_num_value_heads=48`, `linear_value_head_dim=128`,
  `mamba_ssm_dtype=float32`) — отдельно, не входит в KV.
- Bonsai 1.7B: 28 слоёв, 8 KV-голов, 128:
  `4096 * 28 * 8 * 256 * 2 = 469 762 048 B = 448 MiB`; q8_0 — 238 MiB.
  Это совпадает с оценкой `02-ternary.md` (0.22–0.24 GB на 4k при q8_0).

Вывод для 6 GB устройства: Qwen2.5-0.5B/Qwen3-1.7B Q4 — комфортно; Qwen3-4B
Q4_K_M — только с запасом и коротким ctx; MiMo 9B — рискованно; 27B — нет.

---

## 8. Практические рекомендации для SnapdragonLLM

### 8.1 Кэш и поиск

- Debounce **300–400 мс**, минимум 2 символа, отмена предыдущего запроса
  (`AbortController` в JS или request-id в Rust).
- Кэш-ключ: `(нормализованный search, canonical author, filter[], pipeline_tag,
  sort, direction, limit)`; TTL 10–15 мин; LRU 200 записей / 2–4 МБ.
- Детали и tree кэшировать отдельно (TTL 24 ч / 6 ч), с `sha` и `lastModified`.
- При `expand=gguf`/`config` сразу удалять `chat_template` — иначе кэш
  раздувается на сотни КБ на модель.
- Пагинация: следовать `Link rel="next"`, дедуп по `id`, останавливаться при
  отсутствии `Link` или пустом массиве.
- Предзагрузка следующей страницы за 3–5 карточек до конца списка.

### 8.2 Сортировка и UI

- «Популярные» — `sort=downloads&direction=-1`.
- «Новые» — `sort=lastModified&direction=-1`.
- «Обсуждаемые/качественные» — `sort=likes`.
- «Горячее» — `sort=trendingScore`.
- «Старые сначала» — единственный asc: `sort=createdAt&direction=1`.
- Поиск без сортировки — релевантность; не отправлять `sort=relevance`.
- Показывать `downloads` и `likes` как «за период», не как «всего»;
  `downloadsAllTime` — отдельно, если включён `expand`.
- Не показывать `trendingScore` как точное число без объяснения.

### 8.3 Избранное

- Локальная таблица: `repoId`, `sha`, `file`, `quant`, `license`, `note`,
  `addedAt`; никаких API-запросов.
- Избранное не должно ломаться при обновлении репозитория: хранить `sha`, но
  предлагать «обновить до latest» отдельно.
- Для gated — хранить только ссылку, не токен и не «согласие».

### 8.4 Свободное место

- Считать `Σ shards + mmproj + LoRA`; проверять `StatFs` на целевом томе.
- Резерв: `max(1 GB, 5% тома)` + 10% на временные файлы/фрагментацию.
- Не делать копию для проверки sha256: хэшировать потоком при записи.
- Если места мало — предлагать меньший квант, а не удалять пользовательские
  данные.

### 8.5 Докачка, отмена, целостность

- Файл `.part` + sidecar JSON:
  `{url, repo, sha, path, etag, size, sha256, downloaded, updatedAt}`.
- `Range: bytes=N-`; проверять `206` + `Content-Range`; `200` → рестарт.
- При смене `X-Linked-ETag`/`sha` — рестарт, старый `.part` удалить.
- Подписанный URL истекает ≈50 мин → при 403 повторить resolve и продолжить.
- Отмена: закрыть поток, сохранить `.part`, остановить FGS; не удалять
  частичный файл без действия пользователя.
- После завершения: `size == lfs.size`; `sha256 == lfs.oid`; первые 4 байта
  `GGUF`; для split — все части на месте; для mmproj — свой sha.
- Ошибка целостности: удалить, один автоматический retry, затем показать
  «файл повреждён».

### 8.6 Безопасные имена и пути

- Имя брать из `rfilename`, не из URL/`Content-Disposition`.
- Санитизация каждого компонента: `[^A-Za-z0-9._-] -> "_"`; запретить `.`/`..`;
  убрать ведущие `/`; ограничить 100–120 символами; сохранять подкаталоги
  (`BF16/`, `MTP/`, `layouts/`).
- Локальная папка: `models/{author}__{repo}@{sha[0..7]}/` — разные версии не
  перезаписывают друг друга.
- Не следовать редиректам на произвольные хосты при скачивании: разрешить
  `huggingface.co` и официальные CDN/Xet-хосты из docs:
  `cas-server.xethub.hf.co`, `cas-server.xethub-eu.hf.co`,
  `transfer.xethub.hf.co`, `transfer.xethub-eu.hf.co`, `us.aws.cdn.hf.co`,
  `us.gcp.cdn.hf.co`, `cdn-lfs-us-1.hf.co`, `cdn-lfs-eu-1.hf.co`.
  Список меняется — сверяться с
  `https://huggingface.co/.well-known/meta.json` 📄. Иначе — ошибка.
- Проверять `Content-Type` (не сохранять HTML-страницу ошибки как `.gguf`).

### 8.7 Лицензии

- Показывать `cardData.license` + `license_link`; fallback — тег `license:*`.
- `license=other`/`gemma`/`llama3.x` — показывать ссылку и требовать явного
  подтверждения перед скачиванием.
- `gated` — отдельный бейдж; кнопка «Открыть на HF»; не автоматизировать
  принятие условий.
- Хранить снимок лицензии рядом с моделью; при обновлении `sha` — обновлять.
- Для community-квантов показывать `base_model` и то, что лицензия базовой
  модели может отличаться от лицензии репозитория.

### 8.8 Безопасность токена

- Только Rust/Android Keystore (AES-GCM), не JS, не `localStorage`, не логи.
- В WebView не включать `setAllowUniversalAccessFromFileURLs` и подобные
  послабления; грузить UI через `WebViewAssetLoader` на `https://app.local`.
- В логах/ошибках редактировать `Authorization`.
- Токен с правами `read`; не `write`.

### 8.9 Что показывать на карточке

`repoId`, `author`, `sha`, `license`, `gated`, `pipeline_tag`, `arch`,
`params`, `quant`, `fileSize`, `shards`, `mmproj`, `lora`, `layers`,
`kvHeads`, `headDim`, `ctx`, `kvQuant`, `moe`, `ternary`, `baseModel`,
`downloads`, `likes`, `lastModified`, `createdAt`, `tags`.

---

## 9. Что не удалось проверить

1. **Запросы с токеном** (free/PRO): лимиты, доступ к gated, 200 вместо 401 —
   нет аккаунта/токена. ⚠️
2. **429 и `Retry-After`** — не доводили до лимита; формат 429 не снят. ⚠️
3. **Private-репозитории** — не проверялись. ⚠️
4. **Реальный WebView fetch** — проверялись только CORS-заголовки через curl с
   `Origin`; сам WebView не запускался. ⚠️
5. **Legacy pure-LFS** (без Xet) — Range/ETag не снимались отдельно; все
   проверенные репозитории уже Xet-backed. ⚠️
6. **`X-Total-Count`** — в `/api/models` не наблюдался; возможно, отдаётся
   только для других endpoint'ов. ⚠️
7. **Формула `trendingScore`** и период `downloads` (30 дней?) — не
   документированы/не подтверждены. ⚠️
8. **`paths-info` >2000 путей** — ограничение из JSON schema, поведение при
   превышении не проверялось. ⚠️
9. **Xet-native protocol** (`xet-read-token`, `reconstructions`) — не
   реализовывали; использовали обычный HTTP Range через CDN bridge. ⚠️
10. **`gguf`-поле для всех репозиториев** — проверено на нескольких; возможны
    репозитории без него. ⚠️
11. **Точные SSM/linear-attention state** для Qwen3.5/3.8 — формула зависит от
    реализации llama.cpp; в отчёте только порядок величины. ⚠️
12. **Gated-доступ после approval** — не проверен. ⚠️

---

## 10. Краткое резюме (15 пунктов)

1. `GET /api/models` — рабочий публичный поиск; основные параметры:
   `search`, `author`, `filter`, `pipeline_tag`, `num_parameters`, `sort`,
   `direction`, `limit`, `cursor`, `full`, `expand`.
2. `author` **регистрозависим** и точен: `Qwen` ✅, `qwen` ❌; `bartowski` ✅,
   `Bartowski` ❌; `TheBloke` ✅, `thebloke` ❌. Нормализовать через
   `/api/organizations/{name}/overview` (или `/api/users/.../overview`).
3. `limit` максимум 1000; пагинация только cursor из `Link rel="next"`;
   `X-Total-Count` в `/api/models` не наблюдается.
4. Сортировки: `downloads`, `likes`, `lastModified`, `trendingScore` — только
   desc; `createdAt` — единственная с asc. `sort=relevance` не существует.
5. Минимальный список не содержит `author`/`sha`/`lastModified`/`siblings`;
   нужен `full=true` (или `expand=`). `full=false` — кворк, тоже расширяет.
6. `?blobs=true` обязателен для размеров в `siblings`; в `tree` —
   `lfs.oid`, в detail — `lfs.sha256` (одно и то же).
7. `tree/main?recursive=true` — лучший способ получить файлы, размеры, sha256,
   `xetHash`; 1000 по умолчанию, 100 при `expand=true`, cursor.
8. `paths-info` POST — до 2000 путей за раз; удобно для точечной проверки
   шардов/размеров.
9. Скачивание: `/{repo}/resolve/{sha}/{file}?download=true` → 302 на CDN;
   Range/resume (206), `X-Linked-Size`, `X-Linked-ETag` (sha256),
   `X-Xet-Hash`; `Last-Modified` нет; ссылка живёт ≈50 мин.
10. Gated: 401 + `X-Error-Code: GatedRepo`; Gemma 3/Llama 3 — `manual`;
    Gemma 4 31B — Apache-2.0 и не gated; community-GGUF Gemma 3 ungated, но
    license `gemma`.
11. CORS для WebView работает (эхо Origin, preflight разрешает
    `authorization`, CDN `ACAO: *`), но гигабайты качать из JS нельзя —
    нужен Rust JNI + foreground service.
12. **Каждый GET/HEAD на `/resolve/` считается скачиванием**; размеры брать
    из `tree`/`?blobs=true`, а не HEAD.
13. GGUF-метаданные без полной загрузки: Range 8–16 МБ + парсер заголовка;
    проверены `file_type=15` (Q4_K_M) и `37` (TQ2_0), `general.type=adapter`
    для LoRA.
14. RAM: `weights + KV + compute + mmproj + runtime`; KV считать по
    `n_attn_layers × n_kv_heads × (k_len+v_len) × ctx × bpv`; для hybrid —
    только full-attention слои + SSM state; для MoE — весь файл, для ternary —
    размер файла.
15. Практика: debounce 300–400 мс, TTL-кэш (304 не работает), canonical author,
    `.part` + sha256, проверка места, безопасные имена, лицензии и токен только
    в Keystore.

---

## 11. Ссылки

Официальные:

- Hub API (переехало в OpenAPI): <https://huggingface.co/docs/hub/api>
- OpenAPI JSON: <https://huggingface.co/.well-known/openapi.json>
- OpenAPI Markdown: <https://huggingface.co/.well-known/openapi.md>
- OpenAPI Playground: <https://huggingface.co/spaces/huggingface/openapi>
- Rate limits: <https://huggingface.co/docs/hub/rate-limits>
- Скачивание моделей: <https://huggingface.co/docs/hub/models-downloading>
- GGUF: <https://huggingface.co/docs/hub/gguf>
- Gated-модели: <https://huggingface.co/docs/hub/models-gated>
- Статистика скачиваний: <https://huggingface.co/docs/hub/models-download-stats>
- `huggingface_hub` download guide:
  <https://huggingface.co/docs/huggingface_hub/guides/download>
- `huggingface_hub` API reference (`list_models`):
  <https://huggingface.co/docs/huggingface_hub/package_reference/hf_api>
- Xet storage: <https://huggingface.co/docs/hub/xet/using-xet-storage>
- Токены: <https://huggingface.co/docs/hub/security-tokens>
- Машиночитаемый список хостов: <https://huggingface.co/.well-known/meta.json>

Живые примеры, использованные в отчёте:

- <https://huggingface.co/api/models?author=Qwen&limit=3&full=false>
- <https://huggingface.co/api/models?author=qwen&limit=3>
- <https://huggingface.co/api/models?author=bartowski&limit=2>
- <https://huggingface.co/api/models?author=TheBloke&limit=1>
- <https://huggingface.co/api/models?search=Qwen3-4B&filter=gguf&sort=downloads&direction=-1>
- <https://huggingface.co/api/models/bartowski/MiMo-V2.6-Distill-Qwen-9B-GGUF?blobs=true>
- <https://huggingface.co/api/models/bartowski/MiMo-V2.6-Distill-Qwen-9B-GGUF/tree/main?recursive=true&expand=true>
- <https://huggingface.co/api/models/unsloth/Qwen3.8-27B-GGUF?blobs=true>
- <https://huggingface.co/api/models/Qwen/Qwen3-4B-GGUF?blobs=true&full=true>
- <https://huggingface.co/api/models/ewchampion/Ternary-Bonsai-1.7B-TQ2_0-GGUF>
- <https://huggingface.co/api/organizations/qwen/overview>
- <https://huggingface.co/api/users/bartowski/overview>
- <https://huggingface.co/bartowski/MiMo-V2.6-Distill-Qwen-9B-GGUF/resolve/main/MiMo-V2.6-Distill-Qwen-9B-IQ2_M.gguf>
- <https://huggingface.co/google/gemma-3-4b-it/resolve/main/config.json> (gated → 401)
- <https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf>
- <https://huggingface.co/ewchampion/Ternary-Bonsai-1.7B-TQ2_0-GGUF/resolve/main/Ternary-Bonsai-1.7B-TQ2_0.gguf>

---

## Приложение A. Схема ответа `?blobs=true` (сокращённо)

```json
{
  "_id": "6ab1ab2a0904bf417755525e",
  "id": "bartowski/MiMo-V2.6-Distill-Qwen-9B-GGUF",
  "author": "bartowski",
  "gated": false,
  "lastModified": "2026-09-21T22:44:19.000Z",
  "likes": 186,
  "private": false,
  "sha": "4371da10c84fb26da3592d4cf312d24aa82b7b65",
  "downloads": 989262,
  "tags": [
    "gguf", "mimo_v2", "image-text-to-text",
    "base_model:XiaomiMiMo/MiMo-V2.6-Distill-Qwen-9B",
    "base_model:quantized:XiaomiMiMo/MiMo-V2.6-Distill-Qwen-9B",
    "license:apache-2.0", "region:us"
  ],
  "pipeline_tag": "image-text-to-text",
  "createdAt": "2026-09-21T22:09:46.000Z",
  "modelId": "bartowski/MiMo-V2.6-Distill-Qwen-9B-GGUF",
  "siblings": [
    {
      "rfilename": "MiMo-V2.6-Distill-Qwen-9B-IQ2_M.gguf",
      "blobId": "7e861b845b5ee53069049c9b645366f899396f45",
      "size": 3536901664,
      "lfs": {
        "sha256": "d5f703d1bf31c2d790a20e14df1e5a118b74cbf28995f60ebf734c32e2046d82",
        "size": 3536901664,
        "pointerSize": 135
      }
    }
  ],
  "cardData": {
    "quantized_by": "bartowski",
    "base_model": "XiaomiMiMo/MiMo-V2.6-Distill-Qwen-9B",
    "base_model_relation": "quantized"
  },
  "gguf": {
    "total": 8953803264,
    "architecture": "qwen35",
    "context_length": 262144,
    "totalFileSize": 17920693440
  }
}
```

## Приложение B. Заголовки, на которые можно опираться

| Заголовок | Где | Смысл |
|---|---|---|
| `Link` | API list/tree | `rel="next"` cursor |
| `X-Linked-Size` | `/resolve` 302 | размер LFS/Xet-файла |
| `X-Linked-ETag` | `/resolve` 302 | sha256 содержимого (LFS/Xet) |
| `X-Xet-Hash` | `/resolve` 302 | Xet-хеш (не sha256) |
| `X-Repo-Commit` | `/resolve` 302 | commit sha |
| `Accept-Ranges` | resolve/CDN | `bytes` |
| `Content-Range` | CDN 206 | диапазон/полный размер |
| `ETag` | CDN | Xet-хеш (для Xet-файлов), не sha256 |
| `RateLimit`, `RateLimit-Policy` | API/resolve | остаток/политика |
| `X-Error-Code` | 401 | `GatedRepo` |
| `X-HF-Warning` | API/resolve | предупреждение об анонимности |

## Приложение C. Мини-чеклист реализации

- [ ] Canonical author через `/api/organizations|users/{name}/overview`.
- [ ] `full=true`/`expand=` для нужных полей; не парсить отсутствующие поля.
- [ ] Размеры — только из `tree` или `?blobs=true`.
- [ ] `limit<=1000`, cursor из `Link`; дедуп по `id`.
- [ ] `expand=gguf` — вырезать `chat_template` перед кэшем.
- [ ] Pinned `sha` в `/resolve/{sha}/...`.
- [ ] Range + `If-Range`; проверка `206`/`Content-Range`; рестарт на `200`.
- [ ] sha256 из `lfs.oid`/`lfs.sha256`; проверка размера и magic `GGUF`.
- [ ] `.part` + sidecar; повторный resolve при истечении URL.
- [ ] Split shards: regex, все части, исходные имена.
- [ ] mmproj и LoRA — отдельно; `general.type=adapter` не модель.
- [ ] Проверка свободного места и безопасные имена.
- [ ] Токен — только Rust/Keystore.
- [ ] Debounce, TTL-кэш, офлайн-каталог.
