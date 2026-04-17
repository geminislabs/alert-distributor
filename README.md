# alert-distributor

Servicio de ingestion de eventos en tiempo real para un sistema distribuido basado en Kafka/Redpanda.

## Objetivo de la Fase 1

Consumir eventos desde `unit-alerts`, parsearlos y generar logging estructurado en JSON.

Alcance actual:
- Ingestion robusta con `rdkafka::StreamConsumer`
- Parseo tipado con `serde`, `uuid` y `chrono`
- Commit manual de offsets solo cuando el parseo es exitoso
- Manejo de errores sin `panic` ni `unwrap` en runtime
- Distribucion en tiempo real por WebSocket (`/ws`) para web y mobile
- Autenticacion JWT RS256 local por conexion
- Snapshot de permisos en startup desde PostgreSQL (sin consultas runtime)
- Dispatch por `unit_id` con drop no bloqueante en backpressure
- Heartbeat ping/pong para desconexion de conexiones colgadas

## Arquitectura (alto nivel)

- `src/main.rs`: bootstrap del servicio, inicializa logging y runtime async, lanza consumer.
- `src/config/mod.rs`: carga validada de variables de entorno desde `.env`.
- `src/kafka/consumer.rs`: loop de consumo, parseo de eventos, logging y commit manual.
- `src/models/alert_event.rs`: modelo de evento de alerta y tests de parsing/payload.
- `src/logging/mod.rs`: configuración de `tracing` en formato JSON.
- `src/errors/mod.rs`: errores del dominio con `thiserror`.
- `src/db/postgres.rs`: conexión pool PostgreSQL para carga de snapshot.
- `src/permissions/loader.rs`: carga y construcción de cache `(organization_id, user_id) -> unit_ids`.
- `src/websocket/auth.rs`: validacion JWT RS256 y extraccion de claims.
- `src/websocket/registry.rs`: registro de conexiones e indice inverso por unidad.
- `src/websocket/dispatcher.rs`: envio de eventos a clientes por `unit_id`.
- `src/websocket/handler.rs`: endpoint `GET /ws` y ciclo de vida de conexiones.

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
DB_HOST=localhost
DB_USER=user
DB_PASSWORD=pass
DB_PORT=5432
DB_NAME=app_db
KAFKA_BROKERS=redpanda:9092
KAFKA_TOPIC=unit-alerts
KAFKA_GROUP_ID=alert-distributor
RUST_LOG=info
KAFKA_SASL_MECHANISM=SCRAM-SHA-256
KAFKA_USERNAME=siscom-live-consumer
KAFKA_PASSWORD=liveconsumerpassword
KAFKA_SECURITY_PROTOCOL=SASL_PLAINTEXT
AWS_REGION=us-east-1
WS_BIND_ADDR=0.0.0.0:8080
WS_CHANNEL_CAPACITY=1024
WS_HEARTBEAT_INTERVAL_SECS=30
WS_HEARTBEAT_TIMEOUT_SECS=60
JWT_PUBLIC_KEY_PEM=-----BEGIN PUBLIC KEY-----\\n...\\n-----END PUBLIC KEY-----
```

Nota: la conexión a Kafka/Redpanda está configurada para `SASL_PLAINTEXT`.

Notas WebSocket:
- Endpoint: `GET /ws`
- Header requerido: `Authorization: Bearer <jwt>`
- Claims esperados: `user_id` (o `sub`) y `organization_id`
- Permisos de unidades resueltos solo desde cache en memoria cargado al arranque
- Si el usuario no tiene unidades activas en cache: handshake rechazado (`403`)

## Consumo de WebSocket (detallado)

### 1. Requisitos de autenticacion

El token JWT debe:
- Estar firmado con RS256
- Incluir `user_id` (o `sub`) con UUID valido
- Incluir `organization_id` con UUID valido

El servicio valida el token localmente usando `JWT_PUBLIC_KEY_PEM`.

#### Payload del JWT (ejemplo)

```json
{
  "sub": "1de3e794-2555-4f77-9878-67fe2f934535",
  "user_id": "2a99f2d0-8d32-43c1-8894-8b7dd7a54199",
  "organization_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

Campos requeridos:
- `sub` o `user_id`: UUID del usuario (ambos válidos, el servidor acepta cualquiera)
- `organization_id`: UUID de la organización


**Para regenerar JWT válidos:**

Usa tu clave privada con Node.js o Python (código arriba en "Generar JWT válido").

El servidor validará usando `JWT_PUBLIC_KEY_PEM` (la clave pública correspondiente).

#### Generar JWT válido (Node.js)

```bash
npm i jsonwebtoken
```

```js
const jwt = require("jsonwebtoken");
const fs = require("fs");

const privateKey = fs.readFileSync("./private_key.pem");

const payload = {
  sub: "1de3e794-2555-4f77-9878-67fe2f934535",
  user_id: "2a99f2d0-8d32-43c1-8894-8b7dd7a54199",
  organization_id: "550e8400-e29b-41d4-a716-446655440000",
};

const token = jwt.sign(payload, privateKey, { algorithm: "RS256" });
console.log(token);
```

#### Generar JWT válido (Python)

```bash
pip install pyjwt
```

```python
import jwt
import json

with open("private_key.pem") as f:
    private_key = f.read()

payload = {
    "sub": "1de3e794-2555-4f77-9878-67fe2f934535",
    "user_id": "2a99f2d0-8d32-43c1-8894-8b7dd7a54199",
    "organization_id": "550e8400-e29b-41d4-a716-446655440000",
}

token = jwt.encode(payload, private_key, algorithm="RS256")
print(token)
```

#### Validar JWT (decodificar sin validar)

Para debugging, puedes decodificar el JWT sin validar la firma en [jwt.io](https://jwt.io):
1. Pega el token en el sitio
2. Verifica que el payload tiene `user_id`, `organization_id` y `sub`
3. Verifica que el algoritmo es `RS256`

Si la firma es inválida en jwt.io también, la clave privada que usaste no corresponde a tu clave pública.

### 2. Flujo de conexion

1. Cliente abre `ws://<host>:<port>/ws`
2. Envía `Authorization: Bearer <token>` en el handshake
3. Servidor valida JWT
4. Servidor busca permisos en cache en memoria `(organization_id, user_id)`
5. Si hay unidades activas, acepta la conexion
6. Cuando llega un alert por Kafka, se envía solo a clientes autorizados para ese `unit_id`

### 3. Ejemplo con wscat

Instalacion:

```bash
npm i -g wscat
```

Conexion:

```bash
wscat \
	-c ws://localhost:8080/ws \
	-H "Authorization: Bearer <JWT_AQUI>"
```

Si la conexion es exitosa, `wscat` queda escuchando mensajes entrantes.

### 4. Ejemplo con Node.js (`ws`)

```bash
npm i ws
```

```js
const WebSocket = require("ws");

const token = process.env.JWT_TOKEN;

const ws = new WebSocket("ws://localhost:8080/ws", {
	headers: {
		Authorization: `Bearer ${token}`,
	},
});

ws.on("open", () => {
	console.log("WS conectado");
});

ws.on("message", (raw) => {
	try {
		const msg = JSON.parse(raw.toString());
		console.log("alerta", msg);
	} catch (err) {
		console.error("mensaje no JSON", raw.toString());
	}
});

ws.on("close", (code, reason) => {
	console.log("WS cerrado", code, reason.toString());
});

ws.on("error", (err) => {
	console.error("WS error", err.message);
});
```

### 5. Ejemplo con Python (`websockets`)

```bash
pip install websockets
```

```python
import asyncio
import json
import os
import websockets


async def main():
		token = os.environ["JWT_TOKEN"]
		headers = [("Authorization", f"Bearer {token}")]

		async with websockets.connect("ws://localhost:8080/ws", additional_headers=headers) as ws:
				print("WS conectado")
				async for raw in ws:
						try:
								msg = json.loads(raw)
								print("alerta", msg)
						except json.JSONDecodeError:
								print("mensaje no JSON", raw)


asyncio.run(main())
```

### 6. Formato de mensaje recibido

```json
{
	"type": "alert",
	"unit_id": "33333333-3333-3333-3333-333333333333",
	"title": "Engine is OFF",
	"body": "Unit Name",
	"data": {
		"engine_status": "OFF"
	},
	"occurred_at": "2026-03-29T20:56:34Z"
}
```

Campos:
- `type`: tipo de mensaje (`alert`)
- `unit_id`: unidad origen del evento
- `title`: usa `alert_name`; si llega vacio o null, usa `alert_type`
- `body`: usa `unit_name`; si llega vacio o null, se envia `""`
- `data`: payload del evento
- `occurred_at`: timestamp UTC

### 7. Heartbeat y desconexion

- El servidor envía `Ping` periodico (`WS_HEARTBEAT_INTERVAL_SECS`)
- El cliente debe responder `Pong` (la mayoria de librerias lo hacen automaticamente)
- Si no llega `Pong` dentro de `WS_HEARTBEAT_TIMEOUT_SECS`, el servidor cierra la conexion

### 8. Errores frecuentes de handshake

- `401 Unauthorized`:
	- Falta header `Authorization`
	- Token invalido o expirado
	- Claims faltantes (`user_id`/`sub`, `organization_id`)
- `403 Forbidden`:
	- Usuario sin unidades activas en cache de permisos

### 9. Nota para navegador (WebSocket nativo)

El `WebSocket` nativo del navegador no permite setear headers personalizados como `Authorization`.
Para front web, usa una de estas opciones:
- Un backend intermedio que abra el WS con headers
- Un reverse proxy que inyecte auth
- Un ajuste del servidor para soportar token por query param (no implementado actualmente)

Formato de mensaje enviado al cliente:

```json
{
	"type": "alert",
	"unit_id": "33333333-3333-3333-3333-333333333333",
	"title": "Engine is OFF",
	"body": "Unit Name",
	"data": {"engine_status": "OFF"},
	"occurred_at": "2026-03-29T20:56:34Z"
}
```

## Ejemplos completos de salida (WebSocket y SNS)

Reglas aplicadas por el servicio:
- `title`: usa `alert_name`; si viene `null`, vacio o solo espacios, usa `alert_type`.
- `body`: usa `unit_name`; si viene `null`, vacio o solo espacios, envia `""`.

### Caso A: evento con `unit_name` y `alert_name`

Evento de entrada (Kafka):

```json
{
	"id": "7a4b6929-9f8b-4d2e-a68e-1a8e8ea4f1d3",
	"organization_id": "c24ba579-6a27-42d9-a398-0486fbe54f8c",
	"unit_id": "18961401-9405-4124-8d2a-e2c445d11e1a",
	"unit_name": "Camioneta Juan",
	"rule_id": "3b6afa2b-0f8d-4ef2-bdbf-bb20c8af9ae6",
	"source_type": "event",
	"source_id": "aaaabbbb-cccc-dddd-eeee-ffff11112222",
	"alert_type": "64f9709b-8d4c-4b2e-b1ab-44b015527ba5",
	"alert_name": "Ingreso a geocerca",
	"payload": {
		"uuid": "550e8400-e29b-41d4-a716-446655440003",
		"device_id": "device-001",
		"msg_class": "POSITION",
		"latitude": -33.8423,
		"longitude": -56.1605,
		"geofence_id": "550e8400-e29b-41d4-a716-446655440001"
	},
	"occurred_at": "2026-04-16T14:40:10Z"
}
```

Salida por WebSocket:

```json
{
	"type": "alert",
	"unit_id": "18961401-9405-4124-8d2a-e2c445d11e1a",
	"title": "Ingreso a geocerca",
	"body": "Camioneta Juan",
	"data": {
		"uuid": "550e8400-e29b-41d4-a716-446655440003",
		"device_id": "device-001",
		"msg_class": "POSITION",
		"latitude": -33.8423,
		"longitude": -56.1605,
		"geofence_id": "550e8400-e29b-41d4-a716-446655440001"
	},
	"occurred_at": "2026-04-16T14:40:10Z"
}
```

Payload SNS publicado (estructura final):

```json
{
	"default": "alert",
	"GCM": "{ \"notification\": { \"title\": \"Ingreso a geocerca\", \"body\": \"Camioneta Juan\", \"sound\": \"default\" }, \"data\": { \"message\": \"Camioneta Juan\" } }"
}
```

### Caso B: `unit_name = null` y `alert_name` vacio

Evento de entrada (Kafka):

```json
{
	"id": "c79fd4ad-81c9-4b42-a051-3a32d4a1d0e0",
	"organization_id": "d7e11a4b-017b-4799-bf66-77f0eab0f91d",
	"unit_id": "0c2d17d2-1968-4e58-95c0-c5539ae196fd",
	"unit_name": null,
	"rule_id": "417d8f6d-081d-4022-9d1a-b92b3fd3b851",
	"source_type": "event",
	"source_id": "0ef2f6a4-9a32-48da-b394-0bd0c81df0c2",
	"alert_type": "ignition_off",
	"alert_name": "   ",
	"payload": {
		"engine": "off",
		"speed": 0
	},
	"occurred_at": "2026-04-07T14:20:00Z"
}
```

Salida por WebSocket:

```json
{
	"type": "alert",
	"unit_id": "0c2d17d2-1968-4e58-95c0-c5539ae196fd",
	"title": "ignition_off",
	"body": "",
	"data": {
		"engine": "off",
		"speed": 0
	},
	"occurred_at": "2026-04-07T14:20:00Z"
}
```

Payload SNS publicado (estructura final):

```json
{
	"default": "alert",
	"GCM": "{ \"notification\": { \"title\": \"ignition_off\", \"body\": \"\", \"sound\": \"default\" }, \"data\": { \"message\": \"\" } }"
}
```

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
