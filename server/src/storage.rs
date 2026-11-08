//! Where uploaded file bytes live: the local disk (`<data_dir>/files/<id>`, zero-config default) or
//! Amazon S3 (`<prefix><id>`) when `S3_BUCKET` is set. Keys are always server-generated file ids.

use std::{path::PathBuf, sync::Arc};

use axum::body::Body;
use object_store::{
    aws::{AmazonS3, AmazonS3Builder, AmazonS3ConfigKey},
    path::Path as ObjectPath,
    Attribute, Attributes, ObjectStore, ObjectStoreExt, PutOptions, PutPayload,
};

use crate::config::Config;

const MB: i64 = 1024 * 1024;
const GB: i64 = 1024 * MB;
/// (per-team, server-wide) storage caps on local disk.
pub const DISK_LIMITS: (i64, i64) = (200 * MB, 5 * GB);
/// (per-team, server-wide) storage caps on S3.
pub const S3_LIMITS: (i64, i64) = (GB, 100 * GB);

#[derive(Clone)]
pub enum Storage {
    Disk { dir: PathBuf },
    /// `disk` is still read as a fallback for files uploaded before S3 was turned on.
    S3 { store: Arc<AmazonS3>, prefix: String, disk: PathBuf },
}

/// File bytes ready to send: a body plus its length when known up front.
pub struct Object {
    pub body: Body,
    pub size: Option<u64>,
}

impl Storage {
    /// S3 when the config has a bucket, otherwise the disk. Credentials come from the standard
    /// AWS_* environment variables (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, …).
    pub fn from_config(cfg: &Config) -> anyhow::Result<Self> {
        let disk = cfg.data_dir.join("files");
        let Some(s3) = &cfg.s3 else { return Ok(Storage::Disk { dir: disk }) };
        // Like `AmazonS3Builder::from_env`, but an empty `AWS_X=` line (common in env files) counts as unset.
        let mut builder = AmazonS3Builder::new();
        for (k, v) in std::env::vars() {
            if let (true, false, Ok(key)) = (k.starts_with("AWS_"), v.is_empty(), k.to_ascii_lowercase().parse::<AmazonS3ConfigKey>()) {
                builder = builder.with_config(key, v);
            }
        }
        let store = builder.with_bucket_name(&s3.bucket).with_region(&s3.region).build()?;
        Ok(Storage::S3 { store: Arc::new(store), prefix: s3.prefix.clone(), disk })
    }

    pub fn is_s3(&self) -> bool {
        matches!(self, Storage::S3 { .. })
    }

    /// (per-team, server-wide) storage caps in bytes.
    pub fn limits(&self) -> (i64, i64) {
        match self {
            Storage::Disk { .. } => DISK_LIMITS,
            Storage::S3 { .. } => S3_LIMITS,
        }
    }

    pub async fn put(&self, id: &str, content_type: &str, bytes: &[u8]) -> anyhow::Result<()> {
        match self {
            Storage::Disk { dir } => {
                tokio::fs::create_dir_all(dir).await?;
                tokio::fs::write(dir.join(id), bytes).await?;
            }
            Storage::S3 { store, prefix, .. } => {
                let mut attributes = Attributes::new();
                attributes.insert(Attribute::ContentType, content_type.to_string().into());
                let opts = PutOptions { attributes, ..Default::default() };
                let payload = PutPayload::from(bytes.to_vec());
                store.put_opts(&object_key(prefix, id), payload, opts).await?;
            }
        }
        Ok(())
    }

    /// `Ok(None)` when the file exists nowhere.
    pub async fn get(&self, id: &str) -> anyhow::Result<Option<Object>> {
        let disk = match self {
            Storage::Disk { dir } => dir,
            Storage::S3 { store, prefix, disk } => {
                match store.get(&object_key(prefix, id)).await {
                    Ok(r) => {
                        let size = Some(r.meta.size);
                        return Ok(Some(Object { body: Body::from_stream(r.into_stream()), size }));
                    }
                    Err(object_store::Error::NotFound { .. }) => disk,
                    Err(e) => return Err(e.into()),
                }
            }
        };
        match tokio::fs::read(disk.join(id)).await {
            Ok(bytes) => Ok(Some(Object { size: Some(bytes.len() as u64), body: Body::from(bytes) })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

fn object_key(prefix: &str, id: &str) -> ObjectPath {
    ObjectPath::from(format!("{prefix}{id}"))
}

/// Human label for a cap: "200 MB", "1 GB".
pub fn cap_label(bytes: i64) -> String {
    if bytes >= GB && bytes % GB == 0 { format!("{} GB", bytes / GB) } else { format!("{} MB", bytes / MB) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::S3Config;

    fn cfg_with(s3: Option<S3Config>, dir: PathBuf) -> Config {
        let mut cfg = Config::from_env();
        cfg.data_dir = dir;
        cfg.s3 = s3;
        cfg
    }

    #[test]
    fn backend_selection() {
        let disk = Storage::from_config(&cfg_with(None, "data".into())).unwrap();
        assert!(!disk.is_s3());
        assert_eq!(disk.limits(), (200 * MB, 5 * GB));
        let s3 = S3Config { bucket: "b".into(), region: "eu-central-1".into(), prefix: "files/".into() };
        let s3 = Storage::from_config(&cfg_with(Some(s3), "data".into())).unwrap();
        assert!(s3.is_s3());
        assert_eq!(s3.limits(), (GB, 100 * GB));
        assert_eq!(object_key("files/", "abc").as_ref(), "files/abc");
        assert_eq!(cap_label(200 * MB), "200 MB");
        assert_eq!(cap_label(100 * GB), "100 GB");
    }

    async fn body_bytes(o: Object) -> Vec<u8> {
        axum::body::to_bytes(o.body, usize::MAX).await.unwrap().to_vec()
    }

    #[tokio::test]
    async fn disk_round_trip() {
        let dir = std::env::temp_dir().join(format!("sb-storage-{}", crate::util::random_token(8)));
        let st = Storage::from_config(&cfg_with(None, dir.clone())).unwrap();
        st.put("abc", "text/plain", b"hello").await.unwrap();
        let o = st.get("abc").await.unwrap().unwrap();
        assert_eq!(o.size, Some(5));
        assert_eq!(body_bytes(o).await, b"hello");
        assert!(st.get("missing").await.unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `S3_BUCKET=… AWS_ACCESS_KEY_ID=… AWS_SECRET_ACCESS_KEY=… cargo test -- --ignored s3_round_trip`
    #[tokio::test]
    #[ignore]
    async fn s3_round_trip() {
        let _ = dotenvy::dotenv();
        let dir = std::env::temp_dir().join(format!("sb-storage-{}", crate::util::random_token(8)));
        let cfg = cfg_with(Config::from_env().s3, dir.clone());
        assert!(cfg.s3.is_some(), "set S3_BUCKET to run this test");
        let st = Storage::from_config(&cfg).unwrap();
        let id = format!("test-{}", crate::util::random_token(16));
        st.put(&id, "text/plain", b"hello s3").await.unwrap();
        let o = st.get(&id).await.unwrap().expect("object in S3");
        assert_eq!(o.size, Some(8));
        assert_eq!(body_bytes(o).await, b"hello s3");
        // Not in S3 but on disk (pre-S3 upload) → disk fallback.
        std::fs::create_dir_all(dir.join("files")).unwrap();
        std::fs::write(dir.join("files").join("legacy"), b"old").unwrap();
        assert_eq!(body_bytes(st.get("legacy").await.unwrap().unwrap()).await, b"old");
        assert!(st.get("missing-everywhere").await.unwrap().is_none());
        if let Storage::S3 { store, prefix, .. } = &st {
            store.delete(&object_key(prefix, &id)).await.unwrap();
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
