use super::*;
use bollard::query_parameters::{BuildImageOptionsBuilder, BuilderVersion};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::{io::Read, path::Component};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
const ARCHIVE_LIMIT: u64 = 128 * 1024 * 1024;
const EXPANDED_LIMIT: u64 = 512 * 1024 * 1024;
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("Ferrix GitHub deployment")
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("Too many GitHub redirects");
            }
            if attempt.url().scheme() == "https"
                && matches!(
                    attempt.url().host_str(),
                    Some("api.github.com" | "github.com" | "codeload.github.com")
                )
            {
                attempt.follow()
            } else {
                attempt.error("Unexpected GitHub redirect")
            }
        }))
        .build()
        .map_err(|_| "Could not initialize GitHub client.".into())
}
fn request(
    client: &reqwest::Client,
    source: &Source,
    kind: &str,
    reference: &str,
) -> Result<reqwest::RequestBuilder, String> {
    let mut url = url::Url::parse("https://api.github.com/repos/").unwrap();
    let (owner, repo) = source
        .repository
        .split_once('/')
        .ok_or("Invalid saved repository.")?;
    url.path_segments_mut()
        .map_err(|_| "Invalid GitHub URL.")?
        .pop_if_empty()
        .push(owner)
        .push(repo)
        .push(kind)
        .push(reference);
    let request = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10");
    Ok(if let Some(token) = &source.token {
        request.bearer_auth(token)
    } else {
        request
    })
}
pub(super) async fn resolve(source: &Source) -> Result<String, String> {
    let client = client()?;
    let mut response=request(&client,source,"commits",&source.branch)?.timeout(Duration::from_secs(15)).send().await.map_err(|_|"Could not contact GitHub.")?.error_for_status().map_err(|_|"GitHub repository or branch is unavailable. Check token permissions and API rate limits.")?;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read GitHub commit.")?
    {
        if body.len() + chunk.len() > 1024 * 1024 {
            return Err("GitHub commit response is too large.".into());
        }
        body.extend_from_slice(&chunk)
    }
    let value: Value =
        serde_json::from_slice(&body).map_err(|_| "Invalid GitHub commit response.")?;
    value["sha"]
        .as_str()
        .filter(|s| commit(s))
        .map(str::to_owned)
        .ok_or_else(|| "GitHub did not return a valid commit SHA.".into())
}
async fn download(source: &Source, sha: &str) -> Result<tempfile::NamedTempFile, String> {
    let client = client()?;
    let mut response = request(&client, source, "tarball", sha)?
        .send()
        .await
        .map_err(|_| "Could not download GitHub archive.")?
        .error_for_status()
        .map_err(
            |_| "GitHub archive is unavailable. Check the token's Contents read permission.",
        )?;
    if response
        .content_length()
        .is_some_and(|size| size > ARCHIVE_LIMIT)
    {
        return Err("GitHub archive exceeds the 128 MiB limit.".into());
    }
    let temporary =
        tempfile::NamedTempFile::new().map_err(|_| "Could not allocate GitHub archive storage.")?;
    let mut file = tokio::fs::File::from_std(
        temporary
            .reopen()
            .map_err(|_| "Could not open GitHub archive storage.")?,
    );
    let mut size = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "GitHub archive download interrupted.")?
    {
        size += chunk.len() as u64;
        if size > ARCHIVE_LIMIT {
            return Err("GitHub archive exceeds the 128 MiB limit.".into());
        }
        file.write_all(&chunk)
            .await
            .map_err(|_| "Not enough temporary disk space for GitHub archive.")?
    }
    file.flush()
        .await
        .map_err(|_| "Could not flush GitHub archive.")?;
    Ok(temporary)
}
fn context_archive(
    source: &Source,
    archive: tempfile::NamedTempFile,
) -> Result<tempfile::NamedTempFile, String> {
    let root = tempfile::tempdir().map_err(|_| "Could not create source workspace.")?;
    let decoder = flate2::read::GzDecoder::new(
        archive
            .reopen()
            .map_err(|_| "Could not read source archive.")?,
    );
    let mut tar = tar::Archive::new(decoder.take(EXPANDED_LIMIT));
    let mut total = 0u64;
    let mut count = 0u32;
    let mut top = None;
    for entry in tar.entries().map_err(|_| "Invalid GitHub archive.")? {
        let mut entry = entry.map_err(|_| "Invalid GitHub archive entry.")?;
        count += 1;
        if count > 50_000 {
            return Err("Repository has too many files (limit 50,000).".into());
        }
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|_| "Invalid repository path.")?
            .into_owned();
        let components: Vec<_> = path.components().collect();
        if components.is_empty()
            || components
                .iter()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("Repository contains an unsafe archive path.".into());
        }
        let first = components[0].as_os_str().to_owned();
        if let Some(top) = &top {
            if top != &first {
                return Err("Repository archive has multiple roots.".into());
            }
        } else {
            top = Some(first)
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err("Repository archive contains unsupported links or special files.".into());
        }
        total = total
            .checked_add(entry.size())
            .ok_or("Repository size overflow.")?;
        if total > EXPANDED_LIMIT {
            return Err("Repository exceeds the 512 MiB expanded limit.".into());
        }
        if !entry
            .unpack_in(root.path())
            .map_err(|_| "Could not extract repository safely.")?
        {
            return Err("Repository entry is outside its workspace.".into());
        }
    }
    let repository = root.path().join(top.ok_or("Repository archive is empty.")?);
    let context = if source.context == "." {
        repository
    } else {
        repository.join(&source.context)
    };
    php_templates::prepare(&context, &source.dockerfile)?;
    if !context.is_dir() || !context.join(&source.dockerfile).is_file() {
        return Err("Dockerfile or build context was not found in this commit.".into());
    }
    let output = tempfile::NamedTempFile::new().map_err(|_| "Could not allocate build context.")?;
    let mut tar = tar::Builder::new(
        output
            .reopen()
            .map_err(|_| "Could not write build context.")?,
    );
    tar.follow_symlinks(false);
    filtered_context(&mut tar, &context, &source.dockerfile)?;
    tar.finish()
        .map_err(|_| "Could not finalize Docker build context.")?;
    Ok(output)
}
// Docker API clients must filter the context before upload. Match patterns
// against the root-relative path and its parents; later ! rules re-include files.
pub(in crate::deployments) fn filtered_context(
    tar: &mut tar::Builder<std::fs::File>,
    root: &std::path::Path,
    dockerfile: &str,
) -> Result<(), String> {
    let specific = format!("{dockerfile}.dockerignore");
    let ignore_name = if root.join(&specific).is_file() {
        specific
    } else {
        ".dockerignore".into()
    };
    let ignore_path = root.join(&ignore_name);
    let mut rules = Vec::new();
    if ignore_path.is_file() {
        if ignore_path
            .metadata()
            .map_err(|_| "Could not read .dockerignore.")?
            .len()
            > 1024 * 1024
        {
            return Err("The .dockerignore file is too large.".into());
        }
        let text =
            std::fs::read_to_string(&ignore_path).map_err(|_| "Could not read .dockerignore.")?;
        for (index, line) in text.lines().enumerate() {
            if index > 10000 {
                return Err("Too many .dockerignore patterns.".into());
            }
            let line = if index == 0 {
                line.trim_start_matches('\u{feff}')
            } else {
                line
            };
            if line.starts_with('#') {
                continue;
            }
            let mut line = line.trim();
            if line.is_empty() {
                continue;
            }
            let include = line.starts_with('!');
            if include {
                line = &line[1..];
                if line.is_empty() {
                    return Err("Invalid empty .dockerignore exclusion.".into());
                }
            }
            let line = line.trim_matches('/');
            let mut clean = Vec::new();
            for part in line.split('/') {
                match part {
                    "" | "." => {}
                    ".." => {
                        if clean.last().is_some_and(|p| *p != "..") {
                            clean.pop();
                        } else {
                            clean.push(part)
                        }
                    }
                    _ => clean.push(part),
                }
            }
            let pattern = clean.join("/");
            if pattern.is_empty() || pattern == "." {
                continue;
            }
            let matcher = globset::GlobBuilder::new(&pattern)
                .literal_separator(true)
                .backslash_escape(true)
                .build()
                .map_err(|_| "Invalid .dockerignore pattern.")?
                .compile_matcher();
            rules.push((matcher, include));
        }
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut entries = std::fs::read_dir(&directory)
            .map_err(|_| "Could not read source directory.")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Could not read source entry.")?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "Source path is outside build context.")?;
            if php_templates::excluded(dockerfile, relative) {
                continue;
            }
            if relative.components().count() > 128 {
                return Err("Repository paths are too deeply nested.".into());
            }
            let kind = entry
                .file_type()
                .map_err(|_| "Could not read source file type.")?;
            if kind.is_dir() {
                pending.push(path.clone())
            }
            let mut included = true;
            for (pattern, include) in &rules {
                if relative
                    .ancestors()
                    .filter(|p| !p.as_os_str().is_empty())
                    .any(|p| pattern.is_match(p))
                {
                    included = *include
                }
            }
            if relative == std::path::Path::new(dockerfile)
                || relative == std::path::Path::new(&ignore_name)
                || (dockerfile == php_templates::DOCKERFILE
                    && relative.starts_with(".ferrix/php-v1"))
            {
                included = true
            }
            if included {
                tar.append_path_with_name(&path, relative)
                    .map_err(|_| "Could not prepare filtered Docker build context.")?;
            }
        }
    }
    Ok(())
}
async fn log(
    conn: &mut sqlx::PgConnection,
    job: Uuid,
    text: &str,
    token: Option<&str>,
) -> Result<(), String> {
    let text = if let Some(token) = token.filter(|t| !t.is_empty()) {
        text.replace(token, "[redacted]")
    } else {
        text.to_owned()
    };
    let mut end = text.len().min(16384);
    while !text.is_char_boundary(end) {
        end -= 1
    }
    sqlx::query(
        "UPDATE deployments SET logs=right(logs || $2,65536) WHERE id=$1 AND status='running'",
    )
    .bind(job)
    .bind(&text[..end])
    .execute(conn)
    .await
    .map_err(|_| "Could not persist build output.")?;
    Ok(())
}
pub(super) async fn run(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    job: &worker::Job,
) -> Result<bollard::models::ImageInspect, String> {
    let source: Source = open(
        state,
        job.application_id,
        "source",
        job.source_cipher
            .as_deref()
            .ok_or("Missing GitHub source snapshot.")?,
    )
    .map_err(|_| "Could not decrypt GitHub source credentials.")?;
    worker::phase(conn, job.id, "Resolving the latest GitHub branch commit…").await?;
    let sha = resolve(&source).await?;
    sqlx::query("UPDATE deployments SET git_commit=$2 WHERE id=$1 AND status='running'")
        .bind(job.id)
        .bind(&sha)
        .execute(&mut *conn)
        .await
        .map_err(|_| "Could not save Git commit.")?;
    worker::phase(
        conn,
        job.id,
        &format!("Downloading {} at commit {}…", source.repository, sha),
    )
    .await?;
    let archive = download(&source, &sha).await?;
    worker::phase(conn, job.id, "Preparing Dockerfile build context…").await?;
    let source_copy = source.clone();
    let archive = tokio::task::spawn_blocking(move || context_archive(&source_copy, archive))
        .await
        .map_err(|_| "Source preparation was interrupted.")??;
    build_prepared(state, conn, job, &source, &sha, archive).await
}
pub(in crate::deployments) async fn build_prepared(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    job: &worker::Job,
    source: &Source,
    sha: &str,
    archive: tempfile::NamedTempFile,
) -> Result<bollard::models::ImageInspect, String> {
    worker::phase(conn, job.id, "Building Dockerfile with Docker BuildKit…").await?;
    let tag = format!(
        "ferrix-build-{}:{}",
        job.application_id.simple(),
        job.id.simple()
    );
    let labels = HashMap::from([
        ("io.ferrix.managed".to_owned(), "true".to_owned()),
        (
            "io.ferrix.application".into(),
            job.application_id.to_string(),
        ),
        ("io.ferrix.deployment".into(), job.id.to_string()),
        ("io.ferrix.commit".into(), sha.to_owned()),
    ]);
    let options = BuildImageOptionsBuilder::default()
        .dockerfile(&source.dockerfile)
        .t(&tag)
        .rm(true)
        .forcerm(true)
        .version(BuilderVersion::BuilderBuildKit)
        .session(&Uuid::new_v4().to_string())
        .labels(&labels)
        .build();
    let file = tokio::fs::File::from_std(
        archive
            .reopen()
            .map_err(|_| "Could not read Docker build context.")?,
    );
    let mut output = state.docker.build_image(
        options,
        None,
        Some(bollard::body_try_stream(ReaderStream::new(file))),
    );
    while let Some(event) = output.next().await {
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                log(
                    conn,
                    job.id,
                    &format!("Build error: {error}\n"),
                    source.token.as_deref(),
                )
                .await?;
                return Err("Dockerfile build failed. Review deployment output and confirm BuildKit is available on Docker.".into());
            }
        };
        if let Some(text) = event.stream {
            log(conn, job.id, &text, source.token.as_deref()).await?
        }
        if let Some(text) = event.status {
            log(conn, job.id, &format!("{text}\n"), source.token.as_deref()).await?
        }
        if event.error_detail.is_some() {
            return Err("Dockerfile build failed. Review deployment output.".into());
        }
    }
    let image = state
        .docker
        .inspect_image(&tag)
        .await
        .map_err(|_| "Build did not produce an available Docker image.")?;
    sqlx::query("UPDATE deployments SET image=$2 WHERE id=$1 AND status='running'")
        .bind(job.id)
        .bind(tag)
        .execute(&mut *conn)
        .await
        .map_err(|_| "Could not record built image.")?;
    worker::phase(
        conn,
        job.id,
        "Dockerfile build completed. Starting the new version…",
    )
    .await?;
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    use futures_util::FutureExt;
    use std::{io::Write, panic::AssertUnwindSafe};
    fn fixture(files: &[(&str, &str)]) -> tempfile::NamedTempFile {
        let archive = tempfile::NamedTempFile::new().unwrap();
        let encoder = flate2::write::GzEncoder::new(
            archive.reopen().unwrap(),
            flate2::Compression::default(),
        );
        let mut tar = tar::Builder::new(encoder);
        for (path, value) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(value.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(
                &mut header,
                format!("repo-fixture/{path}"),
                value.as_bytes(),
            )
            .unwrap()
        }
        tar.into_inner().unwrap().finish().unwrap().flush().unwrap();
        archive
    }
    fn source() -> Source {
        Source {
            repository: "owner/repository".into(),
            branch: "main".into(),
            dockerfile: "Dockerfile".into(),
            context: ".".into(),
            token: Some("ghp_not_in_build_context".into()),
        }
    }
    #[test]
    fn archive_context_and_credentials_stay_separate() {
        let prepared = context_archive(
            &source(),
            fixture(&[("Dockerfile", "FROM scratch"), ("index.html", "safe")]),
        )
        .unwrap();
        let mut archive = tar::Archive::new(prepared.reopen().unwrap());
        let names: Vec<_> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().into_owned())
            .collect();
        assert!(names.iter().any(|p| p.ends_with("Dockerfile")));
        let bytes = std::fs::read(prepared.path()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("ghp_not_in_build_context"));
        let mut missing = source();
        missing.context = "missing".into();
        assert!(context_archive(&missing, fixture(&[("Dockerfile", "FROM scratch")])).is_err());
    }
    #[tokio::test]
    #[ignore = "optional live GitHub read; set FERRIX_TEST_GITHUB_LIVE=1"]
    async fn public_github_archive_is_pinned_and_prepares_a_real_context() {
        if std::env::var("FERRIX_TEST_GITHUB_LIVE").as_deref() != Ok("1") {
            return;
        }
        let source = Source {
            repository: "eci4ever/ferrix".into(),
            branch: "main".into(),
            token: None,
            ..source()
        };
        let sha = resolve(&source).await.unwrap();
        assert!(commit(&sha));
        let archive = download(&source, &sha).await.unwrap();
        let context = context_archive(&source, archive).unwrap();
        assert!(context.as_file().metadata().unwrap().len() > 0);
    }
    #[tokio::test]
    #[ignore = "requires FERRIX_TEST_DATABASE_URL and Docker BuildKit"]
    async fn dockerfile_build_respects_ignore_and_activates_an_immutable_image() {
        let f = Fixture::new().await;
        let result=AssertUnwindSafe(async{
   let config=json!({"mode":"github","repository":"owner/repository","branch":"main","dockerfile":"Dockerfile","context":".","auto_deploy":false});assert_eq!(f.request("POST",&format!("/api/v1/services/{}/source",f.service),config).await.status,StatusCode::NO_CONTENT);
   let queued=f.request("POST",&format!("/api/v1/services/{}/deploy",f.service),json!({})).await;let id=Uuid::parse_str(queued.body["id"].as_str().unwrap()).unwrap();sqlx::query("UPDATE deployments SET status='running',git_commit=repeat('a',40) WHERE id=$1").bind(id).execute(&f.state.pool).await.unwrap();
   let mut job:worker::Job=sqlx::query_as("SELECT d.id,d.application_id,d.image,d.pinned_image_id,d.source_cipher,d.container_port,d.env_cipher,d.service_kind,d.service_cipher,p.organization_id,p.id AS project_id,e.id AS environment_id FROM deployments d JOIN applications a ON a.id=d.application_id JOIN environments e ON e.id=a.environment_id JOIN projects p ON p.id=e.project_id WHERE d.id=$1").bind(id).fetch_one(&f.state.pool).await.unwrap();
   let archive=context_archive(&source(),fixture(&[("Dockerfile","FROM nginx:alpine\nCOPY . /usr/share/nginx/html/\nRUN test ! -e /usr/share/nginx/html/private.txt\n"),("index.html","Ferrix GitHub build fixture"),("private.txt","must-not-enter-image"),(".dockerignore","private.txt\n")])).unwrap();
   let mut conn=f.state.pool.acquire().await.unwrap();let result=tokio::time::timeout(Duration::from_secs(180),build_prepared(&f.state,&mut conn,&job,&source(),&"a".repeat(40),archive)).await.unwrap();if result.is_err(){let logs:String=sqlx::query_scalar("SELECT logs FROM deployments WHERE id=$1").bind(id).fetch_one(&f.state.pool).await.unwrap();panic!("BuildKit fixture failed: {logs}")};let image=result.unwrap();job.pinned_image_id=image.id;worker::execute(&f.state,&mut conn,&job).await.unwrap();
   let row:(Option<Uuid>,Option<i32>)=sqlx::query_as("SELECT active_deployment_id,host_port FROM applications WHERE id=$1").bind(f.service).fetch_one(&f.state.pool).await.unwrap();assert_eq!(row.0,Some(id));let text=reqwest::get(format!("http://127.0.0.1:{}",row.1.unwrap())).await.unwrap().text().await.unwrap();assert!(text.contains("Ferrix GitHub build fixture"));
   assert!(!f.request("GET",&format!("/api/v1/services/{}/deployments",f.service),Value::Null).await.body.to_string().contains("ghp_not_in_build_context"));
   let rollback=f.request("POST",&format!("/api/v1/services/{}/rollback",f.service),json!({"deployment_id":id})).await;assert_eq!(rollback.status,StatusCode::ACCEPTED);let rollback_id=Uuid::parse_str(rollback.body["id"].as_str().unwrap()).unwrap();
   let metadata:(String,String,bool)=sqlx::query_as("SELECT git_commit,source_repository,source_cipher IS NULL FROM deployments WHERE id=$1").bind(rollback_id).fetch_one(&f.state.pool).await.unwrap();assert_eq!(metadata,("a".repeat(40),"owner/repository".into(),true));
   sqlx::query("UPDATE deployments SET status='running' WHERE id=$1").bind(rollback_id).execute(&f.state.pool).await.unwrap();
   let rollback_job=worker::Job{id:rollback_id,source_cipher:None,..job};worker::execute(&f.state,&mut conn,&rollback_job).await.unwrap();
   assert_eq!(sqlx::query_scalar::<_,Option<Uuid>>("SELECT active_deployment_id FROM applications WHERE id=$1").bind(f.service).fetch_one(&f.state.pool).await.unwrap(),Some(rollback_id));
   drop(conn);
  }).catch_unwind().await;
        let filters = HashMap::from([(
            "label".to_owned(),
            vec![format!("io.ferrix.application={}", f.service)],
        )]);
        if let Ok(containers) = f
            .state
            .docker
            .list_containers(Some(
                bollard::query_parameters::ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
        {
            for container in containers {
                if let Some(id) = container.id {
                    let _ = f
                        .state
                        .docker
                        .remove_container(
                            &id,
                            Some(
                                bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                                    .force(true)
                                    .build(),
                            ),
                        )
                        .await;
                }
            }
        }
        if let Ok(images) = f
            .state
            .docker
            .list_images(Some(
                bollard::query_parameters::ListImagesOptionsBuilder::default()
                    .filters(&filters)
                    .build(),
            ))
            .await
        {
            for image in images {
                let _ = f
                    .state
                    .docker
                    .remove_image(
                        &image.id,
                        Some(
                            bollard::query_parameters::RemoveImageOptionsBuilder::default()
                                .force(true)
                                .build(),
                        ),
                        None,
                    )
                    .await;
            }
        }
        f.finish().await;
        if let Err(error) = result {
            std::panic::resume_unwind(error)
        }
    }
}
