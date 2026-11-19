CREATE SEQUENCE student_hosting_unix_uid START 10000 MAXVALUE 60000;
ALTER TABLE student_hostings ADD COLUMN unix_uid INTEGER NOT NULL DEFAULT nextval('student_hosting_unix_uid') UNIQUE CHECK(unix_uid BETWEEN 10000 AND 60000);
