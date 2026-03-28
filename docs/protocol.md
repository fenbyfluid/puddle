# Protocol Design

## Authorship and Process

This protocol specification was developed through an iterative
collaborative design process between a human engineer and an AI
assistant (GitHub Copilot, powered by Claude). The human engineer
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
5. [HID Protocol](#5-hid-protocol)
6. [WebSocket Protocol](#6-websocket-protocol)
7. [Connection Lifecycle](#7-connection-lifecycle)
8. [Appendix A: Variable Tag Byte Encoding](#appendix-a-variable-tag-byte-encoding)
9. [Appendix B: Worked Examples](#appendix-b-worked-examples)

---

## 1. System Overview

The system consists of a Rust core handling real-time servo drive
control (less than 5ms cycle time), connected to up to three external
controllers:

| Controller | Connection       | Language     | Modifiability |
|------------|------------------|--------------|---------------|
| Handheld   | USB HID / BT HID | C (ESP32-S3) | Difficult     |
| Browser    | WebSocket (×1-2) | TypeScript   | Simple        |

The core holds all authoritative state and persistent storage.
Controllers are display/input endpoints that observe state and issue
commands to mutate it.

### Control State

The core manages an ordered list of **motion commands** executed
sequentially. Each motion command consists of:

| Field        | Type | Description       |
|--------------|------|-------------------|
| position     | i32  | Target position   |
| velocity     | i32  | Maximum velocity  |
| acceleration | i32  | Acceleration rate |
| deceleration | i32  | Deceleration rate |

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

### Core State

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
│ Handheld           │               │ Browser Client    │
│ Controller         │               │ (×1-2)            │
│                    │               │                   │
│ ESP32-S3           │               │ TypeScript        │
│ 4 encoders         │               │ Full config &     │
│ 2 OLED displays    │               │ monitoring UI     │
│ LED rings          │               │                   │
│ LVGL rendering     │               │                   │
└────────────────────┘               └───────────────────┘
```

### HID UI Manager

The HID UI manager is an internal module within the Rust core. It
consumes the same state bus as the WebSocket server but maintains its
own screen state model for the handheld controller. It is responsible
for:

- Defining screen layouts (templates with placeholders)
- Computing display values from raw state (formatting, scaling, unit
  conversion)
- Processing raw encoder deltas through scaling functions (e.g.,
  quadratic/cubic)
- Enforcing parameter bounds and cross-parameter constraints
- Interpreting button events in context (what does "click encoder 1"
  mean on this screen?)
- Translating input into state bus commands

This keeps all business logic in Rust while the C firmware handles
only:

- Rendering text and menus from templates and variable values
- Driving LED rings from position values
- Reporting raw encoder deltas and hardware-detected button events
- Local menu navigation via LVGL's input device system

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
   input terminal. The TypeScript client is a configuration and
   monitoring interface. Neither makes control decisions.

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

### Behaviour

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

### Scope

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

### Controller Identification

The core assigns controller IDs:

| Controller | ID       | Assignment                |
|------------|----------|---------------------------|
| Handheld   | `"hid"`  | Fixed, at most one        |
| Browser 1  | `"ws-1"` | Assigned at WS connection |
| Browser 2  | `"ws-2"` | Assigned at WS connection |

WebSocket clients receive their ID in the connection message. New IDs
are assigned on every connection (no session persistence across
reconnects). A reconnecting client must re-request write access.

---

## 5. HID Protocol

### 5.1 Transport

USB 1.1 HID with 10ms report interval (configurable, may be reduced).
Maximum report payload: 63 bytes. Bluetooth HID uses the same report
descriptors and message format transparently.

The Rust core communicates via `hidraw` on Linux, treating USB and
Bluetooth HID identically.

The TinyUSB stack on the ESP32-S3 sends reports on request rather than
on a fixed polling schedule. Reports are sent when there is data to
transmit.

### 5.2 Report Allocation

| Direction | Report Type | ID | Message        | Frequency     |
|-----------|-------------|----|----------------|---------------|
| D→H       | Feature     | 1  | DeviceInfo     | At connection |
| H→D       | OUT         | 1  | ScreenSpec     | Event-driven  |
| H→D       | OUT         | 2  | VariableUpdate | Up to 60 Hz   |
| D→H       | IN          | 1  | InputReport    | Up to 100 Hz  |

### 5.3 Physical Layout Reference

```
┌──────────────────────┐    ┌──────────────────────┐
│                      │    │                      │
│   Left Display       │    │   Right Display      │
│   (3 lines main)     │    │   (3 lines main)     │
│                      │    │                      │
│ E1 Press │ E2 Press  │    │ E3 Press │ E4 Press  │
│ E1 Param │ E2 Param  │    │ E3 Param │ E4 Param  │
└──────────────────────┘    └──────────────────────┘
    [E1]       [E2]             [E3]       [E4]
  encoder    encoder          encoder    encoder
  + button   + button         + button   + button
  + LEDs     + LEDs           + LEDs     + LEDs
```

Each display: approximately 20 characters × 5 lines (3 main content +
2 encoder labels).

Menu navigation: Encoder 1 navigates left-display menus, Encoder 2
cancels. Encoder 4 navigates right-display menus, Encoder 3 cancels.
This mapping is fixed in firmware based on the physical layout and does
not need to be communicated in the protocol.

### 5.4 Message Definitions

#### 5.4.1 DeviceInfo (Device → Host, Feature Report 1 via GET_REPORT)

Read by the core at connection via GET_REPORT. This is the first
message in the connection handshake.

```
Offset  Size  Field
0       1     report_id = 0x01
1       1     firmware_version_major
2       1     firmware_version_minor
3       1     features              Reserved bitmask (0x00)
```

4 bytes total. The core uses the firmware version to determine
protocol compatibility. The features bitmask is reserved for
future capability negotiation.

#### 5.4.2 ScreenSpec (Host → Device, OUT Report 1)

Sent on screen or mode transitions. Defines the content layout for both
displays and encoder labels.

**Fragmentation header** (per HID OUT report):

```
Offset  Size  Field
0       1     report_id = 0x01
1       1     screen_id            Unique ID for this screen configuration
2       1     frag_total           Total fragment count
3       1     frag_index           0-based fragment index
4       60    payload              Fragment data
```

The firmware buffers fragments and reassembles when all fragments for a
given `screen_id` have been received. If a different `screen_id` arrives
while fragments are being buffered, the partial buffer is discarded and
the new screen begins accumulating. If a `screen_id` arrives that
matches the already-active screen, the firmware treats it as a
retransmission and reprocesses it.

On receipt of a complete ScreenSpec, the firmware resets all local UI
state: menu selection indices reset to 0, placeholder segments are
rebuilt from templates, and variable values are cleared pending the next
VariableUpdate. The firmware reports the processed `screen_id` as
`active_screen_id` in subsequent InputReports. The core uses a mismatch
between the expected and reported `screen_id` to detect lost messages
and retransmit.

**Reassembled payload structure:**

```
Offset  Size    Field
0       varies  encoder_labels[4]    All 4 encoder label pairs
varies  1       left_main_type       0 = TextLines, 1 = Menu
varies  varies  (left main area data, depending on type)
varies  1       right_main_type      0 = TextLines, 1 = Menu
varies  varies  (right main area data, depending on type)
```

**Encoder labels** (always 4 entries, in encoder order):

Each encoder label consists of two sequential null-terminated template
strings:

```
Offset  Size    Field
0       varies  primary_label       Template: "{5}", "Power", or "" (hidden)
varies  varies  secondary_label     Template: "{6}", "Start Pos", or ""
```

The primary label is displayed closest to the encoder, and the
secondary label is displayed above it if present. This allows more
space for screen content if there is no secondary label specified.

The firmware implicitly determines encoder routing from the display
main area types:

- If left display is Menu: encoder 1 rotation and click are routed to
  LVGL for menu navigation; encoder 2 click generates a MenuCancel
  event. Neither encoder sends deltas to the core.
- If right display is Menu: encoder 4 rotation and click are routed to
  LVGL for menu navigation; encoder 3 click generates a MenuCancel
  event. Neither encoder sends deltas to the core.
- If a display is TextLines: its associated encoders send raw deltas
  and button events to the core normally.

**TextLines main area:**

```
Offset  Size    Field
0       1       line_count           Typically 1-3
1       varies  lines[]              Sequential null-terminated template strings
```

**Menu main area:**

```
Offset  Size    Field
0       varies  title_template       Null-terminated UTF-8 template
varies  1       item_count           Number of menu items
varies  varies  items[]              Sequential menu items
```

Each menu item:

```
Offset  Size    Field
0       1       item_id              Returned in MenuSelect event (bits 5:0)
1       1       enabled              0 = disabled (greyed), 1 = enabled
2       varies  label_template       Null-terminated UTF-8 template
```

Menu item IDs must be in the range 0-63 (6 bits). See Section 5.4.4
InputReport for encoding details.

**Template format:** Plain UTF-8 text with `{N}` placeholders where N
is a decimal variable index 0-31. The firmware parses these once on
ScreenSpec receipt, breaking each template into a list of static-text
and placeholder segments for efficient rendering on variable updates.

**Estimated assembled sizes:**

| Screen Type            | Estimated Size | Fragments | Transfer Time |
|------------------------|----------------|-----------|---------------|
| Main control (6 lines) | 80-120 bytes   | 2-3       | 20-30ms       |
| Menu (10 items)        | 150-200 bytes  | 3-4       | 30-40ms       |
| Simple status          | 60-80 bytes    | 2         | 20ms          |

#### 5.4.3 VariableUpdate (Host → Device, OUT Report 2)

The hot-path message. Carries current values for display placeholders
and hardware control parameters.

Sent at up to 60Hz during active operation. Sparse: only variables
that have changed since the last update need be included, though the
core may choose to send full updates at a lower rate for robustness.

```
Offset  Size    Field
0       1       report_id = 0x02
1       1       count                Number of variable entries
2       varies  entries[]            Sequential variable entries
```

Each variable entry consists of a tag byte followed by a type-specific
payload. See [Appendix A](#appendix-a-variable-tag-byte-encoding) for
full encoding details, including the extended format for
forward-compatible future types.

**Compact format** (bit 7 = 0):

```
┌──────┬──────────┬─────────────────┐
│ Bit  │  Field   │  Meaning        │
├──────┼──────────┼─────────────────┤
│ 7    │ Extended │ 0 = compact     │
│ 6:5  │ TYPE     │ Type code (0-3) │
│ 4:0  │ INDEX    │ Index (0-31)    │
└──────┴──────────┴─────────────────┘
```

**Type 0 — FixedPoint (4 bytes total):**

```
Offset  Size  Field
0       1     tag                 TYP = 00
1       1     decimals            Number of decimal places for display
2       1     value_lo            i16 value, low byte (little-endian)
3       1     value_hi            i16 value, high byte
```

The firmware renders the value as a decimal number: a value of 1250
with decimals=1 displays as "125.0". A value of -42 with decimals=0
displays as "-42".

The `decimals` field precedes the value to allow the i16 to be read
as an aligned 2-byte little-endian integer when entries are contiguous
and no variable-length types precede it.

i16 range (±32767) is sufficient as all displayed values are pre-scaled
by the core from the native i32 representation to 3-4 significant
figures.

**Type 1 — ShortString (variable length):**

```
Offset  Size    Field
0       1       tag               TYP = 01
1       varies  data              Null-terminated UTF-8 string
```

Used for all string display variables: state names, warning text,
error messages, dynamic labels, and any other text content. The
firmware scans for the null terminator to determine the string extent.

**Type 2 — Reserved:**

Reserved for future use. Parsers must not encounter this type in the
current protocol version.

**Type 3 — HardwareControl (2 bytes total):**

```
Offset  Size  Field
0       1     tag                 TYP = 11
1       1     value               Parameter value (meaning depends on index)
```

Controls hardware state on the device. The index in the tag byte
identifies which hardware parameter is being set. HardwareControl
indices occupy their own namespace, separate from display variable
indices used by types 0-1.

**HardwareControl index definitions:**

| Index | Parameter   | Values                                   |
|-------|-------------|------------------------------------------|
| 0     | Sleep level | 0 = screens may dim; 1 = screens may off |
| 1     | LED ring 1  | 0 = off, 1-255 = normalised position     |
| 2     | LED ring 2  | 0 = off, 1-255 = normalised position     |
| 3     | LED ring 3  | 0 = off, 1-255 = normalised position     |
| 4     | LED ring 4  | 0 = off, 1-255 = normalised position     |
| 5-31  | Reserved    | Reserved for future hardware parameters  |

LED ring values: 0 turns all LEDs off. Values 1-255 map linearly to
the physical LED count on the ring.

Sleep level: the firmware manages idle detection and sleep transitions
locally. The core sets the permitted sleep depth. At level 0, screens
may dim after an idle timeout but remain readable. At level 1, screens
may turn off completely after a longer idle timeout. Only local user
input (encoder rotation, button press) resets the idle timer — incoming
VariableUpdate messages do not wake the display, as they are continuous
during drive operation and should not prevent sleep. Additional sleep
levels may be defined in the future.

**Example — steady-state motion (2 variables changed):**

```
Hex: 02 02 06 01 D4 30 63 80

02             report_id = 0x02
02             count = 2
06 01 D430     FixedPoint idx=6, decimals=1, value=12500 (LE) → "1250.0"
63 80          HardwareControl idx=3, value=128 (LED ring 3 at ~50%)

Total: 8 bytes
```

**Example — full refresh (8 display values + sleep + 4 LEDs):**

| Entry      | Type            | Idx | Payload     | Size | Display  |
|------------|-----------------|-----|-------------|------|----------|
| Start pos  | FixedPoint      | 0   | dec=1, 1250 | 4    | "125.0"  |
| End pos    | FixedPoint      | 1   | dec=1, 2500 | 4    | "250.0"  |
| Velocity   | FixedPoint      | 2   | dec=0, 5000 | 4    | "5000"   |
| Accel      | FixedPoint      | 3   | dec=0, 1000 | 4    | "1000"   |
| State      | ShortString     | 4   | "MOVING"\0  | 8    | "MOVING" |
| Target pos | FixedPoint      | 5   | dec=1, 1250 | 4    | "125.0"  |
| Actual pos | FixedPoint      | 6   | dec=1, 1249 | 4    | "124.9"  |
| Warnings   | ShortString     | 7   | "OC, OT"\0  | 9    | "OC, OT" |
| Sleep      | HardwareControl | 0   | 0           | 2    | dim only |
| LED E1     | HardwareControl | 1   | 128         | 2    | ~50%     |
| LED E2     | HardwareControl | 2   | 255         | 2    | ~100%    |
| LED E3     | HardwareControl | 3   | 64          | 2    | ~25%     |
| LED E4     | HardwareControl | 4   | 32          | 2    | ~12%     |

**Total:** 2 (header) + 51 (entries) = 53 bytes. Single HID report.

#### 5.4.4 InputReport (Device → Host, IN Report 1)

Sent by the firmware when there is input data to report. At idle, the
firmware NAKs the USB poll (no data). When any encoder moves or any
event occurs, the report is sent on the next poll.

```
Offset  Size    Field
0       1       report_id = 0x01
1       1       active_screen_id     Last processed ScreenSpec screen_id
2       1       encoder_delta_1      i8, raw delta since last report
3       1       encoder_delta_2      i8
4       1       encoder_delta_3      i8
5       1       encoder_delta_4      i8
6       1       event_count          Number of input events (0-28)
7       varies  events[]             Sequential 2-byte events
```

Encoder deltas are signed 8-bit values representing accumulated ticks
since the last report. The encoder hardware accumulates ticks internally
and resets to zero on read by the firmware. At the LVGL polling rate of
33Hz, the ±127 range per report is far more than a physical encoder
produces.

When a menu is active on a display, the associated encoders are
consumed locally by LVGL:

- Left display menu active: encoder 1 drives menu scrolling/selection
  via LVGL, encoder 2 click generates MenuCancel. Both report
  delta = 0.
- Right display menu active: encoder 4 drives menu scrolling/selection
  via LVGL, encoder 3 click generates MenuCancel. Both report
  delta = 0.

`active_screen_id` serves as an implicit acknowledgment of the last
received ScreenSpec. The core uses this to detect if a ScreenSpec was
not processed (e.g., after a dropped fragment) and retransmit if
necessary.

**Input event encoding:**

```
Offset  Size  Field
0       1     event_type
1       1     event_data
```

| Type | Name              | Data                  | Description                   |
|------|-------------------|-----------------------|-------------------------------|
| 0x00 | ButtonClick       | encoder_id (0-3)      | Single click                  |
| 0x01 | ButtonDoubleClick | encoder_id (0-3)      | Double click                  |
| 0x02 | ButtonHoldStart   | encoder_id (0-3)      | Hold gesture started          |
| 0x03 | ButtonHoldEnd     | encoder_id (0-3)      | Hold gesture ended (released) |
| 0x04 | MenuSelect        | display_id:item_id    | Menu item selected            |
| 0x05 | MenuCancel        | display_id (bits 7:6) | Menu cancelled                |

**MenuSelect event_data encoding:**

```
┌──────┬───────────────┐
│ Bit  │  Field        │
├──────┼───────────────┤
│ 7:6  │ display_id    │
│ 5:0  │ item_id       │
└──────┴───────────────┘
```

`display_id`: 0 = left display, 1 = right display. Values 2-3
reserved for future expansion.

`item_id`: the `item_id` from the selected MenuItem definition
(0-63).

**MenuCancel event_data encoding:**

```
┌──────┬───────────────┐
│ Bit  │  Field        │
├──────┼───────────────┤
│ 7:6  │ display_id    │
│ 5:0  │ reserved (0)  │
└──────┴───────────────┘
```

ButtonClick and ButtonDoubleClick are detected by the encoder
hardware's built-in gesture recognition. ButtonHoldStart and
ButtonHoldEnd are generated by the firmware based on the hardware's
press duration reporting.

The core's HID UI manager interprets button events contextually based
on the active screen. For example, ButtonClick on encoder 1 might
toggle drive power on the main control screen, but the same physical
action is consumed locally as a menu selection when a menu is active on
the left display. Write access requests and releases are handled by the
core mapping a specific button event on a specific screen to the
appropriate state bus command — no dedicated HID event types are needed.

**Maximum report size:** 7 (fixed header) + 28 × 2 (max events) =
63 bytes. In practice, 0-2 events per report is typical, giving a
total of 7-11 bytes.

---

## 6. WebSocket Protocol

### 6.1 Transport

Standard WebSocket over TCP. Binary frames are not used; all messages
are JSON-encoded text frames.

Single WebSocket connection per browser client. Message type
discrimination via the `type` field.

### 6.2 Message Framing

All messages follow this structure:

**Client → Core (requests):**

```json
{ "type": "message_type", "seq": 42, ... }
```

**Core → Client (responses):**

```json
{ "type": "message_type", "seq": 42, ... }
```

**Core → Client (broadcasts):**

```json
{ "type": "message_type", ... }
```

- `seq`: Present on client-originated messages. Responses echo the
  same `seq`. Broadcasts omit `seq` entirely. Clients distinguish
  broadcasts from responses by the presence or absence of `seq`.
- Unknown fields in any message must be silently ignored by the
  receiver (forward-compatibility).

### 6.3 Message Catalog

#### Client → Core

| Type                   | Description                        | Requires Writer |
|------------------------|------------------------------------|-----------------|
| `request_write_access` | Claim designated writer            | No              |
| `release_write_access` | Release designated writer          | Yes             |
| `get_state`            | Query current system state         | No              |
| `get_command_set`      | Get commands in a set              | No              |
| `upsert_command_set`   | Replace entire contents of a set   | If active set   |
| `update_command`       | Modify fields in a single command  | Yes             |
| `delete_command_set`   | Clear active / delete saved set    | If active set   |
| `list_saved_sets`      | List available saved sets          | No              |
| `set_drive_power`      | Power on/off                       | Yes             |
| `set_motion_state`     | Start/pause/resume/stop motion     | Yes             |
| `acknowledge_error`    | Clear error state                  | Yes             |

#### Core → Client

| Type                   | Trigger    | Description                          |
|------------------------|------------|--------------------------------------|
| `connected`            | Connection | Controller ID, limits, initial state |
| `ack`                  | Response   | Success/failure with optional reason |
| `write_access_result`  | Response   | Access granted/denied with holder    |
| `command_set`          | Response   | Motion command list with version     |
| `command_result`       | Response   | Mutation result with version         |
| `saved_set_list`       | Response   | Array of saved set metadata          |
| `state`                | Broadcast  | Full system state at up to 60 Hz     |
| `command_set_changed`  | Broadcast  | Active command set was modified      |
| `write_access_changed` | Broadcast  | Designated writer changed            |

### 6.4 Message Definitions

#### 6.4.1 Connection

**`connected`** (Core → Client, on WebSocket open):

```json
{
    "type": "connected",
    "controller_id": "ws-1",
    "limits": {
        "position": 500000,
        "velocity": 100000,
        "acceleration": 50000,
        "deceleration": 50000
    },
    "state": {
        "drive_state": "paused",
        "active_command_index": 0,
        "actual_position": 0,
        "demand_position": 0,
        "demand_velocity": 0,
        "demand_acceleration": 0,
        "current_draw": 0,
        "drive_temperature": 2300,
        "motor_temperature": 2300,
        "warnings": [],
        "error_code": null,
        "command_set_version": 17,
        "write_access_holder": "hid"
    }
}
```

`limits` contains the system parameter bounds. All values are in the
same internal units used throughout the protocol. Position, velocity, 
acceleration, and deceleration are bounded from 0 to their respective
maximums. These limits are fixed for the lifetime of the server.

`state` contains a complete state snapshot, identical in structure to
the `state` message body (without `type` or `seq`). The client should
follow up with `get_command_set` to populate its command set UI if
required.

#### 6.4.2 Write Access

**`request_write_access`:**

```json
{ "type": "request_write_access", "seq": 1 }
```

**`release_write_access`:**

```json
{ "type": "release_write_access", "seq": 2 }
```

**`write_access_result`** (response):

```json
{
    "type": "write_access_result",
    "seq": 1,
    "granted": true,
    "holder": "ws-1"
}
```

```json
{
    "type": "write_access_result",
    "seq": 1,
    "granted": false,
    "holder": "hid"
}
```

When write access is forcibly transferred (HID priority claim, or any
claim while in OFF state), the previous holder receives a
`write_access_changed` broadcast.

**`write_access_changed`** (broadcast to all clients):

```json
{
    "type": "write_access_changed",
    "holder": "ws-1",
    "previous_holder": "hid"
}
```

`holder` is `null` when write access is released or revoked.

#### 6.4.3 System State

**`get_state`:**

```json
{ "type": "get_state", "seq": 3 }
```

**`state`** (response to `get_state`, and broadcast at up to 1 kHz):

When used as a response (with `seq`), provides the current state
snapshot. As a broadcast (without `seq`), provides continuous real-time
telemetry. The structure is identical in both cases.

```json
{
    "type": "state",
    "seq": 3,
    "drive_state": "moving",
    "active_command_index": 1,
    "actual_position": 248750,
    "demand_position": 250000,
    "demand_velocity": 49800,
    "demand_acceleration": 0,
    "current_draw": 2150,
    "drive_temperature": 4523,
    "motor_temperature": 4523,
    "warnings": [],
    "error_code": null,
    "command_set_version": 18,
    "write_access_holder": "hid"
}
```

All numeric values are raw internal integers. The browser client
handles display formatting.

`command_set_version` allows the client to detect staleness of its
cached command set without a separate notification.

`error_code` is `null` unless `drive_state` is `"errored"`.
`warnings` provides resolved names for warning flags.

`write_access_holder` is included for convenience so that a
`get_state` response or any single `state` broadcast provides a
complete system snapshot. However, `write_access_changed` is the
authoritative notification for write access transitions — commands
submitted after that broadcast may fail.

Broadcast continuously at up to 60Hz while at least one WebSocket
client is connected. The core may reduce the broadcast rate if no
clients are connected. Because the state message always contains the
complete state, all state changes (drive transitions, warning changes,
error entry, write access changes) are reflected within one broadcast
cycle (≤16ms) without requiring separate event messages.

#### 6.4.4 Command Set Operations

All command set operations identify the target set via a `set` field:

- `"set": null` — the **active** command set (what the drive executes).
  Mutations require designated writer status.
- `"set": "name"` — a **saved** command set identified by name.
  Mutations are available to all controllers.

Both active and saved sets are versioned. Every mutation increments the
set's version.

**`get_command_set`:**

```json
{ "type": "get_command_set", "seq": 4, "set": null }
```

```json
{ "type": "get_command_set", "seq": 5, "set": "fast_cycle_v2" }
```

**`command_set`** (response):

```json
{
    "type": "command_set",
    "seq": 4,
    "set": null,
    "version": 17,
    "commands": [
        {
            "position": 125000,
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
```

Commands are ordered by array position. There is no explicit index
field — the position in the `commands` array defines the execution
order.

If the requested saved set does not exist, the core responds with
`ack`:

```json
{
    "type": "ack",
    "seq": 5,
    "success": false,
    "reason": "not_found"
}
```

**`upsert_command_set`:**

```json
{
    "type": "upsert_command_set",
    "seq": 6,
    "set": null,
    "base_version": 17,
    "commands": [
        {
            "position": 130000,
            "velocity": 60000,
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
```

Replaces the entire contents of the specified command set. All
commands must be provided in full.

`base_version` is optional. If present, the core checks it against the
current version and returns a version conflict if they do not match.
If absent, the upsert is applied unconditionally (last-write-wins).

For saved sets: if the named set does not exist, it is created. If it
exists, it is replaced. `base_version` enables conflict detection when
multiple clients may be editing the same saved set.

For the active set: requires designated writer status.

Responds with `command_result` on success or version conflict, or
`ack` on pre-condition failure (not_writer).

**`update_command`:**

```json
{
    "type": "update_command",
    "seq": 7,
    "index": 0,
    "fields": {
        "position": 130000
    }
}
```

Modifies fields of a single command in the **active** command set.
Only fields present in `fields` are modified; omitted fields retain
their current values. Requires designated writer status.

This is an optimisation for the interactive control path, where
individual parameters are adjusted frequently (e.g., via encoder input
or browser sliders). The result is identical to an `upsert_command_set`
with the single field changed.

There is no `set` field — this message always targets the active set.
There is no `base_version` — the designated writer is the only client
that can mutate the active set, so version conflicts cannot occur.

Responds with `command_result` on success, or `ack` on pre-condition
failure (not_writer, index out of range).

**`delete_command_set`:**

```json
{
    "type": "delete_command_set",
    "seq": 8,
    "set": "old_config"
}
```

```json
{
    "type": "delete_command_set",
    "seq": 9,
    "set": "old_config",
    "base_version": 4
}
```

```json
{
    "type": "delete_command_set",
    "seq": 10,
    "set": null
}
```

For saved sets: deletes the named set. `base_version` is optional —
if present, the core checks it against the current version and returns
a version conflict if they do not match. Responds with `ack` on
success or not found, or `command_result` on version conflict.

```json
{
    "type": "ack",
    "seq": 8,
    "success": true
}
```

```json
{
    "type": "ack",
    "seq": 8,
    "success": false,
    "reason": "not_found"
}
```

```json
{
    "type": "command_result",
    "seq": 9,
    "success": false,
    "version": 5
}
```

For the active set: clears all commands (empty set). Requires
designated writer status. The version increments. Responds with
`command_result` on success, or `ack` on pre-condition failure.

```json
{
    "type": "command_result",
    "seq": 10,
    "success": true,
    "version": 19
}
```

```json
{
    "type": "ack",
    "seq": 10,
    "success": false,
    "reason": "not_writer"
}
```

**`command_result`** (response to successful mutations and version
conflicts):

`version` is always present: on success it is the new version after
the mutation; on failure it is the current version the client is
behind on. A failed `command_result` always means version conflict.

Success:

```json
{
    "type": "command_result",
    "seq": 6,
    "success": true,
    "version": 18
}
```

Version conflict:

```json
{
    "type": "command_result",
    "seq": 6,
    "success": false,
    "version": 22
}
```

Pre-condition failures (not designated writer, set not found, index
out of range) are returned as `ack`:

```json
{
    "type": "ack",
    "seq": 7,
    "success": false,
    "reason": "not_writer"
}
```

```json
{
    "type": "ack",
    "seq": 8,
    "success": false,
    "reason": "not_found"
}
```

#### 6.4.5 Saved Set Management

**`list_saved_sets`:**

```json
{ "type": "list_saved_sets", "seq": 10 }
```

**`saved_set_list`** (response):

```json
{
    "type": "saved_set_list",
    "seq": 10,
    "sets": [
        {
            "name": "fast_cycle_v2",
            "version": 4,
            "saved_at": "2026-03-20T14:30:00Z"
        },
        {
            "name": "default",
            "version": 1,
            "saved_at": "2026-03-15T09:00:00Z"
        }
    ]
}
```

#### 6.4.6 Drive Control

**`set_drive_power`:**

```json
{
    "type": "set_drive_power",
    "seq": 14,
    "enabled": true
}
```

Transitions the drive between OFF and PREPARING (power on) or from any
non-error state to OFF (power off). Power off while moving causes an
uncontrolled stop.

**`set_motion_state`:**

```json
{
    "type": "set_motion_state",
    "seq": 15,
    "action": "start"
}
```

| Action     | From State    | To State | Behaviour                               |
|------------|---------------|----------|-----------------------------------------|
| `"start"`  | PAUSED        | MOVING   | Begin executing from command index 0    |
| `"pause"`  | MOVING        | PAUSED   | Decelerate to standstill, hold position |
| `"resume"` | PAUSED        | MOVING   | Continue from point of interruption     |
| `"stop"`   | MOVING/PAUSED | PAUSED   | Decelerate (if moving), reset to cmd 0  |

**`acknowledge_error`:**

```json
{ "type": "acknowledge_error", "seq": 16 }
```

Transitions from ERRORED to OFF. The operator must then power on and
restart.

All drive control commands respond with `ack`.

#### 6.4.7 Broadcasts

**`state`** — see Section 6.4.3. Broadcast at up to 60Hz with
complete system state and telemetry.

**`command_set_changed`:**

Broadcast when the **active** command set is modified.

When the modification was a single-command field update (via
`update_command`), the broadcast includes the index and changed fields,
allowing clients to apply the delta to their cached command set without
re-fetching:

```json
{
    "type": "command_set_changed",
    "version": 19,
    "index": 0,
    "fields": { "position": 130000 }
}
```

For all other modifications (`upsert_command_set`, `delete_command_set`
on the active set), the broadcast contains only the new version.
Clients compare against their cached version and re-fetch with
`get_command_set` if stale:

```json
{
    "type": "command_set_changed",
    "version": 19
}
```

Saved set modifications do not generate broadcasts. Clients managing
saved sets are expected to track their own local state.

**`write_access_changed`** — see Section 6.4.2. Broadcast when the
designated writer changes.

---

## 7. Connection Lifecycle

### 7.1 HID Connection

```
    Core                                     Handheld
     │                                           │
     ├───── (detect HID device via OS) ─────────▶│
     │                                           │
     │◀──── GET_REPORT: DeviceInfo ──────────────┤
     │                                           │
     ├───── ScreenSpec (current screen) ────────▶│
     │                                           │
     ├───── VariableUpdate (full initial) ──────▶│
     │                                           │
     │◀──── InputReport (active_screen_id) ──────┤
     │      (implicit ScreenSpec ACK)            │
     │                                           │
     │◀──── Normal operation ───────────────────▶│
     │                                           │
```

### 7.2 HID Disconnection

On disconnection (USB unplug, Bluetooth drop):

1. Core detects disconnection via OS/hidraw.
2. If the handheld was the designated writer:
   a. Write access is revoked.
   b. If motion was active, the core **pauses motion** (safety stop).
   c. All WebSocket clients receive a `write_access_changed` broadcast.
   The next `state` broadcast reflects the updated state.
3. Core stops sending HID reports.

### 7.3 HID Reconnection

On reconnection, the full connection sequence (Section 7.1) is
re-executed. The handheld starts without write access regardless of its
previous state.

### 7.4 WebSocket Connection

```
    Browser                               Core
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

### 7.5 WebSocket Disconnection

On disconnection (TCP close, network failure):

1. Core detects disconnection via WebSocket close or TCP timeout.
2. If the client was the designated writer:
   a. Write access is revoked.
   b. If motion was active, the core **pauses motion**.
   c. All remaining clients receive `write_access_changed`. The next
   `state` broadcast reflects the updated state.
3. The controller ID (`"ws-1"` or `"ws-2"`) becomes available for
   reassignment.

### 7.6 WebSocket Reconnection

A new WebSocket connection is treated as a fresh session. A new
controller ID is assigned (which may be the same string if the slot is
available). No state is carried over from the previous session. The
client must re-request write access if needed.

---

## Appendix A: Variable Tag Byte Encoding

### Compact Format (bit 7 = 0)

```
┌──────┬──────────┬─────────────────┐
│ Bit  │  Field   │  Meaning        │
├──────┼──────────┼─────────────────┤
│ 7    │ Extended │ 0 = compact     │
│ 6:5  │ TYPE     │ Type code (0-3) │
│ 4:0  │ INDEX    │ Index (0-31)    │
└──────┴──────────┴─────────────────┘
```

- **Bit 7:** 0 indicates compact format.
- **Bits 6:5 (TYPE):** Variable type code.
- **Bits 4:0 (INDEX):** Variable index. For display types (0-1), this
  corresponds to `{N}` in templates. For HardwareControl (3), this
  indexes the hardware parameter table.

**Type codes:**

| TYP | Name            | Payload                          | Total Size |
|-----|-----------------|----------------------------------|------------|
| 00  | FixedPoint      | u8 decimals + i16 LE (3 bytes)   | 4 bytes    |
| 01  | ShortString     | null-terminated UTF-8 (varies)   | 2+ bytes   |
| 10  | Reserved        | —                                | —          |
| 11  | HardwareControl | u8 value (1 byte)                | 2 bytes    |

### Extended Format (bit 7 = 1)

```
┌──────────┬──────────┬──────────┬───────────────┐
│ Byte 0   │ Byte 1   │ Byte 2   │ Byte 3+       │
├──────────┼──────────┼──────────┼───────────────┤
│ 1_ttttttt│ index    │ length   │ payload ...   │
└──────────┴──────────┴──────────┴───────────────┘
```

- **Byte 0, bit 7:** 1 indicates extended format.
- **Byte 0, bits 6:0:** Extended type code (0-127).
- **Byte 1:** Variable index (0-255).
- **Byte 2:** Payload length in bytes (0-255).
- **Bytes 3+:** Payload data (`length` bytes).

The extended format allows future variable types to be added without
breaking existing firmware. A parser that does not recognise the
extended type code reads the `length` byte and skips `length` bytes of
payload to advance to the next entry. This guarantees
forward-compatibility: old firmware ignores new variable types
gracefully.

The extended format provides 128 type codes, 256 indices, and payloads
up to 255 bytes — substantially more capacity than the compact format.
No extended types are defined in this version of the protocol.

### Parsing Algorithm

```
offset = 0
for i in 0..count:
    tag = buffer[offset]

    if (tag & 0x80) == 0:
        // Compact format
        typ = (tag >> 5) & 0x03
        idx = tag & 0x1F
        offset += 1

        switch typ:
            case 0 (FixedPoint):
                decimals = buffer[offset]
                value = buffer[offset+1] | (buffer[offset+2] << 8)  // i16 LE
                offset += 3

            case 1 (ShortString):
                start = offset
                while buffer[offset] != 0x00:
                    offset += 1
                string = buffer[start .. offset]
                offset += 1  // skip null terminator

            case 2 (Reserved):
                // Must not appear in current protocol version.
                // Future parsers handle via extended format.
                error("unexpected reserved type")

            case 3 (HardwareControl):
                value = buffer[offset]
                offset += 1

        update_variable(idx, typ, ...)

    else:
        // Extended format — skip unknown types
        ext_type = tag & 0x7F
        idx = buffer[offset]
        length = buffer[offset + 1]
        offset += 2

        if ext_type is known:
            process payload at buffer[offset .. offset+length]

        offset += length
```

### Index Allocation Convention

Display variable indices (types 0-1) and HardwareControl indices
(type 3) occupy separate namespaces. A typical allocation for the main
control screen:

**Display variables (types 0-1):**

| Index | Usage          | Type        | Display              |
|-------|----------------|-------------|----------------------|
| 0     | Start position | FixedPoint  | Left display line 1  |
| 1     | End position   | FixedPoint  | Left display line 1  |
| 2     | Velocity       | FixedPoint  | Left display line 2  |
| 3     | Acceleration   | FixedPoint  | Left display line 3  |
| 4     | Drive state    | ShortString | Right display line 1 |
| 5     | Target pos     | FixedPoint  | Right display line 2 |
| 6     | Actual pos     | FixedPoint  | Right display line 2 |
| 7     | Warnings/error | ShortString | Right display line 3 |
| 8-15  | (available)    | —           | Labels, future use   |

**HardwareControl variables (type 3):**

| Index | Parameter   | Description                      |
|-------|-------------|----------------------------------|
| 0     | Sleep level | 0 = dim allowed, 1 = off allowed |
| 1     | LED ring 1  | Encoder 1 LED ring position      |
| 2     | LED ring 2  | Encoder 2 LED ring position      |
| 3     | LED ring 3  | Encoder 3 LED ring position      |
| 4     | LED ring 4  | Encoder 4 LED ring position      |
| 5-31  | (available) | Future hardware parameters       |

---

## Appendix B: Worked Examples

### B.1 Operator Powers On Drive via Handheld

```
1. User clicks encoder 1 button on main control screen.

2. Firmware sends InputReport to Core:
   InputReport {
     active_screen_id: 1,
     encoder_deltas: [0, 0, 0, 0],
     event_count: 1,
     events: [ButtonClick { encoder_id: 0 }]
   }

3. Core HID UI manager interprets:
   Screen 1, encoder 0 click = "toggle power".
   Checks: handheld is designated writer? If not, ignore
   (or update display to show read-only indicator).
   If writer: issue set_drive_power(enabled: true) on
   state bus.

4. Drive state transitions: OFF -> PREPARING.

5. Core broadcasts state to all WebSocket clients at on a
   cycle. The state message includes drive_state: "preparing"
   and all other current state.

6. Core HID UI manager updates variable for drive state.
   Core sends VariableUpdate to Handheld:
   VariableUpdate {
     count: 1,
     entries: [
       { index: 4, ShortString: "PREPARING\0" }
     ]
   }

7. Drive completes initialisation.
   State transitions: PREPARING -> PAUSED.

8. Next state broadcast includes drive_state: "paused".

9. Core sends VariableUpdate to Handheld:
   VariableUpdate {
     count: 1,
     entries: [
       { index: 4, ShortString: "PAUSED\0" }
     ]
   }
```

### B.2 Operator Adjusts Start Position via Encoder

```
1. User turns encoder 1 two detents clockwise.

2. LVGL polls encoder at 33Hz, reads accumulated delta
   of +2 from hardware, resets accumulator to 0.

3. Firmware sends InputReport to Core:
   InputReport {
     active_screen_id: 1,
     encoder_deltas: [2, 0, 0, 0],
     event_count: 0
   }

4. Core HID UI manager processes:
   - Encoder 0 is in Raw mode, mapped to start_position.
   - Applies scaling: delta=2,
     change = 2 * abs(2) * 10000 = 40000
   - new_start_position = old + 40000
   - Clamps to [min, max]
   - Checks constraint: if start > end, reduces end
   - Computes display value: start_display = start / 10000
   - Computes LED ring positions from normalised ranges

5. Core issues state bus command to update motion command
   parameters.

6. Core sends VariableUpdate to Handheld:
   VariableUpdate {
     count: 4,
     entries: [
       { idx 0, FixedPoint: dec=1, 165 },    "16.5"
       { idx 1, FixedPoint: dec=1, 250 },    "25.0"
       { idx 1, HardwareControl: 140 },      LED ring 1
       { idx 2, HardwareControl: 255 },      LED ring 2
     ]
   }

7. Firmware renders updated values into template
   placeholders and redraws affected display regions.
   LED rings update to new positions.
```

### B.3 Browser Client Replaces Active Command Set

```
1. Browser sends to Core:
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

### B.4 Hold-Modifier Mode Change

```
1. User presses and holds encoder 1 button.

2. Firmware detects hold start via encoder hardware.
   Firmware sends InputReport to Core:
   InputReport {
     ...,
     events: [ButtonHoldStart { encoder_id: 0 }]
   }

3. Core HID UI manager interprets:
   Screen 1, encoder 0 hold start = "enter modifier
   mode: remap encoders 3+4 to fwd/rev velocity".

4. Core sends VariableUpdate to Handheld with updated
   label variables:
   VariableUpdate {
     count: 2,
     entries: [
       { idx 14, ShortString: "Fwd Vel\0" },
       { idx 15, ShortString: "Rev Vel\0" },
     ]
   }
   (Encoder 3 and 4 parameter labels were defined as
   placeholders {14} and {15} in the ScreenSpec, allowing
   the mode change without a new ScreenSpec.)

5. While held, encoder 3 deltas are interpreted as forward
   velocity adjustment, encoder 4 deltas as reverse
   velocity adjustment. Display values and LED rings
   update accordingly via normal VariableUpdate messages.

6. User releases encoder 1 button.
   Firmware sends InputReport to Core:
   InputReport {
     ...,
     events: [ButtonHoldEnd { encoder_id: 0 }]
   }

7. Core sends VariableUpdate restoring original label
   variables:
   VariableUpdate {
     count: 2,
     entries: [
       { idx 14, ShortString: "Velocity\0" },
       { idx 15, ShortString: "Accel\0" },
     ]
   }
```

### B.5 Browser Client Edits Saved Set While Operator Uses Drive

```
1. Operator (handheld, designated writer) is running
   motion using active command set version 17.

2. Browser client (ws-1, not writer) lists saved sets:
   -> { "type": "list_saved_sets", "seq": 30 }
   <- { "type": "saved_set_list", "seq": 30,
        "sets": [
          { "name": "new_pattern", "version": 2,
            "saved_at": "2026-03-20T14:30:00Z" }
        ]
      }

3. Browser retrieves the saved set:
   -> { "type": "get_command_set", "seq": 31,
        "set": "new_pattern" }
   <- { "type": "command_set", "seq": 31,
        "set": "new_pattern", "version": 2,
        "commands": [ ... 5 commands ... ]
      }

4. Browser edits locally, then upserts the saved set
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
   The browser client requests write access and copies
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

### B.6 Menu Navigation on Left Display

```
1. User clicks encoder 3 button (mapped to "Menu" action
   on the main control screen).

2. Firmware sends InputReport to Core:
   InputReport {
     ...,
     events: [ButtonClick { encoder_id: 2 }]
   }

3. Core HID UI manager interprets:
   Screen 1, encoder 2 click = "open menu".
   Core sends ScreenSpec to Handheld:
   ScreenSpec {
     screen_id: 2,
     encoder_labels: [
       { primary: "Select\0", secondary: "\0" },
       { primary: "Cancel\0", secondary: "\0" },
       { primary: "\0",       secondary: "\0" },
       { primary: "\0",       secondary: "\0" },
     ],
     left_main: Menu {
       title: "Load Program\0",
       items: [
         { item_id: 1, enabled: 1,
           label: "fast_cycle_v2\0" },
         { item_id: 2, enabled: 1,
           label: "default\0" },
         { item_id: 3, enabled: 0,
           label: "backup (empty)\0" },
       ]
     },
     right_main: TextLines {
       ... same status display ...
     }
   }

4. Firmware processes ScreenSpec:
   - Left display type is Menu -> encoder 1 routes to
     LVGL menu navigation, encoder 2 click will generate
     MenuCancel.
   - Right display type is TextLines -> encoders 3 and 4
     send raw deltas normally.
   - Renders menu on left display, status on right.

5. Core continues sending VariableUpdates for right
   display values (actual position, state, etc.) at 60Hz.
   Left display is static menu content.

6. User rotates encoder 1 to scroll through menu items.
   LVGL handles this locally — no InputReport sent.

7a. User clicks encoder 1 to select "fast_cycle_v2".
    Firmware sends InputReport to Core:
    InputReport {
      ...,
      events: [MenuSelect {
        display_id: 0, item_id: 1
      }]
    }
    event_data byte: 0b00_000001 = 0x01

8a. Core processes MenuSelect: display 0, item 1 ->
    load "fast_cycle_v2". Core sends new ScreenSpec
    returning to main control screen.

7b. Alternatively, user clicks encoder 2 to cancel.
    Firmware sends InputReport to Core:
    InputReport {
      ...,
      events: [MenuCancel { display_id: 0 }]
    }
    event_data byte: 0b00_000000 = 0x00

8b. Core sends ScreenSpec returning to previous screen.
```
