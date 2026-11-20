<?php
// CLI only, fixed code invoked after dropping to this student's Unix identity.
if (PHP_SAPI !== 'cli') { http_response_code(404); exit; }
try {
    $r = json_decode(stream_get_contents(STDIN), true, 16, JSON_THROW_ON_ERROR);
    $pdo = new PDO('mysql:host=mariadb;dbname='.$r['database'].';charset=utf8mb4', $r['username'], $r['password'], [
        PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
        PDO::MYSQL_ATTR_LOCAL_INFILE => false,
        PDO::ATTR_TIMEOUT => 10,
    ]);
    $pdo->exec('SET SESSION max_statement_time=25');
    if ($r['operation'] === 'import') {
        // PDO does not interpret mysql CLI commands (source, system, reconnect).
        $pdo->exec($r['sql']);
        echo json_encode(['ok'=>true]);
    } else {
        $quoteId = fn($name) => '`'.str_replace('`','``',$name).'`';
        $out = "-- Ferrix: tables and data from your database\nSET NAMES utf8mb4;\nSET sql_mode='NO_AUTO_VALUE_ON_ZERO';\nSET FOREIGN_KEY_CHECKS=0;\n";
        $append = function($text) use (&$out) {
            if (strlen($out)+strlen($text)>16*1024*1024) throw new RuntimeException('Export exceeds 16 MiB');
            $out .= $text;
        };
        // InnoDB data is read from one consistent snapshot. Export does not change data.
        $pdo->exec('SET TRANSACTION ISOLATION LEVEL REPEATABLE READ');
        $pdo->beginTransaction();
        $tables=$pdo->query("SHOW FULL TABLES WHERE Table_type = 'BASE TABLE'")->fetchAll(PDO::FETCH_NUM);
        foreach ($tables as [$name]) {
            $table=$quoteId($name);
            $create=$pdo->query('SHOW CREATE TABLE '.$table)->fetch(PDO::FETCH_NUM)[1];
            $append("DROP TABLE IF EXISTS $table;\n$create;\n");
            // Generated columns must be omitted from INSERT.
            $columns=[];
            foreach ($pdo->query('SHOW FULL COLUMNS FROM '.$table) as $col) {
                if (!preg_match('/(?:VIRTUAL|STORED) GENERATED/', $col['Extra'])) $columns[]=$col['Field'];
            }
            if (!$columns) continue;
            $names=implode(',',array_map($quoteId,$columns));
            $query=$pdo->query("SELECT $names FROM $table");
            while ($row=$query->fetch(PDO::FETCH_NUM)) {
                $values=array_map(fn($v)=>$v===null?'NULL':(preg_match('//u',(string)$v)?$pdo->quote($v):"FROM_BASE64('".base64_encode($v)."')"),$row);
                $append("INSERT INTO $table ($names) VALUES (".implode(',',$values).");\n");
            }
        }
        $pdo->commit();
        $append("SET FOREIGN_KEY_CHECKS=1;\n");
        echo json_encode(['hex'=>bin2hex($out)], JSON_THROW_ON_ERROR);
    }
} catch (Throwable $error) {
    // Do not return SQL, credentials, other schemas or server diagnostics.
    exit(1);
}
