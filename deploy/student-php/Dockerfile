FROM php:8.4-apache
RUN docker-php-ext-install pdo_mysql mysqli \
    && printf 'expose_php=Off\ndisplay_errors=Off\nlog_errors=On\nupload_max_filesize=8M\npost_max_size=10M\nmemory_limit=128M\nmax_execution_time=30\nopcache.revalidate_freq=0\n' > /usr/local/etc/php/conf.d/student.ini \
    && printf '<Directory /var/www/html>\nAllowOverride None\nOptions -Indexes -ExecCGI\n</Directory>\n' > /etc/apache2/conf-enabled/student.conf
HEALTHCHECK --interval=10s --timeout=3s CMD php -r '$s=@fsockopen("127.0.0.1",80); exit($s?0:1);'

COPY index.php /var/www/html/index.php
RUN chown -R www-data:www-data /var/www/html
