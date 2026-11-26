//! Versioned source scaffolding and orchestration; deployment remains the normal worker.
use super::*;
use axum::{http::header, response::IntoResponse};
use std::path::Path as FsPath;

pub(super) const DOCKERFILE: &str = ".ferrix/php-v1/Dockerfile";
const IMAGE: &str = "php:8.5-apache-bookworm";
fn generated() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            DOCKERFILE,
            include_str!("../../templates/php-v1/Dockerfile"),
        ),
        (
            ".ferrix/php-v1/apache.conf",
            include_str!("../../templates/php-v1/apache.conf"),
        ),
        (
            ".ferrix/php-v1/php.ini",
            include_str!("../../templates/php-v1/php.ini"),
        ),
        (
            ".ferrix/php-v1/start.sh",
            include_str!("../../templates/php-v1/start.sh"),
        ),
    ]
}
fn files() -> Vec<(&'static str, &'static str)> {
    let mut files = generated();
    files.extend([
        (
            "public/index.php",
            include_str!("../../templates/php-v1/index.php"),
        ),
        (
            "public/style.css",
            include_str!("../../templates/php-v1/style.css"),
        ),
    ]);
    files
}
pub(super) fn prepare(root: &FsPath, dockerfile: &str) -> Result<(), String> {
    if dockerfile != DOCKERFILE {
        return Ok(());
    }
    if !root.join("public/index.php").is_file() {
        return Err("PHP template requires public/index.php in the selected build context. Root document directories are not served.".into());
    }
    // Reserve a separate path: never replace a user's Dockerfile or Apache configuration.
    for (name, content) in generated() {
        let path = root.join(name);
        if path.exists() {
            if std::fs::read(&path).map_err(|_| "Could not inspect reserved template file.")?
                != content.as_bytes()
            {
                return Err("A reserved .ferrix/php-v1 file differs from the built-in version. Keep your Dockerfile and select its path, or remove the reserved files before choosing the template.".into());
            }
        } else {
            std::fs::create_dir_all(path.parent().ok_or("Invalid template path.")?)
                .map_err(|_| "Could not prepare template directory.")?;
            std::fs::write(path, content).map_err(|_| "Could not prepare PHP template.")?;
        }
    }
    Ok(())
}
pub(super) fn excluded(dockerfile: &str, path: &FsPath) -> bool {
    dockerfile == DOCKERFILE
        && path.components().any(|p| {
            let name = p.as_os_str().to_string_lossy();
            name == ".git"
                || name == ".svn"
                || name == ".hg"
                || name == ".env"
                || name.starts_with(".env.")
        })
}
fn archive() -> Result<Vec<u8>, ApiError> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, content) in files() {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, content.as_bytes())
            .map_err(|_| internal())?;
    }
    builder.into_inner().map_err(|_| internal())
}
pub(super) fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/templates/php-v1", get(catalog))
        .route("/templates/php-v1/download", get(download))
        .route("/services/{id}/php-template", get(detail))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::sessions::identify,
        ))
        .merge(
            Router::new()
                .route("/environments/{id}/php-applications", post(create))
                .route("/services/{id}/php-template/provision", post(provision))
                .route_layer(axum::middleware::from_fn_with_state(
                    state,
                    crate::auth::sessions::protect,
                )),
        )
}
async fn catalog(id: Identity) -> Result<Json<Value>, ApiError> {
    id.manager()?;
    Ok(Json(
        json!({"version":"php-v1","runtime":IMAGE,"document_root":"public/","dockerfile":DOCKERFILE,"extensions":["mysqli","pdo_mysql"],"files":files().into_iter().map(|(path,content)|json!({"path":path,"content":content})).collect::<Vec<_>>(),"storage":"/opt/data","environment_keys":["DB_HOST","DB_PORT","DB_DATABASE","DB_USERNAME","DB_PASSWORD"]}),
    ))
}
async fn download(id: Identity) -> Result<impl IntoResponse, ApiError> {
    id.manager()?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-tar"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"ferrix-php-v1.tar\"",
            ),
        ],
        archive()?,
    ))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    request_id: Uuid,
    name: String,
    student_id: Uuid,
    database_service_id: Uuid,
    memory_mib: i64,
    cpus: f64,
}
#[derive(sqlx::FromRow, Serialize)]
struct Record {
    application_id: Uuid,
    student_id: Option<Uuid>,
    database_service_id: Uuid,
    version: String,
    status: String,
    message: String,
    #[sqlx(default)]
    public_urls: Vec<String>,
    #[sqlx(default)]
    lan_urls: Vec<String>,
    #[sqlx(default)]
    diagnostic_url: Option<String>,
}
async fn detail(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
) -> Result<Json<Option<Record>>, ApiError> {
    let application = app_access(&state, &id, app).await?;
    let mut record: Option<Record> = sqlx::query_as(
        "SELECT * FROM php_classroom_applications WHERE application_id=$1 AND organization_id=$2",
    )
    .bind(app)
    .bind(id.organization_id)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(record) = &mut record {
        record.diagnostic_url = application
            .host_port
            .map(|port| format!("http://127.0.0.1:{port}/"));
        if let Some(url) = application.with_gateway(&state).automatic_url {
            if state.gateway.domain_target().is_some_and(|target| target.3) {
                record.diagnostic_url = Some(url);
            } else if state.gateway.private_mode() {
                record.lan_urls.push(url);
            } else {
                record.public_urls.push(url);
            }
        }
        let routes: Vec<(String,String)> = sqlx::query_as("SELECT hostname,route_kind FROM application_domains WHERE application_id=$1 AND verified_at IS NOT NULL AND dns_status='ready' ORDER BY hostname").bind(app).fetch_all(&state.pool).await?;
        for (hostname, kind) in routes {
            let target = state.gateway.domain_target();
            let https = kind == "tunnel" || target.is_some_and(|t| t.2);
            let port = if kind == "tunnel" {
                443
            } else {
                target.map(|t| t.1).unwrap_or(if https { 443 } else { 80 })
            };
            let scheme = if https { "https" } else { "http" };
            let suffix = if (https && port == 443) || (!https && port == 80) {
                String::new()
            } else {
                format!(":{port}")
            };
            let url = format!("{scheme}://{hostname}{suffix}/");
            if kind == "direct" && target.is_some_and(|target| target.3) {
                record.diagnostic_url = Some(url);
                continue;
            }
            let urls = if kind == "lan" {
                &mut record.lan_urls
            } else {
                &mut record.public_urls
            };
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    Ok(Json(record))
}
async fn create(
    State(state): State<AppState>,
    id: Identity,
    Path(environment): Path<Uuid>,
    Json(input): Json<Create>,
) -> Result<Json<Record>, ApiError> {
    id.manager()?;
    environment_access(&state, &id, environment).await?;
    let server = app_access(&state, &id, input.database_service_id).await?;
    if server.kind != "database"
        || server.database_engine.as_deref() != Some("mariadb")
        || server.environment_id != environment
    {
        return Err(invalid(
            "Choose an existing MariaDB service in this environment.",
        ));
    }
    let runtime = application_runtime::RuntimeConfig {
        memory_mib: input.memory_mib,
        cpus: input.cpus,
        storage: Some(application_runtime::Storage {
            target: "/opt/data".into(),
            read_only: false,
        }),
        ..Default::default()
    };
    runtime.validate()?;
    let parameters = serde_json::to_value(&input).map_err(|_| internal())?;
    let app = input.request_id;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("php-template:{}:{app}", id.organization_id))
        .execute(&mut *tx)
        .await?;
    let restricted: bool = sqlx::query_scalar(
        "SELECT restrict_member_resources FROM auth_organizations WHERE id=$1 FOR SHARE",
    )
    .bind(id.organization_id)
    .fetch_one(&mut *tx)
    .await?;
    if !restricted {
        return Err(invalid(
            "Enable assignment-only resource access in Team before creating student applications. Organization membership alone must not expose other applications.",
        ));
    }
    let student: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_members m JOIN auth_users u ON u.id=m.user_id WHERE m.organization_id=$1 AND m.user_id=$2 AND m.role='member' AND u.system_role='user' AND NOT auth_user_banned(u.banned,u.ban_expires_at))").bind(id.organization_id).bind(input.student_id).fetch_one(&mut *tx).await?;
    if !student {
        return Err(invalid(
            "Choose an active student member of this organization, without an organization manager role.",
        ));
    }
    let broad: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM project_assignments g JOIN auth_members m ON m.organization_id=g.organization_id AND m.user_id=g.user_id JOIN environments e ON e.project_id=g.project_id WHERE e.id=$1 AND m.role='member')").bind(environment).fetch_one(&mut *tx).await?;
    if broad {
        return Err(invalid(
            "Remove broad student project assignments first. Classroom isolation uses individual service assignments.",
        ));
    }
    let existing: Option<(Value,Uuid)> = sqlx::query_as("SELECT t.parameters,a.environment_id FROM php_classroom_applications t JOIN applications a ON a.id=t.application_id WHERE t.application_id=$1 AND t.organization_id=$2").bind(app).bind(id.organization_id).fetch_optional(&mut *tx).await?;
    if let Some((saved, original_environment)) = existing {
        if saved != parameters || original_environment != environment {
            return Err(invalid(
                "This request ID already belongs to a different template request. Retry the original request or use a new ID.",
            ));
        }
    } else {
        insert_application(
            &state,
            &mut tx,
            app,
            environment,
            Configuration {
                name: input.name,
                image: IMAGE.into(),
                container_port: 80,
                environment: String::new(),
                runtime,
            },
        )
        .await?;
        sqlx::query("INSERT INTO php_classroom_applications(application_id,organization_id,student_id,database_service_id,version,parameters) VALUES($1,$2,$3,$4,'php-v1',$5)").bind(app).bind(id.organization_id).bind(input.student_id).bind(input.database_service_id).bind(parameters).execute(&mut *tx).await.map_err(database_error)?;
        sqlx::query("INSERT INTO service_assignments(organization_id,service_id,user_id,permission) VALUES($1,$2,$3,'developer')").bind(id.organization_id).bind(app).bind(input.student_id).execute(&mut *tx).await?;
        uploads::starter(&mut tx, id.user_id, app, archive()?).await?;
    }
    tx.commit().await?;
    let Json(record) = detail(State(state), id, Path(app)).await?;
    Ok(Json(record.ok_or_else(missing)?))
}
async fn provision(
    State(state): State<AppState>,
    id: Identity,
    Path(app): Path<Uuid>,
) -> Result<Json<Record>, ApiError> {
    id.manager()?;
    let Json(record) = detail(State(state.clone()), id.clone(), Path(app)).await?;
    let record = record.ok_or_else(missing)?;
    if record.status == "ready" {
        // Repeated provisioning must not rewrite environment values or race a deployment.
        return Ok(Json(record));
    }
    let result =
        managed_databases::provision_application(&state, &id, record.database_service_id, app)
            .await;
    let (status, message) = if result.is_ok() {
        (
            "ready",
            "Database provisioned, restricted credentials saved. Review source and deploy the Application.",
        )
    } else {
        (
            "failed",
            "Database provisioning incomplete. Check the shared service, then retry. Existing application and student data are retained.",
        )
    };
    sqlx::query(
        "UPDATE php_classroom_applications SET status=$2,message=$3 WHERE application_id=$1",
    )
    .bind(app)
    .bind(status)
    .bind(message)
    .execute(&state.pool)
    .await?;
    result?;
    let Json(record) = detail(State(state), id, Path(app)).await?;
    Ok(Json(record.ok_or_else(missing)?))
}
pub(super) async fn ensure_ready(tx: &mut sqlx::PgConnection, app: Uuid) -> Result<(), ApiError> {
    let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM php_classroom_applications t WHERE t.application_id=$1 AND (t.status<>'ready' OR NOT EXISTS(SELECT 1 FROM managed_databases d WHERE d.application_id=t.application_id AND d.service_id=t.database_service_id AND d.status='ready')))").bind(app).fetch_one(tx).await?;
    if blocked {
        return Err(invalid(
            "Ask your lecturer to provision the PHP application's shared database before deploying. A failed or revoked database must be recovered first.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scaffold_never_replaces_user_files_and_excludes_secrets() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("public")).unwrap();
        std::fs::write(
            root.path().join("public/index.php"),
            "<?php echo 'student';",
        )
        .unwrap();
        std::fs::write(root.path().join("Dockerfile"), "FROM user/image").unwrap();
        prepare(root.path(), DOCKERFILE).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("Dockerfile")).unwrap(),
            "FROM user/image"
        );
        prepare(root.path(), DOCKERFILE).unwrap();
        std::fs::write(root.path().join(DOCKERFILE), "FROM changed/image").unwrap();
        assert!(prepare(root.path(), DOCKERFILE).is_err());
        assert!(excluded(DOCKERFILE, FsPath::new("public/.env.local")));
        assert!(excluded(DOCKERFILE, FsPath::new(".git/config")));
        assert!(!excluded("Dockerfile", FsPath::new(".env")));
        assert!(!excluded(DOCKERFILE, FsPath::new("public/index.php")));
    }
}
