# Puddle

Linear servo drive controller.

## Supported Hardware

* LinMot C1250-MI drive with the LinUDP interface installed

## Useful Commands

DHCP:
```
sudo dnsmasq -d --log-dhcp --bind-interfaces --interface eth0 --dhcp-range=192.168.2.100,192.168.2.200,24h --dhcp-host=00:1a:4e:xx:xx:xx,192.168.2.2
```

LinUDP Proxy:
```
socat -x -d -d udp4-listen:49360 udp4:192.168.2.2:49360,sourceport=41136
```

Configuration (RSTalk) Proxy:
```
socat -x -d -d udp4-listen:20000 udp4:192.168.2.2:20000
```

Serial Console:
```
picocom -b 38400 --omap spchex,tabhex,crhex,lfhex,8bithex,nrmhex --imap spchex,tabhex,crhex,lfhex,8bithex,nrmhex /dev/ttyAMA0
```

## Interesting UPIDs
| UPID | Name                    | Notes                        |
|------|-------------------------|------------------------------|
| 1BF3 | Max Read Out Motor Temp | Actual motor temp            |
| 1C00 | Min. Motor Temp Reserve | 0 = Motor too hot            |
| 1BCD | Temp Core               | Drive CPU core temp          |
| 1BCE | Max Drive Temp          | Drive sensor temp            |
| 1BDD | Motor Power Losses      | Motor heat production        |
| 1E0A | Target Position         | Motion final target position |
| 1E0B | Max Velocity            | Motion max velocity          |
| 1E0C | Acceleration            | Motion acceleration          |
| 1E0D | Deceleration            | Motion deceleration          |
| 1E0E | VAI Position            | VAI demand position          |
| 1E0F | VAI Velocity            | VAI demand velocity          |
| 1E10 | VAI Acceleration        | VAI demand acceleration      |

## TODO

* Redesign the HID interface for two-way communication
* ~~Drive connection testing and retry~~
* WebSocket interface
* ~~TSDB integration (InfluxDB Core 3? Prometheus? QuestDB?) for metrics storage (positions, amps, volts, temps?)~~
  * Add any UPIDs we want to track reliably to the LinUDP protocol list, up to 4
  * InfluxBD 3 has known issues with data loss after power outage
  * Prometheus can't backfill recent data, so needs to scrape at our request rate (5ms default), unclear how it'll handle gaps
  * QuestDB may not have these issues?
  * VictoriaMetrics?
* SQLite database for persistent session storage
* ~~We had a case where the drive moved the slider out of bounds despite our commands, investigate further if this happens again once we have telemetry, no hints in the log~~
* ~~Configure drive current limits and slider bounds for safety (manually)~~
* Option to check drive configuration hash and reconfigure everything?
