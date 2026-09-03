#!/bin/bash
set -e

# Generate wrdp.ini if it doesn't exist
mkdir -p /etc/wrdp
if [ ! -f /etc/wrdp/wrdp.ini ]; then
  cat << 'INI' > /etc/wrdp/wrdp.ini
[server]
listen_addr = "192.168.4.1:3389"
max_connections = 1
session_timeout = 3600
tls_cert = "/etc/wrdp/cert.pem"
tls_key = "/etc/wrdp/key.pem"
[display]
allow_resize = true
allowed_resolutions = []
INI
fi

# Generate TLS certificate
if [ ! -f /etc/wrdp/key.pem ]; then
  openssl req -x509 -newkey rsa:4096 -nodes \
    -keyout /etc/wrdp/key.pem -out /etc/wrdp/cert.pem \
    -days 365 -subj '/CN=cheirizo.local'
  chmod 0600 /etc/wrdp/key.pem
  chmod 0644 /etc/wrdp/cert.pem
fi
