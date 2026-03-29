# Puddle

Linear servo drive controller.

See [docs/design.md](docs/design.md) for more information.

## Supported Hardware

* LinMot C1250-MI drive with the LinUDP interface installed

## Useful Commands

LinUDP Proxy:
```
socat -x -d -d udp4-listen:49360 udp4:192.168.10.2:49360,sourceport=41136
```

Configuration (RSTalk) Proxy:
```
socat -x -d -d udp4-listen:20000 udp4:192.168.10.2:20000
```

Serial Console:
```
picocom -b 38400 --omap spchex,tabhex,crhex,lfhex,8bithex,nrmhex --imap spchex,tabhex,crhex,lfhex,8bithex,nrmhex /dev/ttyUSB0
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
