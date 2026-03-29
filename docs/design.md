# Design

## Authorship and Process

This design and protocol specification was developed through an
iterative collaborative design process between a human engineer and an
AI assistant (GitHub Copilot, powered by Claude). The human engineer
provided all system requirements, hardware constraints, domain
expertise, and architectural decisions — including the physical
controller design, drive control semantics, safety requirements, and
operational workflows. Each design choice was reviewed, challenged,
and refined through multiple rounds of discussion before being accepted
into the specification.

The AI assistant contributed structural organisation, protocol
analysis, worked examples, and drafted prose based on the engineer's
direction. All technical content reflects decisions made or explicitly
approved by the human engineer. The document was produced through
approximately twenty rounds of review, with corrections and refinements
applied at each stage.

This disclaimer is included in the interest of transparency regarding
the use of LLM-based tooling in technical documentation.

## Table of Contents

1. [System Overview](#1-system-overview)
2. [Architecture](#2-architecture)
3. [Design Principles](#3-design-principles)
4. [Designated Writer](#4-designated-writer)
5. [Sub-Protocol Specifications](#5-sub-protocol-specifications)
    - [HID Protocol (Handheld)](#51-hid-protocol-handheld)
    - [WebSocket Protocol (Browser/CLI)](#52-websocket-protocol-browsercli)

---

## 1. System Overview

The system consists of a Rust core handling real-time servo drive
control (less than 5ms cycle time), connected to multiple external
controllers:

| Controller | Connection       | Language    | Modifiability |
|------------|------------------|-------------|---------------|
| Handheld   | USB HID / BT HID | C (ESP-IDF) | Difficult     |
| Browser    | WebSocket        | TypeScript  | Simple        |
| CLI        | WebSocket        | Rust        | Simple        |

The core holds all authoritative state and persistent storage.
Controllers are display/input endpoints that observe state and issue
commands to mutate it.

---

## 2. Architecture

```
┌────────────────────────────────────────────────────────┐
│                        RUST CORE                       │
│                                                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ Servo Drive  │  │  State Bus   │  │  Persistent  │  │
│  │ Controller   │◀▶│              │◀▶│  Storage     │  │
│  │ (<5ms cycle) │  │ Commands &   │  │              │  │
│  └──────────────┘  │ Core State   │  └──────────────┘  │
│                    │              │                    │
│         ┌─────────▶│              │◀──────────┐        │
│         │          └──────────────┘           │        │
│         │                                     │        │
│  ┌──────┴────────┐                 ┌──────────┴──────┐ │
│  │ HID UI        │                 │ WebSocket       │ │
│  │ Manager       │                 │ Server          │ │
│  │               │                 │                 │ │
│  │ Maintains     │                 │ JSON messages   │ │
│  │ screen state, │                 │ over WS         │ │
│  │ encoder       │                 │                 │ │
│  │ scaling,      │                 │                 │ │
│  │ input interp. │                 │                 │ │
│  └──────┬────────┘                 └──────────┬──────┘ │
│         │                                     │        │
└─────────┼─────────────────────────────────────┼────────┘
          │                                     │
          │ Binary HID reports                  │ WebSocket
          │ (USB or Bluetooth)                  │ (TCP)
          ▼                                     ▼
┌────────────────────┐               ┌───────────────────┐
│ Handheld           │               │ WebSocket Client  │
│ Controller         │               │ (Browser or CLI)  │
│                    │               │                   │
│ ESP-IDF C Firmare  │               │ TS (Browser) or   │
│ 4 encoders         │               │ Rust (CLI)        │
│ 2 OLED displays    │               │                   │
│ LED rings          │               │ Full config &     │
│ LVGL rendering     │               │ monitoring UI     │
└────────────────────┘               └───────────────────┘
```

### Serialization Decisions

| Protocol           | Encoding      | Rationale                                        |
|--------------------|---------------|--------------------------------------------------|
| HID                | Manual binary | Optimal density, exact byte layout, C simplicity |
| WebSocket commands | JSON          | Developer productivity, debuggability            |
| WebSocket state    | JSON          | Sufficient for 60Hz; binary upgrade path exists  |

Avro and Protobuf were evaluated and rejected. Avro's schema resolution
is poorly suited to embedded C. Protobuf (nanopb) adds build complexity
and code size without meaningful benefit given the small, stable message
set. Manual binary serialization for HID is already fully specified at
the byte level and is more compact than either alternative.

Compression was evaluated and rejected. The frequent messages
(VariableUpdate, InputReport) are already minimal, and the ScreenSpec
(100-300 bytes) is infrequent. Compression would add code complexity
and a decompression dependency in firmware for negligible bandwidth
savings.

All multi-byte integer fields in the HID protocol use **little-endian**
byte order, consistent with the USB HID specification and the other
network protocols in the system.

---

## 3. Design Principles

1. **Business logic lives in Rust.** The C firmware is a rendering and
   input terminal. The TypeScript and CLI clients are configuration and
   monitoring interfaces. Neither makes control decisions.

2. **The core is the single source of truth.** All state is
   authoritative in the core. Controllers observe and request mutations;
   they do not hold independent state.

3. **Safety by default.** Motion pauses on designated writer
   disconnection. Write access is always explicitly claimed. System
   parameter bounds are fixed in the core and not configurable at
   runtime.

4. **Protocol forward-compatibility.** The HID variable tag byte
   reserves bit 7 for an extended encoding that includes an explicit
   payload length, allowing old firmware to skip unknown variable types.
   WebSocket JSON messages naturally tolerate unknown fields.

5. **Minimise firmware changes.** New screens, parameters, and control
   schemes should require only Rust changes. Firmware changes should
   only be needed for new rendering primitives or input modalities.

---

## 4. Designated Writer

At most one controller may hold write access at any time. Write access
must be explicitly requested — it is never granted automatically.

### 4.1 Behaviour

- Any controller may request write access.
- If no controller holds it, the request is granted immediately.
- If another controller holds it, the request is denied — **unless**
  the requester is the HID controller, in which case write access is
  forcibly transferred to it. This ensures the physical operator always
  has priority control of the machine.
- Additionally, when the drive is in the **OFF** state, any controller
  may claim write access regardless of the current holder, as no
  motion is at risk.
- The holder may release write access voluntarily.
- Write access is **revoked on disconnection**.
- If the writer disconnects while motion is active, the core **pauses
  motion** as a safety measure.

### 4.2 Scope

The designated writer is required for operations that mutate the
**active motion command set** and **drive control state**:

- Active command set CRUD (insert, update, delete, reorder)
- Drive power control
- Motion state control (start, pause, resume, stop)
- Error acknowledgment
- Loading a saved command set into the active set

The following operations are available to **all controllers** regardless
of write access:

- Query state (active command set, system status)
- Receive state broadcasts
- Request and release write access
- All operations on **saved (inactive) command sets**: list, query,
  insert, update, delete, reorder, save-to, and delete-set

This separation allows a browser client to design and manage saved
motion patterns while the machine operator with the physical controller
is actively using a different pattern on the drive.

### 4.3 Controller Identification

The core assigns controller IDs:

| Controller | ID                       | Assignment                |
|------------|--------------------------|---------------------------|
| Handheld   | `"hid"`                  | Fixed, at most one        |
| WebSocket  | `"ws-N"` (e.g. `"ws-1"`) | Assigned at WS connection |

WebSocket clients receive their ID in the connection message. New IDs
are assigned on every connection (no session persistence across
reconnections). A reconnecting client must re-request write access.

---

## 5. Sub-Protocol Specifications

Detailed documentation for each transport protocol is maintained in
separate documents:

### 5.1 HID Protocol (Handheld)

Documentation for the binary HID protocol used by the physical
handheld controller.

**See: [HID Protocol Specification](hid_protocol.md)**

### 5.2 WebSocket Protocol (Browser/CLI)

Documentation for the JSON-based WebSocket API used by browser and
terminal clients, including overall drive state information.

**See: [WebSocket Protocol Specification](websocket_protocol.md)**
