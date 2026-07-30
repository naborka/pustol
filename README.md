# Пустол

Бронирование столов в баре из Telegram Mini App. Гость выбирает размер компании, вечер и время; бар
получает смену, которую можно вести с телефона за стойкой.

Rust 1.96, PostgreSQL 17, Node 20.9+.

```bash
scripts/pg.sh install && scripts/pg.sh start

export DATABASE_URL="$(scripts/pg.sh url)"
export TELEGRAM_BOT_TOKEN=123456:your-bot-token
export TELEGRAM_ADMIN_USERNAME=your_telegram_username

cargo run -p pustol-api --bin seed     # создаёт бар, его неделю, его зал и первого админа
cargo run -p pustol-api                # API и работник очереди, на :8080

cd web && npm install && npm run dev   # Mini App на :3000, проксирует /api на :8080
```

```bash
cargo test --workspace                 # тестам базы нужен scripts/pg.sh start
cd web && npm test
```

## Документация

Всё остальное — в книге: идея, как пользоваться каждым экраном, как запустить, API, принятые решения
и история изменений.

**<https://naborka.github.io/pustol/>**

Исходники книги — в `docs/`. Локально:

```bash
mdbook build            # пишет в target/book; откройте target/book/index.html
```
