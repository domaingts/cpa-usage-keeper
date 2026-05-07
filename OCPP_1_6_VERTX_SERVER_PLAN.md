# OCPP 1.6 Server Plan Using Vert.x

This plan targets an OCPP 1.6J server: JSON messages over WebSocket. Vert.x should be used as the transport/runtime layer, while the OCPP protocol parser, validator, session model, and business handlers should stay mostly framework-independent.

## Target Architecture

```text
Charge Point
  -> wss://server.example.com/ocpp/{chargePointId}
  -> Vert.x WebSocket endpoint
  -> OCPP frame parser
  -> schema validator
  -> action dispatcher
  -> business handlers
  -> database / message bus / APIs
```

Recommended module layout:

```text
transport/
  OcppWebSocketVerticle
  WebSocketAuthHandler

protocol/
  OcppMessage
  OcppCall
  OcppCallResult
  OcppCallError
  OcppCodec
  OcppSchemaValidator

session/
  ChargePointSession
  SessionRegistry
  PendingCallRegistry

handler/
  BootNotificationHandler
  HeartbeatHandler
  AuthorizeHandler
  StartTransactionHandler
  StopTransactionHandler
  StatusNotificationHandler
  MeterValuesHandler
  DataTransferHandler

command/
  CentralSystemCommandService
  RemoteStartTransactionCommand
  RemoteStopTransactionCommand
  ResetCommand

persistence/
  ChargePointRepository
  ConnectorRepository
  TransactionRepository
  MeterValueRepository
  CommandRepository
```

## 1. Protocol Scope

Start with OCPP 1.6J only.

Do not support SOAP unless a real charger integration requires it. The WebSocket endpoint should accept OCPP 1.6J clients using the standard WebSocket subprotocol:

```text
Sec-WebSocket-Protocol: ocpp1.6
```

Use URLs like:

```text
wss://server.example.com/ocpp/CP001
```

The `{chargePointId}` path segment becomes the logical charger identity, but it must not be trusted by itself. The charger should also authenticate.

## 2. Vert.x WebSocket Server

Create one Vert.x verticle responsible for WebSocket transport.

Responsibilities:

- Accept WebSocket upgrade requests.
- Extract `chargePointId` from the URL path.
- Validate the `Sec-WebSocket-Protocol` value.
- Authenticate the charge point.
- Register the connection in `SessionRegistry`.
- Attach text message, close, exception, and ping/pong handlers.
- Forward valid WebSocket text frames to the OCPP protocol layer.

Keep this verticle thin. Do not put transaction or charger business logic directly inside the WebSocket handler.

## 3. OCPP Message Codec

OCPP 1.6J messages are JSON arrays, not ordinary JSON objects.

Message shapes:

```json
[2, "message-id", "BootNotification", {}]
```

```json
[3, "message-id", {}]
```

```json
[4, "message-id", "ErrorCode", "Description", {}]
```

Build a codec that converts raw JSON text into typed Java objects:

```java
sealed interface OcppMessage permits OcppCall, OcppCallResult, OcppCallError {}
```

The codec should reject:

- Invalid JSON
- Wrong array length
- Unknown message type
- Missing action
- Non-object payload
- Malformed message ID

Invalid client calls should usually produce a proper OCPP `CALLERROR`, not only close the WebSocket.

## 4. Schema Validation

Add JSON Schema validation per action.

Validate at least these request payloads:

```text
BootNotificationRequest
HeartbeatRequest
AuthorizeRequest
StartTransactionRequest
MeterValuesRequest
StopTransactionRequest
StatusNotificationRequest
DataTransferRequest
```

Map validation failures to OCPP error codes such as:

```text
FormationViolation
PropertyConstraintViolation
TypeConstraintViolation
OccurenceConstraintViolation
```

Use the official OCPP 1.6J schema package from the Open Charge Alliance as the source of truth.

## 5. Session Management

Create a `ChargePointSession` per connected charger.

Track:

```text
chargePointId
webSocket
connectedAt
lastSeenAt
bootAccepted
heartbeatInterval
connector states
pending outbound calls
protocol version
remote address
auth identity
```

Rules to implement early:

- One active connection per `chargePointId`.
- Define a duplicate connection policy: reject the new connection, or close the old one and accept the new one.
- Remove sessions cleanly on close.
- Timeout pending server-initiated calls.
- Never block the Vert.x event loop.

## 6. Core Charger-Initiated Messages

Implement these first:

```text
BootNotification
Heartbeat
StatusNotification
Authorize
StartTransaction
MeterValues
StopTransaction
DataTransfer
```

Recommended behavior:

- `BootNotification`: validate or register the charger, then return `Accepted`, `Pending`, or `Rejected`.
- `Heartbeat`: return current server time.
- `StatusNotification`: update connector status.
- `Authorize`: check `idTag` against your authorization service.
- `StartTransaction`: create a transaction and return a `transactionId`.
- `MeterValues`: persist readings and optionally publish telemetry.
- `StopTransaction`: close the transaction and store final meter values.
- `DataTransfer`: support vendor-specific extensions later.

## 7. Server-Initiated Commands

After inbound messages work, add commands from the Central System to the charger:

```text
RemoteStartTransaction
RemoteStopTransaction
Reset
UnlockConnector
ChangeAvailability
ChangeConfiguration
GetConfiguration
TriggerMessage
ClearCache
```

Expose a command service:

```java
Future<OcppCallResult> sendCommand(
    String chargePointId,
    String action,
    JsonObject payload
);
```

Internally it should:

- Find the active session.
- Generate a unique message ID.
- Store a pending call.
- Send WebSocket text.
- Complete the future when `CALLRESULT` or `CALLERROR` arrives.
- Fail on timeout or disconnect.

## 8. Persistence Model

Minimum tables or entities:

```text
charge_point
connector
charge_point_session_log
authorization_token
transaction
meter_value
ocpp_message_log
central_command
```

Keep an `ocpp_message_log` from the beginning. It is important for charger integration debugging.

Log:

```text
chargePointId
direction: INBOUND / OUTBOUND
messageId
messageType
action
payload
timestamp
result/error
```

## 9. Security

For production, use:

```text
wss://
TLS 1.2+
Basic Auth or token auth
optional mTLS if chargers support it
per-charge-point credentials
```

Do not trust the URL identity alone. A charger connecting as `/ocpp/CP001` should also prove that it is allowed to act as `CP001`.

## 10. Vert.x Runtime Rules

Do not block the Vert.x event loop.

Use async clients or worker execution for:

```text
database writes
external HTTP calls
authorization lookups
billing system calls
firmware or diagnostics storage calls
```

Good runtime shape:

```text
WebSocket Verticle
  -> protocol parse/validate
  -> event bus or service call
  -> async DB/service
  -> response
```

## 11. Testing

Test in layers:

```text
OcppCodec unit tests
JSON schema validation tests
handler unit tests
Vert.x WebSocket integration tests
charger simulator tests
real charger interoperability tests
OCA test/certification tool tests if certification matters
```

Important negative cases:

```text
invalid JSON
unknown action
wrong message type
duplicate message ID
charger disconnect during transaction
server command timeout
duplicate chargePointId connection
BootNotification rejected or pending
MeterValues before transaction
StopTransaction for unknown transaction
```

## Recommended Build Order

1. Vert.x WebSocket endpoint
2. OCPP message codec
3. Session registry
4. BootNotification and Heartbeat
5. StatusNotification
6. Authorize
7. StartTransaction and StopTransaction
8. MeterValues
9. Message logging
10. Server-initiated commands
11. TLS and authentication hardening
12. Smart charging and advanced profiles

## Design Principle

Keep the OCPP layer as its own library inside the project:

```text
Vert.x = transport
OCPP codec = pure Java
OCPP handlers = business layer
Persistence = separate service/repository layer
```

This keeps protocol logic portable. If the system later needs Netty directly, Spring Boot, MQTT bridging, or test simulators, the OCPP implementation will not be trapped inside Vert.x handlers.
