# HID Protocol Specification

This document defines the binary HID protocol used by the handheld
controller. For overall system architecture, design principles, and
writer concepts, see the **[Main Protocol Specification](design.md)**.

## Table of Contents

1. [HID UI Manager](#1-hid-ui-manager)
2. [HID Protocol](#2-hid-protocol)
3. [Connection Lifecycle](#3-connection-lifecycle)
4. [Appendix A: Variable Tag Byte Encoding](#appendix-a-variable-tag-byte-encoding)
5. [Appendix B: Worked Examples](#appendix-b-worked-examples)

---

## 1. HID UI Manager

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

### Design Principles

#### Template-Based Displays (HID)

To minimise HID bandwidth and keep C firmware thin:
- Core defines "Screen Specs" (templates).
- Core sends only variable updates (`VariableUpdate`) for dynamic data.
- Firmware handles rendering the template with updated variable values.

---

## 2. HID Protocol

### 2.1 Transport

USB 1.1 HID with 10ms report interval (configurable, may be reduced).
Maximum report payload: 63 bytes. Bluetooth HID uses the same report
descriptors and message format transparently.

The Rust core communicates via `hidraw` on Linux, treating USB and
Bluetooth HID identically.

The TinyUSB stack in ESP-IDF sends reports on request rather than
on a fixed polling schedule. Reports are sent when there is data to
transmit.

### 2.2 Report Allocation

| Direction | Report Type | ID | Message        | Frequency     |
|-----------|-------------|----|----------------|---------------|
| D→H       | Feature     | 1  | DeviceInfo     | At connection |
| H→D       | OUT         | 1  | ScreenSpec     | Event-driven  |
| H→D       | OUT         | 2  | VariableUpdate | Up to 60 Hz   |
| D→H       | IN          | 1  | InputReport    | Up to 100 Hz  |

### 2.3 Physical Layout Reference

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

### 2.4 Message Definitions

#### 2.4.1 DeviceInfo (Device → Host, Feature Report 1 via GET_REPORT)

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

#### 2.4.2 ScreenSpec (Host → Device, OUT Report 1)

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

```
Offset  Size    Field
0       varies  primary_label        Null-terminated UTF-8 string
varies  varies  secondary_label      Null-terminated UTF-8 string
```

**TextLines data:**

```
Offset  Size    Field
0       1       top_margin           Top margin in pixels
1       1       line_count           Number of lines (0-6)
2       varies  lines[]              Sequential null-terminated UTF-8 strings
```

**Menu data:**

```
Offset  Size    Field
0       1       top_margin           Top margin in pixels
1       varies  title                Null-terminated UTF-8 string
varies  1       item_count           Number of menu items (0-63)
varies  varies  items[]              Sequential MenuItem definitions
```

**MenuItem definition:**

```
Offset  Size    Field
0       1       item_id              ID reported when selected (0-63)
1       1       flags                Bit 0: enabled (1) or disabled/greyed (0)
2       varies  label                Null-terminated UTF-8 string
```

#### 2.4.3 VariableUpdate (Host → Device, OUT Report 2)

Sent periodically (up to 60Hz) or on change to update dynamic data
placeholders in the active ScreenSpec and control hardware parameters.

```
Offset  Size    Field
0       1       report_id = 0x02
1       1       count                Number of variable entries in this report
2       varies  entries[]            Sequential VariableTag entries
```

Each entry begins with a **Variable Tag Byte**, followed by a
type-specific payload.

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
| 0     | LED ring 1  | 0 = off, 1-255 = normalised position     |
| 1     | LED ring 2  | 0 = off, 1-255 = normalised position     |
| 2     | LED ring 3  | 0 = off, 1-255 = normalised position     |
| 3     | LED ring 4  | 0 = off, 1-255 = normalised position     |
| 4     | Reset       | 1 = reset device                         |
| 5     | Sleep level | 0 = screens may dim; 1 = screens may off |
| 6-31  | Reserved    | Reserved for future hardware parameters  |

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
63 80          HardwareControl idx=3, value=128 (LED ring 4 at ~50%)

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
| Sleep      | HardwareControl | 5   | 0           | 2    | dim only |
| LED E1     | HardwareControl | 0   | 128         | 2    | ~50%     |
| LED E2     | HardwareControl | 1   | 255         | 2    | ~100%    |
| LED E3     | HardwareControl | 2   | 64          | 2    | ~25%     |
| LED E4     | HardwareControl | 3   | 32          | 2    | ~12%     |

**Total:** 2 (header) + 51 (entries) = 53 bytes. Single HID report.

#### 2.4.4 InputReport (Device → Host, IN Report 1)

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

## 3. Connection Lifecycle

### 3.1 HID Connection

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

### 3.2 HID Disconnection

On disconnection (USB unplug, Bluetooth drop):

1. Core detects disconnection via OS/hidraw.
2. If the handheld was the designated writer:
   a. Write access is revoked.
   b. If motion was active, the core **pauses motion** (safety stop).
   c. All WebSocket clients receive a `write_access_changed` broadcast.
   The next `state` broadcast reflects the updated state.
3. Core stops sending HID reports.

### 3.3 HID Reconnection

On reconnection, the full connection sequence (Section 3.1) is
re-executed. The handheld starts without write access regardless of its
previous state.

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
┌──────┬──────────┬─────────────────┐
│ Bit  │  Field   │  Meaning        │
├──────┼──────────┼─────────────────┤
│ 7    │ Extended │ 1 = extended    │
│ 6    │ RESERVED │ Must be 0       │
│ 5    │ RESERVED │ Must be 0       │
│ 4:0  │ TYPE     │ Type code (0-31)│
└──────┴──────────┴─────────────────┘
```

Followed by:
- **Index:** u8 (0-255)
- **Length:** u8 (for variable length types)
- **Payload**

Extended format is reserved for future use when the compact index or
type space is exhausted.

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
       { idx 0, HardwareControl: 140 },      LED ring 1
       { idx 1, HardwareControl: 255 },      LED ring 2
     ]
   }

7. Firmware renders updated values into template
   placeholders and redraws affected display regions.
   LED rings update to new positions.
```

### B.3 Hold-Modifier Mode Change

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

### B.4 Menu Navigation on Left Display

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

8a. Core processes MenuSelect: display 0, item 1 ->
    load "fast_cycle_v2". Core sends new ScreenSpec
    returning to main control screen.
```
