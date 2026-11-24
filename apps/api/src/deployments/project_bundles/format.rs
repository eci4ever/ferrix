use super::*;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
pub(super) type Files = Vec<(Uuid, Vec<u8>)>;
pub(super) const LIMIT: usize = 64 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub version: u32,
    pub project_id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub services: Vec<Service>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Service {
    pub id: Uuid,
    pub lineage: Uuid,
    pub name: String,
    pub environment: String,
    pub kind: String,
    pub image: String,
    pub image_id: String,
    pub container_port: i32,
    pub runtime: Value,
    pub environment_values: Vec<String>,
    pub worker: bool,
    pub was_running: bool,
    pub backup_sha256: Option<String>,
    pub source_reference: Value,
    #[serde(default)]
    pub domains: Vec<Value>,
}
pub(super) fn key(value: &str) -> Result<[u8; 32], ApiError> {
    let bytes = hex::decode(value)
        .map_err(|_| invalid("Use the original 64-character bundle recovery key."))?;
    bytes
        .try_into()
        .map_err(|_| invalid("Use the original 64-character bundle recovery key."))
}
pub(super) fn pack(
    key: &[u8; 32],
    manifest: &Manifest,
    files: &[(Uuid, Vec<u8>)],
) -> Result<Vec<u8>, String> {
    let mut builder = tar::Builder::new(Vec::new());
    let metadata = serde_json::to_vec(manifest).map_err(|_| "Could not encode manifest.")?;
    let mut total = metadata.len();
    append(&mut builder, "manifest.json", &metadata)?;
    for (id, data) in files {
        total += data.len() + 1024;
        if total > LIMIT - 1024 * 1024 {
            return Err("Project bundle exceeds the 64 MiB portable export limit.".into());
        }
        append(&mut builder, &format!("{}.dump", id.simple()), data)?;
    }
    let plain = builder
        .into_inner()
        .map_err(|_| "Could not finish bundle.")?;
    let mut nonce = [0; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let cipher = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: b"ferrix-project-bundle:v1",
            },
        )
        .map_err(|_| "Could not encrypt bundle.")?;
    Ok([b"FERRIXPB1".as_slice(), &nonce, &cipher].concat())
}
fn append(builder: &mut tar::Builder<Vec<u8>>, name: &str, data: &[u8]) -> Result<(), String> {
    let mut h = tar::Header::new_gnu();
    h.set_mode(0o600);
    h.set_uid(0);
    h.set_gid(0);
    h.set_size(data.len() as u64);
    h.set_cksum();
    builder
        .append_data(&mut h, name, data)
        .map_err(|_| "Could not append bundle data.".into())
}
pub(super) fn unpack(key: &[u8; 32], bytes: &[u8]) -> Result<(Manifest, Files), String> {
    if bytes.len() > LIMIT + 128 || bytes.len() < 49 || !bytes.starts_with(b"FERRIXPB1") {
        return Err("Invalid or oversized encrypted project bundle.".into());
    }
    let plain = XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(&bytes[9..33]),
            Payload {
                msg: &bytes[33..],
                aad: b"ferrix-project-bundle:v1",
            },
        )
        .map_err(|_| "Bundle key or integrity verification failed.")?;
    let mut archive = tar::Archive::new(Cursor::new(plain));
    let mut manifest = None;
    let mut files = Vec::new();
    let mut names = HashSet::new();
    let mut total = 0;
    for entry in archive.entries().map_err(|_| "Invalid bundle archive.")? {
        let e = entry.map_err(|_| "Invalid bundle entry.")?;
        let name = e
            .path()
            .map_err(|_| "Invalid bundle path.")?
            .to_string_lossy()
            .into_owned();
        if !e.header().entry_type().is_file() || !names.insert(name.clone()) || names.len() > 21 {
            return Err("Unsupported bundle entries.".into());
        }
        total += e.size();
        if total > LIMIT as u64 {
            return Err("Bundle expanded size exceeds its limit.".into());
        }
        let mut data = Vec::new();
        e.take(LIMIT as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|_| "Could not read bundle.")?;
        if name == "manifest.json" {
            manifest = Some(
                serde_json::from_slice::<Manifest>(&data)
                    .map_err(|_| "Invalid bundle manifest.")?,
            );
        } else {
            let id = Uuid::parse_str(
                name.strip_suffix(".dump")
                    .ok_or("Unsupported bundle path.")?,
            )
            .map_err(|_| "Invalid bundle data identifier.")?;
            if name != format!("{}.dump", id.simple()) {
                return Err("Invalid bundle path.".into());
            }
            files.push((id, data));
        }
    }
    let manifest = manifest.ok_or("Missing bundle manifest.")?;
    if manifest.version != 1 || manifest.services.is_empty() || manifest.services.len() > 20 {
        return Err("Unsupported project bundle version or service count.".into());
    }
    let mut ids = HashSet::new();
    for s in &manifest.services {
        if !ids.insert(s.id)
            || s.name.len() > 80
            || s.environment.len() > 80
            || s.environment_values.len() > 200
            || s.environment_values.iter().map(String::len).sum::<usize>() > 65536
            || !(1..=65535).contains(&s.container_port)
            || !matches!(s.kind.as_str(), "application" | "database")
        {
            return Err("Invalid service snapshot.".into());
        }
        let persistent =
            s.kind == "database" || s.runtime.get("storage").is_some_and(|v| !v.is_null());
        if persistent != s.backup_sha256.is_some()
            || s.worker
                != (s.kind == "application" && s.runtime["worker_mode"].as_bool().unwrap_or(false))
        {
            return Err("Inconsistent backup/worker manifest.".into());
        }
        let file = files.iter().find(|(id, _)| *id == s.id);
        match (&s.backup_sha256, file) {
            (Some(expected), Some((_, bytes)))
                if hex::encode(Sha256::digest(bytes)) == *expected => {}
            (None, None) => {}
            _ => return Err("Bundle backup checksum mismatch.".into()),
        }
    }
    if files.iter().any(|(id, _)| !ids.contains(id)) {
        return Err("Unrelated data in project bundle.".into());
    }
    Ok((manifest, files))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_bundle_rejects_wrong_key_and_tampering() {
        let m = Manifest {
            version: 1,
            project_id: Uuid::new_v4(),
            name: "test".into(),
            created_at: Utc::now(),
            services: vec![Service {
                id: Uuid::new_v4(),
                lineage: Uuid::new_v4(),
                name: "web".into(),
                environment: "production".into(),
                kind: "application".into(),
                image: "nginx:alpine".into(),
                image_id: "sha256:test".into(),
                container_port: 80,
                runtime: json!({}),
                environment_values: vec!["APP_SECRET=private".into()],
                worker: false,
                was_running: true,
                backup_sha256: None,
                source_reference: json!({}),
                domains: Vec::new(),
            }],
        };
        let mut bytes = pack(&[42; 32], &m, &[]).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("APP_SECRET"));
        assert!(unpack(&[43; 32], &bytes).is_err());
        assert!(unpack(&[42; 32], &bytes).is_ok());
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(unpack(&[42; 32], &bytes).is_err());
    }
}
