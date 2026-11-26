#!/bin/sh
set -eu
install -d -o www-data -g www-data -m 0700 /opt/data/uploads /opt/data/sessions
exec apache2-foreground
