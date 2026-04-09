# alert-distributor

Servicio de ingestion de eventos en tiempo real para un sistema distribuido basado en Kafka/Redpanda.

## Objetivo de la Fase 1

Consumir eventos desde `unit-alerts`, parsearlos y generar logging estructurado en JSON.

Alcance actual:
- Ingestion robusta con `rdkafka::StreamConsumer`
- Parseo tipado con `serde`, `uuid` y `chrono`
- Commit manual de offsets solo cuando el parseo es exitoso
- Manejo de errores sin `panic` ni `unwrap` en runtime

## Arquitectura (alto nivel)

- `src/main.rs`: bootstrap del servicio, inicializa logging y runtime async, lanza consumer.
- `src/config/mod.rs`: carga validada de variables de entorno desde `.env`.
- `src/kafka/consumer.rs`: loop de consumo, parseo de eventos, logging y commit manual.
- `src/models/alert_event.rs`: modelo de evento de alerta y tests de parsing/payload.
- `src/logging/mod.rs`: configuración de `tracing` en formato JSON.
- `src/errors/mod.rs`: errores del dominio con `thiserror`.

## Ejecución local

1. Copiar variables de entorno:

```bash
cp .env.example .env
```

2. Ejecutar el servicio:

```bash
cargo run
```

## Variables de entorno

```env
KAFKA_BROKERS=redpanda:9092
KAFKA_TOPIC=unit-alerts
KAFKA_GROUP_ID=alert-distributor
RUST_LOG=info
KAFKA_SASL_MECHANISM=SCRAM-SHA-256
KAFKA_USERNAME=siscom-live-consumer
KAFKA_PASSWORD=liveconsumerpassword
KAFKA_SECURITY_PROTOCOL=SASL_PLAINTEXT
```

Nota: la conexión a Kafka/Redpanda está configurada para `SASL_PLAINTEXT`.

## Calidad de código

Comandos definidos para control local/CI:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build
```

## Git hooks locales

Hooks configurados en `.git/hooks`:
- `pre-commit`: `cargo fmt --check` y `cargo clippy`
- `pre-push`: `cargo build` y `cargo test`

## Roadmap

### Fase 2
- Cache en memoria de reglas
- Consumo de `alert-rules-updates`

### Fase 3
- Evaluación de reglas

### Fase 4
- Distribución de alertas
