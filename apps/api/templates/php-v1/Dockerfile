FROM php:8.5-apache-bookworm
RUN docker-php-ext-install -j2 mysqli pdo_mysql && a2enmod rewrite
COPY --from=composer:2 /usr/bin/composer /usr/local/bin/composer
WORKDIR /var/www/app
COPY . /var/www/app
RUN test -f public/index.php || (echo 'PHP template requires public/index.php in the selected context.' >&2; exit 1)
RUN if [ -f composer.json ]; then test -f composer.lock || (echo 'Commit composer.lock before building dependencies.' >&2; exit 1); COMPOSER_ALLOW_SUPERUSER=1 composer install --no-dev --no-interaction --prefer-dist --no-progress --no-scripts; fi
COPY .ferrix/php-v1/apache.conf /etc/apache2/sites-available/000-default.conf
COPY .ferrix/php-v1/php.ini /usr/local/etc/php/conf.d/ferrix.ini
COPY .ferrix/php-v1/start.sh /usr/local/bin/ferrix-php-start
RUN chmod 755 /usr/local/bin/ferrix-php-start
EXPOSE 80
CMD ["ferrix-php-start"]
