# Eventos de control en alert-distributor

El contrato de `control-plane-events` vive en siscom-admin-api:
`docs/architecture/eventos-de-control.md`, con la decisión en el ADR-008. Este
documento solo dice qué toma de ahí este servicio y cómo lo aplica.

## Qué lee

Las cinco entidades del contrato:

| Entidad | Para qué |
|---|---|
| `user` | Un usuario `INACTIVE` no recibe nada, aunque su membresía siga activa |
| `organization_user` | Rol (`owner` o no) y estado de cada membresía |
| `user_unit` | Qué unidades se concedieron a quien no es `owner` |
| `unit_device` | Qué unidades tienen equipo y de qué organización son |
| `user_device` | A qué móviles se manda el push, por persona |

Una entidad desconocida se ignora, y una `schema_version` distinta de `1` se salta
con un aviso.

## La regla de destinatarios (ADR-008 §6)

Para una unidad con equipo de la organización O:

- toda membresía `owner` de O, y
- toda otra membresía de O que tenga la unidad en `user_units`,

siempre que la membresía **y** el usuario estén `ACTIVE`. Reactivar cualquiera
de los dos devuelve el acceso. No se usa `users.is_master`: es del usuario y no
de la membresía, y daría a un maestro todas las unidades de **cada**
organización donde tenga una membresía.

El WebSocket aplica la misma regla al conectar (`units_for`).

## Cómo arranca

1. Anota el offset final de cada partición de `control-plane-events`. Si no
   puede leer el topic (no existe o falta la ACL), **el proceso no arranca**.
2. Carga la foto de las cinco vistas `control_plane.*`, no de las tablas.
3. Consume desde lo anotado en el paso 1, sin grupo y sin commit. Cada evento
   cambia solo su entrada; no se vuelve a leer la base.

## Salud

`GET /health`, en el mismo puerto del WebSocket, devuelve:

```json
{ "control_plane": { "ok": true, "applied_offset": 7, "lag": 0,
                     "users": 2, "memberships": 2, "user_units": 1,
                     "unit_devices": 0, "user_devices": 2 } }
```

Responde **503** si el consumidor no puede leer, y el `HEALTHCHECK` del
Dockerfile lo convierte en `unhealthy`. Cada 30 s se pregunta al broker por el
final del topic: eso lo recupera tras una caída sin esperar un evento, y da el
`lag`.

## Endpoints de SNS

Los errores de `Publish` se clasifican por su tipo, no por su texto:

| Error de SNS | Qué hace |
|---|---|
| `EndpointDisabled`, `NotFound`, `InvalidParameter` sobre el endpoint | Desactiva el dispositivo en `user_devices` y lo saca del estado |
| `PlatformApplicationDisabled` | Error fatal en el log; **no** desactiva a nadie (es la credencial de FCM/APNs, no el móvil) |
| Throttling, error interno, red | Reintenta |
| Cualquier otro | No reintenta y registra el código real |

La desactivación escribe directo en la base y no publica evento: este servicio
es el único que la necesita y ya actualizó su estado. Si el usuario vuelve a
abrir la app, admin-api reactiva la fila y el `UPSERT` lo devuelve aquí.

## Despliegue

- `KAFKA_CONTROL_PLANE_TOPIC` (por defecto `control-plane-events`).
- ACL de lectura sobre el topic para `alerts-distributor`, sin ACL de grupo.
- `SELECT` sobre `control_plane.*` para el usuario de base (`siscom`).
- Requiere siscom-admin-api con la migración `044_vistas_del_plano_de_control`.
- Ya no lee `user-devices-updates`, `unit-devices-updates` ni `user-units-updates`.
