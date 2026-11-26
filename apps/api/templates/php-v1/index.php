<?php
declare(strict_types=1);
header('Content-Type: text/html; charset=utf-8');
header("Content-Security-Policy: default-src 'self'; style-src 'self'; frame-ancestors 'none'; base-uri 'none'");
header('X-Content-Type-Options: nosniff');
try {
    $db = new PDO(
        'mysql:host=' . getenv('DB_HOST') . ';port=' . getenv('DB_PORT') . ';dbname=' . getenv('DB_DATABASE') . ';charset=utf8mb4',
        getenv('DB_USERNAME'),
        getenv('DB_PASSWORD'),
        [PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION, PDO::ATTR_EMULATE_PREPARES => false]
    );
    $db->exec('CREATE TABLE IF NOT EXISTS ferrix_visits (id INT PRIMARY KEY, visits BIGINT NOT NULL)');
    $db->exec('INSERT INTO ferrix_visits (id, visits) VALUES (1, 1) ON DUPLICATE KEY UPDATE visits = visits + 1');
    $visits = (int) $db->query('SELECT visits FROM ferrix_visits WHERE id = 1')->fetchColumn();
    $status = 'Database connected';
} catch (Throwable $error) {
    http_response_code(503);
    error_log('PHP classroom starter: database unavailable. Check managed connection settings.');
    $status = 'Database unavailable. Ask your lecturer to check connection settings.';
    $visits = null;
}
?>
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Ferrix PHP classroom</title><link rel="stylesheet" href="/style.css"></head>
<body><main><p>Ferrix · PHP classroom</p><h1>Your application is running.</h1><p><?= htmlspecialchars($status, ENT_QUOTES, 'UTF-8') ?></p><?php if ($visits !== null): ?><p>Visits: <?= $visits ?></p><?php endif; ?><p>Edit public/index.php and deploy your source. Keep private code outside public/.</p></main></body></html>
