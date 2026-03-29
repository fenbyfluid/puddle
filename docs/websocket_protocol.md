# WebSocket Protocol Specification

This document defines the JSON-based WebSocket API used by browser and
terminal clients. For overall system architecture, design principles,
and writer concepts, see the **[Main Protocol Specification](design.md)**.

## Table of Contents

1. [Drive State Information](#1-drive-state-information)
2. [WebSocket Protocol](#2-websocket-protocol)
3. [Connection Lifecycle](#3-connection-lifecycle)
4. [Appendix: Worked Examples](#appendix-worked-examples)

---

## 1. Drive State Information

### 1.1 Control State

The core manages an ordered list of **motion commands** executed
sequentially. Each motion command consists of:

| Field        | Type | Description       |
|--------------|------|-------------------|
| position     | i32  | Target position   |
| velocity     | i32  | Maximum velocity  |
| acceleration | i32  | Acceleration rate |
| deceleration | i32  | Deceleration rate |

### 1.2 Drive State

The system transitions through the following **drive states**:

```
                       ┌─────────────┐
              ┌───────▶│     OFF     │◀───────┐
              │        └──────┬──────┘        │
              │               │               │
         Power Off        Power On        Acknowledge
              │               │               │
              │               ▼               │
              │       ┌───────────────┐       │
              │       │   PREPARING   │       │
              │       └───────┬───────┘       │
              │               │               │
              │             Ready             │
              │               │               │
              │               ▼               │
              │        ┌─────────────┐        │
              ├────────│   PAUSED    │        │
              │        └──┬───────┬──┘        │
              │           │       ▲           │
              │        Start   Pause/Stop     │
              │           │       │           │
              │           ▼       │           │
              │       ┌──────────────┐        │
              ├───────│    MOVING    │        │
              │       └──────┬───────┘        │
              │              │                │
              │         Fault/Error           │
              │              │                │
              │              ▼                │
              │       ┌──────────────┐        │
              └───────│   ERRORED    │────────┘
                      └──────────────┘
```

- **DISCONNECTED**: Drive not connected.
- **OFF**: Drive powered down.
- **PREPARING**: Drive powering up, performing initialisation.
- **PAUSED**: Drive ready, motion not active. Commands may be edited.
- **MOVING**: Executing the motion command list in a loop.
- **ERRORED**: A fault has occurred. Requires acknowledgment to return
  to OFF.

**Pause** decelerates to standstill using the current command's
deceleration and holds position. Resuming continues from the point of
interruption within the active command.

**Stop** decelerates to standstill and resets to the beginning of the
command set. The next start begins from command index 0.

### 1.3 Core State

| Field                | Update Rate | Description                           |
|----------------------|-------------|---------------------------------------|
| drive_state          | On change   | Current drive state enum              |
| active_command_index | Per cycle   | Currently executing command           |
| actual_position      | Per cycle   | Measured position                     |
| demand_position      | Per cycle   | Desired position                      |
| demand_velocity      | Per cycle   | Desired velocity                      |
| demand_aceleration   | Per cycle   | Desired acceleration                  |
| current_draw         | Per cycle   | Motor current                         |
| drive_temperature    | Per cycle   | Drive temperature                     |
| motor_temperature    | Per cycle   | Motor temperature                     |
| warnings             | On change   | Active warnings                       |
| error_code           | On change   | Error identifier (ERRORED state only) |
| command_set_version  | On change   | Monotonically increasing version      |
| write_access_holder  | On change   | Controller ID or null                 |

---

## 2. WebSocket Protocol

### 2.1 Transport

Standard WebSocket over TCP. Binary frames are not used; all messages
are JSON-encoded text frames.

Single WebSocket connection per client (browser or terminal). Message type
discrimination via the `type` field.

### 2.2 Commands (Client → Core)

#### 2.2.1 `request_write_access`

Request to become the Designated Writer.

```json
{
  "type": "request_write_access",
  "seq": 1
}
```

**Response:** `write_access_result`.

#### 2.2.2 `release_write_access`

Relinquish Designated Writer status.

```json
{
  "type": "release_write_access",
  "seq": 2
}
```

**Response:** `write_access_result`.

#### 2.2.3 `get_command_set`

Retrieve either the active motion command set or a named saved set.

```json
{
  "type": "get_command_set",
  "seq": 3,
  "set": null // null for active set, or "string_name" for saved set
}
```

**Response:** `command_set`.

#### 2.2.4 `upsert_command_set`

Create or update a command set. If `set` is `null`, updates the active
set. This requires write access.

```json
{
  "type": "upsert_command_set",
  "seq": 4,
  "set": null, // null for active, or "string_name"
  "base_version": 17, // optional, for optimistic concurrency
  "commands": [
    {
      "position": 100000,
      "velocity": 50000,
      "acceleration": 10000,
      "deceleration": 10000
    }
  ]
}
```

**Response:** `command_result`.

#### 2.2.5 `delete_saved_set`

Delete a named saved set.

```json
{
  "type": "delete_saved_set",
  "seq": 5,
  "set": "string_name"
}
```

**Response:** `command_result`.

#### 2.2.6 `list_saved_sets`

Retrieve a list of all saved command sets (metadata only).

```json
{
  "type": "list_saved_sets",
  "seq": 6
}
```

**Response:** `saved_set_list`.

#### 2.2.7 `set_drive_power`

Enable or disable drive power. Requires write access.

```json
{
  "type": "set_drive_power",
  "seq": 7,
  "enabled": true
}
```

**Response:** `command_result`.

#### 2.2.8 `acknowledge_error`

Acknowledge a drive fault and return to OFF state. Requires write access.

```json
{
  "type": "acknowledge_error",
  "seq": 8
}
```

**Response:** `command_result`.

#### 2.2.9 `motion_control`

Start, pause, or stop motion. Requires write access.

```json
{
  "type": "motion_control",
  "seq": 9,
  "action": "start" // "start", "pause", "stop"
}
```

**Response:** `command_result`.

### 2.3 Responses and Broadcasts (Core → Client)

#### 2.3.1 `connected` (Response)

Sent immediately upon WebSocket connection.

```json
{
  "type": "connected",
  "controller_id": "ws-1",
  "write_access_holder": "hid"
}
```

#### 2.3.2 `write_access_result` (Response)

Response to `request_write_access` or `release_write_access`.

```json
{
  "type": "write_access_result",
  "seq": 1,
  "granted": true,
  "holder": "ws-1",
  "reason": null // null or "busy"
}
```

#### 2.3.3 `command_set` (Response)

Contains the requested command set data.

```json
{
  "type": "command_set",
  "seq": 3,
  "set": null,
  "version": 17,
  "commands": [ ... ]
}
```

#### 2.3.4 `command_result` (Response)

Generic success/failure response for mutation commands.

```json
{
  "type": "command_result",
  "seq": 4,
  "success": true,
  "version": 18, // current version after operation
  "error": null // error message if success is false
}
```

#### 2.3.5 `saved_set_list` (Response)

List of available saved sets.

```json
{
  "type": "saved_set_list",
  "seq": 6,
  "sets": [
    { "name": "pattern_a", "version": 2, "saved_at": "..." },
    { "name": "pattern_b", "version": 5, "saved_at": "..." }
  ]
}
```

#### 2.3.6 `state` (Broadcast)

Periodic full state update (e.g., 10-60Hz).

```json
{
  "type": "state",
  "drive_state": "moving",
  "active_command_index": 0,
  "actual_position": 124950,
  "demand_position": 125000,
  "demand_velocity": 50000,
  "demand_acceleration": 10000,
  "current_draw": 450,
  "drive_temperature": 42,
  "motor_temperature": 38,
  "warnings": [],
  "error_code": null,
  "command_set_version": 18,
  "write_access_holder": "ws-1"
}
```

#### 2.3.7 `write_access_changed` (Broadcast)

Sent when any controller gains or loses write access.

```json
{
  "type": "write_access_changed",
  "holder": "hid"
}
```

#### 2.3.8 `command_set_changed` (Broadcast)

Sent when the active command set is modified.

```json
{
  "type": "command_set_changed",
  "version": 19
}
```

---

## 3. Connection Lifecycle

### 3.1 WebSocket Connection

```
   Client                                 Core
     │                                      │
     ├───── WebSocket handshake ───────────▶│
     │                                      │
     │◀──── connected { controller_id,      │
     │       write_access_holder } ─────────┤
     │                                      │
     ├───── get_command_set ───────────────▶│
     │◀──── command_set { ... } ────────────┤
     │                                      │
     │◀──── state (broadcast) ──────────────┤
     │◀──── Normal operation ──────────────▶│
     │                                      │
```

### 3.2 WebSocket Disconnection

On disconnection (TCP close, network failure):

1. Core detects disconnection via WebSocket close or TCP timeout.
2. If the client was the designated writer:
   a. Write access is revoked.
   b. If motion was active, the core **pauses motion**.
   c. All remaining clients receive `write_access_changed`. The next
   `state` broadcast reflects the updated state.
3. The controller ID (e.g. `"ws-1"`) becomes available for
   reassignment.

### 3.3 WebSocket Reconnection

A new WebSocket connection is treated as a fresh session. A new
controller ID is assigned (which may be the same string if the ID is
available). No state is carried over from the previous session. The
client must re-request write access if needed.

---

## Appendix: Worked Examples

### A.1 WebSocket Client Replaces Active Command Set

```
1. Client sends to Core:
   { "type": "upsert_command_set", "seq": 20,
     "set": null,
     "commands": [
       {
         "position": 125000,
         "velocity": 50000,
         "acceleration": 10000,
         "deceleration": 10000
       },
       {
         "position": 180000,
         "velocity": 50000,
         "acceleration": 10000,
         "deceleration": 10000
       },
       {
         "position": 250000,
         "velocity": 50000,
         "acceleration": 10000,
         "deceleration": 10000
       }
     ]
   }

2. Core validates:
   - set is null (active set) -> requires writer.
   - Client is designated writer? Yes.
   - No base_version provided -> last-write-wins.

3. Core replaces active command set.
   Version increments to 18.

4. Core responds to requesting client:
   { "type": "command_result", "seq": 20,
     "success": true, "version": 18 }

5. Core broadcasts to all WebSocket clients:
   { "type": "command_set_changed",
     "version": 18 }

6. Core HID UI manager notes command set change.
   If the handheld is on a screen showing command set
   info, updates relevant variables via VariableUpdate.
```

### A.2 WebSocket Client Edits Saved Set While Operator Uses Drive

```
1. Operator (handheld, designated writer) is running
   motion using active command set version 17.

2. Client (ws-1, not writer) lists saved sets:
   -> { "type": "list_saved_sets", "seq": 30 }
   <- { "type": "saved_set_list", "seq": 30,
        "sets": [
          { "name": "new_pattern", "version": 2,
            "saved_at": "2026-03-20T14:30:00Z" }
        ]
      }

3. Client retrieves the saved set:
   -> { "type": "get_command_set", "seq": 31,
        "set": "new_pattern" }
   <- { "type": "command_set", "seq": 31,
        "set": "new_pattern", "version": 2,
        "commands": [ ... 5 commands ... ]
      }

4. Client edits locally, then upserts the saved set
   (no writer required; base_version for conflict
   detection):
   -> { "type": "upsert_command_set", "seq": 32,
        "set": "new_pattern", "base_version": 2,
        "commands": [ ... 5 modified commands ... ] }
   <- { "type": "command_result", "seq": 32,
        "success": true, "version": 3 }

5. Active motion continues uninterrupted throughout.
   The operator's drive control is not affected.

6. Later, when the operator is ready, they pause motion.
   The WebSocket client requests write access and copies
   the saved set into the active set:
   -> { "type": "request_write_access", "seq": 33 }
   <- { "type": "write_access_result", "seq": 33,
        "granted": true, "holder": "ws-1" }
   -> { "type": "get_command_set", "seq": 34,
        "set": "new_pattern" }
   <- { "type": "command_set", "seq": 34,
        "set": "new_pattern", "version": 3,
        "commands": [ ... ] }
   -> { "type": "upsert_command_set", "seq": 35,
        "set": null,
        "commands": [ ... same commands ... ] }
   <- { "type": "command_result", "seq": 35,
        "success": true, "version": 18 }
```
