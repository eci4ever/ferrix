FROM php:8.4-fpm
RUN apt-get update && apt-get install -y --no-install-recommends python3 passwd \
    && rm -rf /var/lib/apt/lists/* \
    && docker-php-ext-install pdo_mysql mysqli \
    && printf 'expose_php=Off\ndisplay_errors=Off\nlog_errors=On\nmemory_limit=64M\nmax_execution_time=30\nopcache.enable=0\nffi.enable=false\nenable_dl=Off\n' > /usr/local/etc/php/conf.d/hosting.ini
COPY hosting-manager.py /usr/local/bin/hosting-manager
COPY index.php /usr/local/share/hosting-index.php
RUN chmod 0755 /usr/local/bin/hosting-manager
ENTRYPOINT ["/usr/local/bin/hosting-manager", "init"]
HEALTHCHECK --interval=5s --timeout=3s CMD test -S /run/hosting/_health.sock
