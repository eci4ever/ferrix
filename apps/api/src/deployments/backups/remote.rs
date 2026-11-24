use super::*;
use bollard::{
    exec::{CreateExecOptions, StartExecResults},
    query_parameters::UploadToContainerOptionsBuilder,
};
use s3::{Bucket, Region, creds::Credentials};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Keys {
    pub access_key: String,
    pub secret_key: String,
    pub session_token: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Destination {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub path_style: bool,
    pub credentials: Keys,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EditDestination {
    endpoint: String,
    region: String,
    bucket: String,
    prefix: String,
    path_style: bool,
    credentials: Option<Keys>,
}
pub(super) fn validate(
    state: &AppState,
    edit: EditDestination,
    previous: Option<Destination>,
) -> Result<Destination, ApiError> {
    let endpoint = edit.endpoint.trim().trim_end_matches('/').to_owned();
    if !endpoint.is_empty() {
        let url =
            url::Url::parse(&endpoint).map_err(|_| invalid("Use a valid S3 endpoint URL."))?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        let local_origin = url::Url::parse(&state.auth.origin)
            .ok()
            .is_some_and(|u| matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")));
        if url.host_str().is_none()
            || (!url.username().is_empty())
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
            || !(url.scheme() == "https" || (url.scheme() == "http" && loopback && local_origin))
        {
            return Err(invalid(
                "Use an HTTPS endpoint without credentials, query or path. Local development also permits HTTP loopback.",
            ));
        }
    }
    let region = edit.region.trim().to_owned();
    if region.is_empty()
        || region.len() > 63
        || !region
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(invalid("Use a valid S3 region."));
    }
    let bucket = edit.bucket.trim().to_owned();
    if !(3..=63).contains(&bucket.len())
        || !bucket
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        || !bucket
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !bucket
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(invalid("Use a valid S3 bucket name."));
    }
    let prefix = edit.prefix.trim().trim_matches('/').to_owned();
    if prefix.len() > 256
        || !prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        || prefix.split('/').any(|s| s == ".." || s == ".")
    {
        return Err(invalid(
            "Use a simple S3 prefix without parent-directory segments.",
        ));
    }
    let credentials = edit
        .credentials
        .or_else(|| previous.map(|p| p.credentials))
        .ok_or_else(|| invalid("Enter S3 access and secret keys."))?;
    if credentials.access_key.is_empty()
        || credentials.access_key.len() > 256
        || credentials.secret_key.is_empty()
        || credentials.secret_key.len() > 4096
        || credentials.access_key.chars().any(char::is_control)
        || credentials.secret_key.chars().any(char::is_control)
        || credentials
            .session_token
            .as_ref()
            .is_some_and(|t| t.len() > 8192 || t.chars().any(char::is_control))
    {
        return Err(invalid(
            "Use valid S3 credentials without control characters.",
        ));
    }
    Ok(Destination {
        endpoint,
        region,
        bucket,
        prefix,
        path_style: edit.path_style,
        credentials,
    })
}
pub(super) fn seal(
    state: &AppState,
    service: Uuid,
    destination: &Destination,
) -> Result<Vec<u8>, ApiError> {
    use rand::RngCore;
    let plain = serde_json::to_vec(destination).map_err(|_| internal())?;
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let aad = format!("ferrix:backup-s3:{service}");
    let encrypted = XChaCha20Poly1305::new((&state.auth.key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| internal())?;
    Ok([nonce.to_vec(), encrypted].concat())
}
pub(super) fn open(
    state: &AppState,
    service: Uuid,
    cipher: &[u8],
) -> Result<Destination, ApiError> {
    if cipher.len() < 40 {
        return Err(internal());
    }
    let aad = format!("ferrix:backup-s3:{service}");
    let plain = XChaCha20Poly1305::new((&state.auth.key).into())
        .decrypt(
            XNonce::from_slice(&cipher[..24]),
            Payload {
                msg: &cipher[24..],
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| internal())?;
    serde_json::from_slice(&plain).map_err(|_| internal())
}
impl Destination {
    pub(super) fn redacted(&self) -> Value {
        json!({"endpoint":self.endpoint,"region":self.region,"bucket":self.bucket,"prefix":self.prefix,"path_style":self.path_style,"credentials_saved":true})
    }
    pub(super) fn key(&self, service: Uuid, job: Uuid) -> String {
        format!(
            "{}ferrix/{}/{}.dump",
            if self.prefix.is_empty() {
                String::new()
            } else {
                format!("{}/", self.prefix)
            },
            service.simple(),
            job.simple()
        )
    }
    fn bucket(&self) -> Result<Box<Bucket>, String> {
        let credentials = Credentials::new(
            Some(&self.credentials.access_key),
            Some(&self.credentials.secret_key),
            None,
            self.credentials.session_token.as_deref(),
            None,
        )
        .map_err(|_| "Could not initialize S3 credentials.")?;
        let region = if self.endpoint.is_empty() {
            Region::Custom {
                region: self.region.clone(),
                endpoint: format!("https://s3.{}.amazonaws.com", self.region),
            }
        } else {
            Region::Custom {
                region: self.region.clone(),
                endpoint: self.endpoint.clone(),
            }
        };
        let mut bucket = Bucket::new(&self.bucket, region, credentials)
            .map_err(|_| "Could not initialize S3 destination.")?;
        if self.path_style {
            bucket = bucket.with_path_style()
        }
        bucket.set_request_timeout(Some(Duration::from_secs(120)));
        Ok(bucket)
    }
    pub(super) async fn test(&self, service: Uuid) -> Result<(), String> {
        let bucket = self.bucket()?;
        let key = format!("{}.test", self.key(service, Uuid::new_v4()));
        bucket
            .put_object(&key, b"Ferrix S3 connectivity test")
            .await
            .map_err(|_| "S3 write failed.")?;
        let read = bucket
            .get_object(&key)
            .await
            .map(|r| r.as_slice() == b"Ferrix S3 connectivity test")
            .unwrap_or(false);
        let delete = bucket.delete_object(&key).await.is_ok();
        if read && delete {
            Ok(())
        } else {
            Err("S3 read/delete failed.".into())
        }
    }
}
pub(super) fn transfer_name(job: Uuid) -> String {
    format!("ferrix-backup-transfer-{}", job.simple())
}
async fn transfer(
    state: &AppState,
    job: &Job,
    source: Uuid,
    write: bool,
) -> Result<String, String> {
    let volume = storage(state, source, write).await?;
    let name = transfer_name(job.id);
    let config = ContainerCreateBody {
        image: Some(if job.engine == "application" {
            applications::helper_image(state).await?
        } else {
            job.image_id.clone()
        }),
        user: Some("0:0".into()),
        entrypoint: Some(vec!["sh".into(), "-c".into()]),
        cmd: Some(vec!["exec sleep 1200".into()]),
        labels: Some(HashMap::from([
            ("io.ferrix.managed".into(), "true".into()),
            (
                "io.ferrix.application".into(),
                job.application_id.to_string(),
            ),
            ("io.ferrix.backup-job".into(), job.id.to_string()),
        ])),
        host_config: Some(HostConfig {
            network_mode: Some("none".into()),
            readonly_rootfs: Some(true),
            memory: Some(64 * 1024 * 1024),
            pids_limit: Some(16),
            cap_drop: Some(vec!["ALL".into()]),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            mounts: Some(vec![Mount {
                typ: Some(MountTypeEnum::VOLUME),
                source: Some(volume),
                target: Some("/backups".into()),
                read_only: Some(!write),
                ..Default::default()
            }]),
            log_config: Some(HostConfigLogConfig {
                typ: Some("none".into()),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    state
        .docker
        .create_container(
            Some(CreateContainerOptionsBuilder::default().name(&name).build()),
            config,
        )
        .await
        .map_err(|_| "Could not create backup transfer helper.")?;
    state
        .docker
        .start_container(&name, None)
        .await
        .map_err(|_| "Could not start backup transfer helper.")?;
    Ok(name)
}
async fn exec(
    state: &AppState,
    container: &str,
    cmd: Vec<String>,
) -> Result<
    (
        String,
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<
                        Item = Result<bollard::container::LogOutput, bollard::errors::Error>,
                    > + Send,
            >,
        >,
    ),
    String,
> {
    let exec = state
        .docker
        .create_exec(
            container,
            CreateExecOptions {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                cmd: Some(cmd),
                ..Default::default()
            },
        )
        .await
        .map_err(|_| "Could not start backup file transfer.")?;
    let StartExecResults::Attached { output, .. } =
        state
            .docker
            .start_exec(&exec.id, None)
            .await
            .map_err(|_| "Could not attach backup file transfer.")?
    else {
        return Err("Could not attach backup file transfer.".into());
    };
    Ok((exec.id, output))
}
pub(super) async fn upload(
    state: &AppState,
    job: &Job,
    size: i64,
    checksum: &str,
) -> Result<(), String> {
    let cipher = job
        .s3_cipher
        .as_ref()
        .ok_or("Missing S3 destination snapshot.")?;
    let key = job.s3_key.as_ref().ok_or("Missing S3 object key.")?;
    let destination =
        open(state, job.application_id, cipher).map_err(|_| "Could not decrypt S3 destination.")?;
    let helper = transfer(state, job, job.application_id, false).await?;
    let temporary =
        NamedTempFile::new().map_err(|_| "Could not allocate temporary backup file.")?;
    let mut file = tokio::fs::File::from_std(
        temporary
            .reopen()
            .map_err(|_| "Could not open temporary backup file.")?,
    );
    let (exec, mut output) = exec(
        state,
        &helper,
        vec!["cat".into(), format!("/backups/{}.dump", job.id.simple())],
    )
    .await?;
    let mut bytes = 0i64;
    let mut hash = Sha256::new();
    while let Some(chunk) = output.next().await {
        let chunk = chunk.map_err(|_| "Could not read local backup.")?;
        if !matches!(chunk, bollard::container::LogOutput::StdOut { .. }) {
            return Err("Could not read local backup file.".into());
        }
        let chunk = chunk.into_bytes();
        bytes = bytes
            .checked_add(chunk.len() as i64)
            .ok_or("Backup size overflow.")?;
        if bytes > size {
            return Err("Local backup size differs from its manifest.".into());
        }
        hash.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|_| "Temporary backup disk is full or unavailable.")?;
    }
    if bytes != size
        || hex::encode(hash.finalize()) != checksum
        || state
            .docker
            .inspect_exec(&exec)
            .await
            .map_err(|_| "Could not verify backup transfer.")?
            .exit_code
            != Some(0)
    {
        return Err("Local backup integrity verification failed.".into());
    }
    file.flush()
        .await
        .map_err(|_| "Could not flush temporary backup file.")?;
    drop(file);
    let mut file = tokio::fs::File::from_std(
        temporary
            .reopen()
            .map_err(|_| "Could not reopen temporary backup file.")?,
    );
    let response = destination
        .bucket()?
        .put_object_stream_builder(key)
        .with_max_concurrent_chunks(1)
        .execute_stream(&mut file)
        .await
        .map_err(|_| "S3 upload failed. Check connectivity, bucket permissions and credentials.")?;
    if !(200..300).contains(&response.status_code()) || response.uploaded_bytes() as i64 != size {
        return Err("S3 upload size verification failed.".into());
    }
    Ok(())
}
pub(super) async fn download(
    state: &AppState,
    conn: &mut PgConnection,
    job: &Job,
    source: Uuid,
    backup: Uuid,
) -> Result<(), String> {
    let(size,checksum):(i64,String)=sqlx::query_as("SELECT size_bytes,sha256 FROM database_jobs WHERE id=$1 AND application_id=$2 AND status='succeeded' AND pruned_at IS NULL AND NOT pruning").bind(backup).bind(source).fetch_optional(&mut *conn).await.map_err(|_|"Could not read remote backup manifest.")?.ok_or("Remote backup is unavailable.")?;
    let destination = open(
        state,
        source,
        job.s3_cipher
            .as_deref()
            .ok_or("Missing S3 destination snapshot.")?,
    )
    .map_err(|_| "Could not decrypt original S3 destination.")?;
    let key = job.s3_key.as_ref().ok_or("Missing S3 object key.")?;
    let mut response = destination
        .bucket()?
        .get_object_stream(key)
        .await
        .map_err(|_| "S3 backup download failed. Check access to the original destination.")?;
    let temporary =
        NamedTempFile::new().map_err(|_| "Could not allocate temporary restore file.")?;
    let mut file = tokio::fs::File::from_std(
        temporary
            .reopen()
            .map_err(|_| "Could not open temporary restore file.")?,
    );
    let mut bytes = 0i64;
    let mut hash = Sha256::new();
    while let Some(chunk) = response.bytes.next().await {
        let chunk = chunk.map_err(|_| "S3 download interrupted.")?;
        bytes = bytes
            .checked_add(chunk.len() as i64)
            .ok_or("Backup size overflow.")?;
        if bytes > size {
            return Err("S3 backup is larger than its manifest.".into());
        }
        hash.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|_| "Temporary restore disk is full or unavailable.")?;
    }
    if bytes != size || hex::encode(hash.finalize()) != checksum {
        return Err("S3 backup failed integrity verification (size or SHA-256).".into());
    }
    file.flush()
        .await
        .map_err(|_| "Could not flush restore file.")?;
    drop(file);
    let archive = tokio::task::spawn_blocking(move || {
        let archive = NamedTempFile::new()?;
        let mut tar = tar::Builder::new(archive.reopen()?);
        let mut header = tar::Header::new_gnu();
        header.set_size(size as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("{}.dump", backup.simple()),
            temporary.reopen()?,
        )?;
        tar.finish()?;
        Ok::<_, std::io::Error>(archive)
    })
    .await
    .map_err(|_| "Could not prepare restore archive.")?
    .map_err(|_| "Could not write restore archive.")?;
    let helper = transfer(state, job, source, true).await?;
    let file = tokio::fs::File::from_std(
        archive
            .reopen()
            .map_err(|_| "Could not read restore archive.")?,
    );
    state
        .docker
        .upload_to_container(
            &helper,
            Some(
                UploadToContainerOptionsBuilder::default()
                    .path("/backups")
                    .no_overwrite_dir_non_dir("true")
                    .build(),
            ),
            bollard::body_try_stream(ReaderStream::new(file)),
        )
        .await
        .map_err(|_| "Could not stage remote backup on Docker storage.")?;
    Ok(())
}
pub(super) async fn remove(state: &AppState, job: &Job) -> Result<(), String> {
    if let (Some(cipher), Some(key)) = (&job.s3_cipher, &job.s3_key) {
        open(state, job.application_id, cipher)
            .map_err(|_| "Could not decrypt retained S3 destination.")?
            .bucket()?
            .delete_object(key)
            .await
            .map_err(|_| "Could not remove expired S3 backup.")?;
    }
    remove_local(state, job).await
}
pub(super) async fn remove_local(state: &AppState, job: &Job) -> Result<(), String> {
    // An absent local volume is fine for an S3 backup; never create a volume merely to delete a file.
    if let Err(error) = state
        .docker
        .inspect_volume(&volume_name(job.application_id))
        .await
    {
        if matches!(
            error,
            bollard::errors::Error::DockerResponseServerError {
                status_code: 404,
                ..
            }
        ) {
            return Ok(());
        }
        return Err("Could not inspect local backup retention storage.".into());
    }
    let helper = transfer(state, job, job.application_id, true).await?;
    let (exec, mut output) = exec(
        state,
        &helper,
        vec![
            "rm".into(),
            "-f".into(),
            format!("/backups/{}.dump", job.id.simple()),
            format!("/backups/{}.dump.part", job.id.simple()),
        ],
    )
    .await?;
    while let Some(chunk) = output.next().await {
        chunk.map_err(|_| "Could not remove expired local backup.")?;
    }
    if state
        .docker
        .inspect_exec(&exec)
        .await
        .map_err(|_| "Could not confirm expired backup removal.")?
        .exit_code
        != Some(0)
    {
        return Err("Could not remove expired local backup.".into());
    }
    Ok(())
}

pub(super) async fn bundle_read(state: &AppState, job: &Job) -> Result<Vec<u8>, String> {
    let (size, checksum): (i64, String) = sqlx::query_as(
        "SELECT size_bytes,sha256 FROM database_jobs WHERE id=$1 AND status='succeeded'",
    )
    .bind(job.id)
    .fetch_one(&state.pool)
    .await
    .map_err(|_| "Backup is not complete.")?;
    if !(0..=64 * 1024 * 1024).contains(&size) {
        return Err("Project export supports at most 64 MiB of backup data.".into());
    }
    let result = async {
        let helper = transfer(state, job, job.application_id, false).await?;
        let (exec, mut stream) = exec(
            state,
            &helper,
            vec!["cat".into(), format!("/backups/{}.dump", job.id.simple())],
        )
        .await?;
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "Could not read bundle backup.")?;
            if !matches!(chunk, bollard::container::LogOutput::StdOut { .. }) {
                return Err("Bundle backup transfer failed.".into());
            }
            let chunk = chunk.into_bytes();
            if bytes.len() + chunk.len() > size as usize {
                return Err("Backup size mismatch.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() != size as usize
            || hex::encode(Sha256::digest(&bytes)) != checksum
            || state
                .docker
                .inspect_exec(&exec)
                .await
                .map_err(|_| "Could not inspect bundle transfer.")?
                .exit_code
                != Some(0)
        {
            return Err("Backup checksum verification failed.".into());
        }
        Ok(bytes)
    }
    .await;
    cleanup_helper(state, job).await?;
    result
}
pub(super) async fn bundle_write(
    state: &AppState,
    job: &Job,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let result = async {
        let helper = transfer(state, job, job.application_id, true).await?;
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                format!("{}.dump", job.id.simple()),
                bytes.as_slice(),
            )
            .map_err(|_| "Could not prepare imported backup.")?;
        let body = builder
            .into_inner()
            .map_err(|_| "Could not finish imported backup.")?;
        state
            .docker
            .upload_to_container(
                &helper,
                Some(
                    bollard::query_parameters::UploadToContainerOptionsBuilder::default()
                        .path("/backups")
                        .build(),
                ),
                bollard::body_full(body.into()),
            )
            .await
            .map_err(|_| "Could not write imported backup.")?;
        Ok(())
    }
    .await;
    cleanup_helper(state, job).await?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn endpoint_and_credentials_are_validated_without_exposing_secrets() {
        let state = AppState {
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://ferrix@127.0.0.1/unused")
                .unwrap(),
            docker: bollard::Docker::connect_with_local_defaults().unwrap(),
            gateway: Default::default(),
            auth: std::sync::Arc::new(crate::auth::AuthConfig {
                origin: "http://localhost:5173".into(),
                cookie_name: "unused".into(),
                secure: false,
                key: [0; 32],
            }),
        };
        let input = |endpoint: &str, prefix: &str| {
            serde_json::from_value::<EditDestination>(json!({"endpoint":endpoint,"region":"us-east-1","bucket":"ferrix-test","prefix":prefix,"path_style":true,"credentials":{"access_key":"test-access","secret_key":"test-secret","session_token":null}})).unwrap()
        };
        for endpoint in [
            "http://169.254.169.254",
            "https://name:password@example.com",
            "https://example.com/path",
            "https://example.com?key=value",
            "https://example.com#fragment",
        ] {
            assert!(validate(&state, input(endpoint, "backups"), None).is_err())
        }
        assert!(validate(&state, input("https://example.com", "../other"), None).is_err());
        let destination =
            validate(&state, input("http://127.0.0.1:9000", "backups"), None).unwrap();
        assert!(!destination.redacted().to_string().contains("test-secret"));
        let mut keep = input("https://example.com", "new-prefix");
        keep.credentials = None;
        assert!(validate(&state, input("", "backups"), None).is_ok());
        assert_eq!(
            validate(&state, keep, Some(destination))
                .unwrap()
                .credentials
                .secret_key,
            "test-secret"
        );
    }
}
