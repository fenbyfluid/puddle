# LinPi

LinMot servo drive controller using the LinUDPv2 interface.

## Useful Commands

DHCP:
```
sudo dnsmasq -d --log-dhcp --bind-interfaces --interface eth0 --dhcp-range=192.168.2.100,192.168.2.200,24h --dhcp-host=00:1a:4e:xx:xx:xx,192.168.2.2```
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

