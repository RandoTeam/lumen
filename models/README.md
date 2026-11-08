# Модели

## Clover (основная)

| | |
| --- | --- |
| Файл | `clover/Clover-35B-A3B-Preview.APEX-I-NanoPlus-V2.1.gguf` |
| Размер | 12 557 370 304 bytes (11.69 GiB) |
| SHA-256 | `30E7EE2268128B662DC0514067FAAE3ABB0609A55F087B1AD0465EA71FE50FCE` |
| Квант | APEX-I-NanoPlus V2.1 (IsValorum) |
| База | DiffuSpace Clover-35B-A3B-Preview (Apache-2.0) |
| Профиль | `clover/expert-profile.bin` (196 632 bytes) |

Источник:
<https://huggingface.co/IsValorum/Clover-35B-A3B-Preview-APEX-I-NanoPlus-V2.1-GGUF>

> **Важно (2026-11-07):** `expert-profile.bin` — **не профиль Clover**.
> Формат `STRP` v1 содержит 48×512 = 24 576 пар, а у Clover 40×256 = 10 240.
> Это профиль другой модели (вероятно Qwen3.8-Flash-Next). Для Clover его
> использовать нельзя — нужен новый профиль 40×256 или онлайн-счётчики.
> Подробно: `docs/research/agents/03-expert-cache-io.md`.

Профиль экспертов перенесён из 4 GB проекта. Он модельный, но собран на другом железе и
другом движке; на Android его нужно проверить и пересобрать под Clover.

## Правила

- GGUF-файлы **не коммитим** в git (см. `.gitignore`); храним локально/в LFS.
- Перед запуском проверять SHA-256; битый файл не грузить.
- Не переименовывать файл без обновления хеша в документации.
- Модель не входит в APK; приложение скачивает/импортирует её отдельно.

## Проверка

```powershell
Get-FileHash -Algorithm SHA256 models\clover\Clover-35B-A3B-Preview.APEX-I-NanoPlus-V2.1.gguf
```

Ожидаемый хеш — в таблице выше.
